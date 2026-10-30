//! 主界面：工具栏 + 会话树侧栏 + 终端标签页 + 状态栏。

use gpui_kit as gpui;

use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, point, px, size, AnyWindowHandle, AppContext, Bounds, Context, CursorStyle, Div, Entity,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Point, Render,
    SharedString, Stateful, StatefulInteractiveElement, Styled, Subscription, TitlebarOptions, Window,
    WindowBounds, WindowControlArea, WindowOptions,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt, DropdownMenu, PopupMenu, PopupMenuItem},
    Icon,
};
use onecrt_core::mcp;
use onecrt_core::sessions::{
    self, AuthMethod, CmdNode, CommandTree, Node, SessionTree, SshSession,
};
use onecrt_core::settings::{self, UiPrefs};
use onecrt_core::terminal;
use onecrt_core::Backend;

use crate::about_dialog::open_about;
use crate::command_form::{open_command_form, CommandFields};
use crate::dialog_window;
use crate::file_panel::FilePanel;
use crate::i18n::{tr, Lang};
use crate::import_flow::{self, ImportKind, Plan as ImportPlan};
use crate::mcp_dialog::open_mcp_dialog;
use crate::password_prompt::open_password_prompt;
use crate::perf_panel::{PerfPanel, PerfPanelEvent};
use crate::quick_connect::open_quick_connect;
use crate::session_form::{open_session_form, open_text_prompt, SessionDraft};
use crate::term_view::{ConnState, ConnectSpec, TabEvent, TerminalView, Zoom};
use crate::theme::{self, Mode, Theme};

struct TabEntry {
    view: Entity<TerminalView>,
    _sub: Subscription,
}

const DEFAULT_MONO: f32 = 14.;

/// 停靠面板的稳定标识；同一面板同一时刻只存在于一个停靠组或一个浮动窗口里。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PanelId {
    Sessions,
    Commands,
    Files,
    Perf,
}

impl PanelId {
    fn key(self) -> &'static str {
        match self {
            PanelId::Sessions => "sessions",
            PanelId::Commands => "commands",
            PanelId::Files => "files",
            PanelId::Perf => "perf",
        }
    }

    fn icon(self) -> IconName {
        match self {
            PanelId::Sessions => IconName::Server,
            PanelId::Commands => IconName::SquareTerminal,
            PanelId::Files => IconName::FolderOpen,
            PanelId::Perf => IconName::ChartLine,
        }
    }

    fn title(self, lang: Lang) -> SharedString {
        match self {
            PanelId::Sessions => tr(lang, "会话管理器", "Session Manager"),
            PanelId::Commands => tr(lang, "命令管理器", "Command Manager"),
            PanelId::Files => tr(lang, "文件面板", "File Panel"),
            PanelId::Perf => tr(lang, "性能面板", "Performance Panel"),
        }
    }

    fn short_title(self, lang: Lang) -> SharedString {
        match self {
            PanelId::Sessions => tr(lang, "会话", "Sessions"),
            PanelId::Commands => tr(lang, "命令", "Commands"),
            PanelId::Files => tr(lang, "文件", "Files"),
            PanelId::Perf => tr(lang, "性能", "Performance"),
        }
    }

    fn default_zone(self) -> DockZone {
        match self {
            PanelId::Sessions | PanelId::Commands => DockZone::Left,
            PanelId::Files | PanelId::Perf => DockZone::Right,
        }
    }
}

/// 与 VS 一致的四条停靠边；每组页签在组内叠加。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DockZone {
    Left,
    Right,
    Bottom,
    Top,
}

impl DockZone {
    const ALL: [DockZone; 4] = [
        DockZone::Left,
        DockZone::Right,
        DockZone::Bottom,
        DockZone::Top,
    ];

    fn index(self) -> usize {
        match self {
            DockZone::Left => 0,
            DockZone::Right => 1,
            DockZone::Bottom => 2,
            DockZone::Top => 3,
        }
    }

    fn key(self) -> &'static str {
        match self {
            DockZone::Left => "left",
            DockZone::Right => "right",
            DockZone::Bottom => "bottom",
            DockZone::Top => "top",
        }
    }
}

#[derive(Clone, Default)]
struct DockGroup {
    panels: Vec<PanelId>,
    active: Option<PanelId>,
    collapsed: bool,
}

impl DockGroup {
    fn contains(&self, panel: PanelId) -> bool {
        self.panels.contains(&panel)
    }

    fn add(&mut self, panel: PanelId) {
        if !self.contains(panel) {
            self.panels.push(panel);
        }
        self.active = Some(panel);
        self.collapsed = false;
    }

    fn remove(&mut self, panel: PanelId) -> bool {
        let before = self.panels.len();
        self.panels.retain(|p| *p != panel);
        let removed = self.panels.len() != before;
        if removed && self.active == Some(panel) {
            self.active = self.panels.last().copied();
        }
        if self.panels.is_empty() {
            self.collapsed = false;
        }
        removed
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DockTarget {
    Zone(DockZone),
    Float,
}

/// 自动隐藏组的浮出状态：悬停探出，点击固定，固定点图钉后重新停靠。
#[derive(Clone, Copy, PartialEq, Eq)]
struct DockPeek {
    zone: DockZone,
    panel: PanelId,
    sticky: bool,
}

/// 页签拖动携带的值；落点在拖拽移动时算好，松手时直接取用。
#[derive(Clone, Copy)]
struct DockDrag {
    panel: PanelId,
}

/// 拖动页签时跟随光标的胶囊，对齐 VS 的拖拽提示。
struct DockGhost {
    panel: PanelId,
    theme: Theme,
    lang: Lang,
}

impl Render for DockGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(26.))
            .px(px(10.))
            .rounded(px(6.))
            .bg(self.theme.bg_selected)
            .border_1()
            .border_color(self.theme.accent)
            .text_size(px(12.))
            .text_color(self.theme.text)
            .child(Icon::new(self.panel.icon()).size(px(14.)))
            .child(self.panel.short_title(self.lang))
    }
}

/// 拖分隔条时的空载浮层：只为了让 gpui 的 drag 生命周期跑起来，本身不画东西。
struct SplitGhost;

impl Render for SplitGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SplitEdge {
    Left,
    Right,
}

struct VDivider;

struct TopDivider;

#[derive(Clone)]
struct CmdClip {
    node: CmdNode,
    cut: bool,
}

#[derive(Clone)]
struct SessClip {
    node: Node,
    cut: bool,
}

pub struct OneCrt {
    backend: Arc<Backend>,
    theme: Theme,
    lang: Lang,
    prefs: UiPrefs,
    mono_size: f32,
    /// 已同步给平台的窗口标题，避免每帧重复 set_window_title
    window_title: String,
    tree: SessionTree,
    selected: Option<String>,
    sess_clip: Option<SessClip>,
    tabs: Vec<TabEntry>,
    active: Option<usize>,
    next_tab: usize,
    error: Option<String>,
    filter: Entity<InputState>,
    _filter_sub: Subscription,
    commands: CommandTree,
    cmd_selected: Option<String>,
    cmd_clip: Option<CmdClip>,
    cmd_filter: Entity<InputState>,
    _cmd_filter_sub: Subscription,
    files: Option<Entity<FilePanel>>,
    perf: Option<Entity<PerfPanel>>,
    _perf_sub: Option<Subscription>,
    groups: [DockGroup; 4],
    bottom_h: f32,
    top_h: f32,
    active_panel: PanelId,
    floating: Vec<(PanelId, AnyWindowHandle)>,
    guide: Option<DockTarget>,
    guide_area: Option<(f32, f32)>,
    left_w: f32,
    right_w: f32,
    mcp: mcp::Status,
    _mcp_poll: gpui::Task<()>,
    /// 自动隐藏时贴边探出的浮层；不改停靠布局，只覆盖在内容上。
    peek: Option<DockPeek>,
    peek_gen: u64,
    rail_hover: bool,
    flyout_hover: bool,
}

impl OneCrt {
    pub fn new(backend: Arc<Backend>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tree = sessions::load_tree().unwrap_or_default();
        let commands = sessions::load_commands().unwrap_or_default();
        let prefs = settings::load();
        let mode = if prefs.theme == "light" {
            Mode::Light
        } else {
            Mode::Dark
        };
        let theme = Theme::new(mode, prefs.font_size);
        let mono_size = prefs.font_size;
        theme::sync_component(theme.mode, cx);
        let lang = if prefs.lang == "en" {
            Lang::En
        } else {
            Lang::Zh
        };
        let mcp_status = {
            let mgr = backend.mcp.clone();
            backend
                .handle()
                .block_on(async move { mcp::status(&mgr).await })
        };
        let filter = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr(
                lang,
                "按文件夹/会话名筛选",
                "Filter folders or sessions",
            ))
        });
        let filter_sub = cx.subscribe(&filter, |_this, _input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let cmd_filter = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr(
                lang,
                "按分组/命令名筛选",
                "Filter folders or commands",
            ))
        });
        let cmd_filter_sub = cx.subscribe(&cmd_filter, |_this, _input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let poll_backend = backend.clone();
        let mcp_poll = cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(5))
                .await;
            let mgr = poll_backend.mcp.clone();
            let status = poll_backend
                .handle()
                .block_on(async move { mcp::status(&mgr).await });
            let alive = this.update(cx, |this, cx| {
                this.mcp = status;
                cx.notify();
            });
            if alive.is_err() {
                break;
            }
        });
        Self {
            backend,
            theme,
            lang,
            prefs,
            mono_size,
            window_title: "OneCRT".to_string(),
            tree,
            selected: None,
            sess_clip: None,
            tabs: Vec::new(),
            active: None,
            next_tab: 0,
            error: None,
            filter,
            _filter_sub: filter_sub,
            commands,
            cmd_selected: None,
            cmd_clip: None,
            cmd_filter,
            _cmd_filter_sub: cmd_filter_sub,
            files: None,
            perf: None,
            _perf_sub: None,
            groups: [
                DockGroup {
                    panels: vec![PanelId::Sessions, PanelId::Commands],
                    active: Some(PanelId::Sessions),
                    collapsed: false,
                },
                DockGroup::default(),
                DockGroup::default(),
                DockGroup::default(),
            ],
            bottom_h: 210.,
            top_h: 190.,
            active_panel: PanelId::Sessions,
            floating: Vec::new(),
            guide: None,
            guide_area: None,
            left_w: 320.,
            right_w: 460.,
            mcp: mcp_status,
            _mcp_poll: mcp_poll,
            peek: None,
            peek_gen: 0,
            rail_hover: false,
            flyout_hover: false,
        }
    }

    /// 文件/性能面板都跟着活动标签页走，切页、开页、关页、重连都要同步。
    fn sync_panels(&mut self, cx: &mut Context<Self>) {
        self.sync_files(cx);
        self.sync_perf(cx);
    }

    fn sync_files(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = self.files.clone() else {
            return;
        };
        let session = self.active.and_then(|i| self.tabs.get(i)).map(|tab| {
            let view = tab.view.read(cx);
            (view.id.clone(), view.spec.clone())
        });
        panel.update(cx, |panel, cx| panel.set_session(session, cx));
    }

    fn ensure_panel(&mut self, panel: PanelId, cx: &mut Context<Self>) {
        match panel {
            PanelId::Files if self.files.is_none() => {
                let entity = cx.new(|cx| {
                    FilePanel::new(self.backend.clone(), self.theme.clone(), self.lang, cx)
                });
                self.files = Some(entity);
                self.sync_files(cx);
            }
            PanelId::Perf if self.perf.is_none() => {
                let interval = self.prefs.perf_interval;
                let entity = cx.new(|cx| {
                    PerfPanel::new(
                        self.backend.clone(),
                        self.theme.clone(),
                        self.lang,
                        interval,
                        cx,
                    )
                });
                let sub =
                    cx.subscribe(&entity, |this, _, event: &PerfPanelEvent, cx| match event {
                        PerfPanelEvent::Close => this.hide_panel(PanelId::Perf, cx),
                        PerfPanelEvent::Interval(sec) => {
                            this.prefs.perf_interval = *sec;
                            let _ = settings::save(&this.prefs);
                        }
                    });
                self.perf = Some(entity);
                self._perf_sub = Some(sub);
                self.sync_perf(cx);
            }
            _ => {}
        }
    }

    fn zone_of(&self, panel: PanelId) -> Option<DockZone> {
        DockZone::ALL
            .into_iter()
            .find(|zone| self.groups[zone.index()].contains(panel))
    }

    fn float_of(&self, panel: PanelId) -> Option<AnyWindowHandle> {
        self.floating
            .iter()
            .find(|(p, _)| *p == panel)
            .map(|(_, handle)| *handle)
    }

    fn panel_visible(&self, panel: PanelId) -> bool {
        self.zone_of(panel).is_some() || self.float_of(panel).is_some()
    }

    fn show_panel(&mut self, panel: PanelId, cx: &mut Context<Self>) {
        self.ensure_panel(panel, cx);
        if let Some(zone) = self.zone_of(panel) {
            let group = &mut self.groups[zone.index()];
            group.active = Some(panel);
            group.collapsed = false;
            if self.peek.map(|p| p.zone == zone).unwrap_or(false) {
                self.peek = None;
                self.peek_gen = self.peek_gen.wrapping_add(1);
            }
            if panel == PanelId::Files {
                self.sync_files(cx);
            } else if panel == PanelId::Perf {
                self.sync_perf(cx);
            }
        } else if let Some(handle) = self.float_of(panel) {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        } else {
            self.groups[panel.default_zone().index()].add(panel);
        }
        self.active_panel = panel;
        cx.notify();
    }

    fn toggle_panel(&mut self, panel: PanelId, cx: &mut Context<Self>) {
        if self.panel_visible(panel) {
            self.hide_panel(panel, cx);
        } else {
            self.show_panel(panel, cx);
        }
    }

    fn activate_panel(&mut self, panel: PanelId, cx: &mut Context<Self>) {
        for group in &mut self.groups {
            if group.contains(panel) {
                group.active = Some(panel);
            }
        }
        // 自动隐藏时换页签只换浮出内容，不把分组重新停靠展开。
        if let Some(peek) = &mut self.peek {
            peek.panel = panel;
        }
        self.active_panel = panel;
        if panel == PanelId::Files {
            self.sync_files(cx);
        } else if panel == PanelId::Perf {
            self.sync_perf(cx);
        }
        cx.notify();
    }

    /// 自动隐藏的窄条悬停：探出浮层；点住页签后固定，直到点别处。
    fn peek_open(&mut self, zone: DockZone, panel: PanelId, sticky: bool, cx: &mut Context<Self>) {
        if !self.groups[zone.index()].contains(panel) {
            return;
        }
        self.groups[zone.index()].active = Some(panel);
        self.active_panel = panel;
        self.peek = Some(DockPeek {
            zone,
            panel,
            sticky,
        });
        self.peek_gen = self.peek_gen.wrapping_add(1);
        if panel == PanelId::Files {
            self.sync_files(cx);
        } else if panel == PanelId::Perf {
            self.sync_perf(cx);
        }
        cx.notify();
    }

    fn peek_close(&mut self, cx: &mut Context<Self>) {
        if self.peek.take().is_some() {
            self.peek_gen = self.peek_gen.wrapping_add(1);
            cx.notify();
        }
    }

    /// 鼠标离开窄条或浮层时给一点宽限，够它横移进浮层再取消。
    fn peek_close_later(&mut self, cx: &mut Context<Self>) {
        if !self.peek.map(|p| !p.sticky).unwrap_or(false) {
            return;
        }
        let gen = self.peek_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(180))
                .await;
            this.update(cx, |this, cx| {
                if this.peek_gen == gen
                    && !this.rail_hover
                    && !this.flyout_hover
                    && this.peek.map(|p| !p.sticky).unwrap_or(false)
                {
                    this.peek = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn rail_hovered(
        &mut self,
        zone: DockZone,
        panel: PanelId,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        self.rail_hover = hovered;
        if hovered {
            self.peek_gen = self.peek_gen.wrapping_add(1);
            let pinned = self
                .peek
                .map(|p| p.sticky && p.zone == zone)
                .unwrap_or(false);
            if !pinned {
                self.peek_open(zone, panel, false, cx);
            }
        } else {
            self.peek_close_later(cx);
        }
    }

    fn set_collapsed(&mut self, zone: DockZone, collapsed: bool, cx: &mut Context<Self>) {
        self.groups[zone.index()].collapsed = collapsed;
        if self.peek.map(|p| p.zone == zone).unwrap_or(false) {
            self.peek = None;
            self.peek_gen = self.peek_gen.wrapping_add(1);
        }
        cx.notify();
    }

    /// 停靠区之外的可见范围：给浮层和点空白收起用的背景层定位。
    fn dock_insets(&self) -> (f32, f32, f32, f32) {
        let span = |zone: DockZone| {
            let group = &self.groups[zone.index()];
            if group.panels.is_empty() {
                0.
            } else if group.collapsed {
                if matches!(zone, DockZone::Bottom | DockZone::Top) {
                    30.
                } else {
                    36.
                }
            } else {
                match zone {
                    DockZone::Left => self.left_w,
                    DockZone::Right => self.right_w,
                    DockZone::Bottom => self.bottom_h,
                    DockZone::Top => self.top_h,
                }
            }
        };
        (
            span(DockZone::Left),
            span(DockZone::Right),
            span(DockZone::Top),
            span(DockZone::Bottom),
        )
    }

    fn remove_panel_state(&mut self, panel: PanelId) {
        for group in &mut self.groups {
            group.remove(panel);
        }
    }

    fn take_float(&mut self, panel: PanelId) -> Option<AnyWindowHandle> {
        let handle = self.float_of(panel);
        self.floating.retain(|(p, _)| *p != panel);
        handle
    }

    /// 关闭浮动窗口要等事件派发结束，避免在窗口自己的回调里重入 update。
    fn close_float_window(handle: AnyWindowHandle, cx: &mut Context<Self>) {
        cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(30))
                .await;
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        })
        .detach();
    }

    fn shutdown_panel(&mut self, panel: PanelId, cx: &mut Context<Self>) {
        match panel {
            PanelId::Files => {
                if let Some(entity) = self.files.take() {
                    entity.update(cx, |panel, cx| panel.shutdown(cx));
                }
            }
            PanelId::Perf => {
                if let Some(entity) = self.perf.take() {
                    entity.update(cx, |panel, cx| panel.shutdown(cx));
                }
                self._perf_sub = None;
            }
            PanelId::Sessions | PanelId::Commands => {}
        }
    }

    fn hide_panel(&mut self, panel: PanelId, cx: &mut Context<Self>) {
        let floating = self.take_float(panel);
        self.remove_panel_state(panel);
        if self.peek.map(|p| p.panel == panel).unwrap_or(false) {
            self.peek = None;
            self.peek_gen = self.peek_gen.wrapping_add(1);
        }
        if let Some(handle) = floating {
            Self::close_float_window(handle, cx);
        }
        self.shutdown_panel(panel, cx);
        if self.active_panel == panel {
            self.active_panel = PanelId::Sessions;
        }
        cx.notify();
    }

    /// 浮动窗口由平台侧关掉时的收尾，窗口本身已经在关闭流程里。
    fn forget_panel(&mut self, panel: PanelId, cx: &mut Context<Self>) {
        self.take_float(panel);
        self.remove_panel_state(panel);
        self.shutdown_panel(panel, cx);
        if self.active_panel == panel {
            self.active_panel = PanelId::Sessions;
        }
        cx.notify();
    }

    fn dock_panel(&mut self, panel: PanelId, zone: DockZone, cx: &mut Context<Self>) {
        self.ensure_panel(panel, cx);
        let floating = self.take_float(panel);
        self.remove_panel_state(panel);
        if self.peek.map(|p| p.panel == panel).unwrap_or(false) {
            self.peek = None;
            self.peek_gen = self.peek_gen.wrapping_add(1);
        }
        if let Some(handle) = floating {
            Self::close_float_window(handle, cx);
        }
        self.groups[zone.index()].add(panel);
        self.active_panel = panel;
        cx.notify();
    }

    fn float_panel(&mut self, panel: PanelId, window: &mut Window, cx: &mut Context<Self>) {
        if self.float_of(panel).is_some() {
            return;
        }
        self.ensure_panel(panel, cx);
        self.remove_panel_state(panel);
        if self.peek.map(|p| p.panel == panel).unwrap_or(false) {
            self.peek = None;
            self.peek_gen = self.peek_gen.wrapping_add(1);
        }
        self.active_panel = panel;
        let app = cx.entity();
        let bounds = window.bounds();
        let title = panel.title(self.lang);
        // 拖放回调里主实体还在更新中，新窗口首帧会读它，等这轮更新结束再开窗。
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(30))
                .await;
            let opened = cx.update(|cx| {
                let dialog_size = size(px(520.), px(600.));
                let origin = point(bounds.origin.x + px(80.), bounds.origin.y + px(60.));
                let options = WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(origin, dialog_size))),
                    titlebar: Some(TitlebarOptions {
                        title: Some(title),
                        appears_transparent: true,
                        traffic_light_position: None,
                    }),
                    window_min_size: Some(size(px(260.), px(220.))),
                    ..Default::default()
                };
                cx.open_window(options, move |window, cx| {
                    // 浮动面板同样自绘标题栏，拖动时也要跳过移动循环里的整帧重绘
                    crate::move_paint_skip::install(window);
                    let view = cx.new(|cx| PanelWindow::new(app.clone(), panel, cx));
                    view.update(cx, |view, cx| view.install_handlers(window, cx));
                    view
                })
            });
            match opened {
                Ok(handle) => {
                    let _ = this.update(cx, |this, cx| {
                        this.floating.push((panel, handle.into()));
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.error = Some(format!("打开浮动窗口失败: {error}"));
                        cx.notify();
                    });
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn sync_perf(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = self.perf.clone() else {
            return;
        };
        let session = self.active.and_then(|i| self.tabs.get(i)).map(|tab| {
            let view = tab.view.read(cx);
            (view.id.clone(), view.spec.clone())
        });
        panel.update(cx, |panel, cx| panel.set_session(session, cx));
    }

    /// 用活动标签的参数预填快速连接，没有标签页时就是一张空表单。
    fn quick_connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let initial = self.active.and_then(|i| self.tabs.get(i)).map(|tab| {
            let view = tab.view.read(cx);
            let spec = view.spec.clone();
            SshSession {
                id: String::new(),
                name: view.title.to_string(),
                host: spec.host,
                port: spec.port,
                username: spec.username,
                password: spec.password,
                key_path: spec.key_path,
                auth: default_auth(),
            }
        });
        let entity = cx.entity();
        open_quick_connect(
            window,
            cx,
            self.lang,
            initial,
            move |title, spec, window, cx| {
                entity.update(cx, |this, cx| this.open_tab(title, spec, window, cx));
            },
        );
    }

    fn open_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entity = cx.entity();
        let lang = self.lang;
        import_flow::open_import_picker(window, cx, lang, move |kind, window, cx| {
            entity.update(cx, |this, cx| this.run_import(kind, window, cx));
        });
    }

    fn run_import(&mut self, kind: ImportKind, window: &mut Window, cx: &mut Context<Self>) {
        let picked = match kind {
            ImportKind::SecureCrt => rfd::FileDialog::new()
                .add_filter("SecureCRT Sessions", &["xml"])
                .pick_file(),
            ImportKind::WinScp => rfd::FileDialog::new()
                .add_filter("WinSCP.ini", &["ini"])
                .pick_file(),
        };
        let Some(path) = picked else {
            return;
        };
        let text = match import_flow::read_text(&path) {
            Ok(text) => text,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        let imported = match import_flow::parse(kind, &text) {
            Ok(nodes) => nodes,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        if imported.is_empty() {
            self.error = Some(tr(self.lang, "没有可导入的会话", "Nothing to import").into());
            cx.notify();
            return;
        }
        let plan = import_flow::plan(&self.tree.sessions, imported);
        if plan.conflicts.is_empty() {
            import_flow::append(&mut self.tree, plan.to_append);
            self.persist(cx);
        } else {
            self.show_import_conflict(plan, window, cx);
        }
    }

    /// 冲突三选：只新增 / 新增并覆盖口令 / 取消。
    fn show_import_conflict(
        &mut self,
        plan: ImportPlan,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entity = cx.entity();
        let lang = self.lang;
        let new_count = plan.new_count;
        let conflict_count = plan.conflicts.len();
        let pending = std::rc::Rc::new(std::cell::RefCell::new(Some(plan)));
        import_flow::open_conflict_dialog(
            window,
            cx,
            lang,
            new_count,
            conflict_count,
            move |overwrite, _, cx| {
                let Some(plan) = pending.borrow_mut().take() else {
                    return;
                };
                entity.update(cx, |this, cx| {
                    import_flow::append(&mut this.tree, plan.to_append);
                    if overwrite {
                        import_flow::overwrite(&mut this.tree, &plan.conflicts);
                    }
                    this.persist(cx);
                });
            },
        );
    }

    fn toggle_folder(&mut self, id: &str, cx: &mut Context<Self>) {
        fn walk(nodes: &mut [Node], id: &str) -> bool {
            for n in nodes.iter_mut() {
                if let Node::Folder {
                    id: fid,
                    expanded,
                    children,
                    ..
                } = n
                {
                    if fid == id {
                        *expanded = !*expanded;
                        return true;
                    }
                    if walk(children, id) {
                        return true;
                    }
                }
            }
            false
        }
        walk(&mut self.tree.sessions, id);
        cx.notify();
    }

    fn session(&self, id: &str) -> Option<&SshSession> {
        fn find<'a>(nodes: &'a [Node], id: &str) -> Option<&'a SshSession> {
            for n in nodes {
                match n {
                    Node::Folder { children, .. } => {
                        if let Some(s) = find(children, id) {
                            return Some(s);
                        }
                    }
                    Node::Session(s) if s.id == id => return Some(s),
                    Node::Session(_) => {}
                }
            }
            None
        }
        find(&self.tree.sessions, id)
    }

    fn session_mut(&mut self, id: &str) -> Option<&mut SshSession> {
        fn find<'a>(nodes: &'a mut [Node], id: &str) -> Option<&'a mut SshSession> {
            for n in nodes {
                match n {
                    Node::Folder { children, .. } => {
                        if let Some(s) = find(children, id) {
                            return Some(s);
                        }
                    }
                    Node::Session(s) if s.id == id => return Some(s),
                    Node::Session(_) => {}
                }
            }
            None
        }
        find(&mut self.tree.sessions, id)
    }

    fn folder_mut(&mut self, id: &str) -> Option<&mut Vec<Node>> {
        fn find<'a>(nodes: &'a mut [Node], id: &str) -> Option<&'a mut Vec<Node>> {
            for n in nodes {
                if let Node::Folder {
                    id: fid, children, ..
                } = n
                {
                    if fid == id {
                        return Some(children);
                    }
                    if let Some(found) = find(children, id) {
                        return Some(found);
                    }
                }
            }
            None
        }
        find(&mut self.tree.sessions, id)
    }

    fn remove_node(&mut self, id: &str) -> Option<Node> {
        fn take(nodes: &mut Vec<Node>, id: &str) -> Option<Node> {
            if let Some(index) = nodes.iter().position(|n| match n {
                Node::Folder { id: fid, .. } => fid == id,
                Node::Session(s) => s.id == id,
            }) {
                return Some(nodes.remove(index));
            }
            for n in nodes.iter_mut() {
                if let Node::Folder { children, .. } = n {
                    if let Some(found) = take(children, id) {
                        return Some(found);
                    }
                }
            }
            None
        }
        take(&mut self.tree.sessions, id)
    }

    fn rename_node(&mut self, id: &str, name: String) {
        if let Some(session) = self.session_mut(id) {
            session.name = name;
            return;
        }
        fn walk(nodes: &mut [Node], id: &str, name: &str) -> bool {
            for n in nodes.iter_mut() {
                if let Node::Folder {
                    id: fid,
                    name: fname,
                    children,
                    ..
                } = n
                {
                    if fid == id {
                        *fname = name.to_string();
                        return true;
                    }
                    if walk(children, id, name) {
                        return true;
                    }
                }
            }
            false
        }
        walk(&mut self.tree.sessions, id, &name);
    }

    fn persist(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = sessions::save_tree(&self.tree) {
            self.error = Some(error);
        }
        cx.notify();
    }

    fn apply_draft(&mut self, draft: SessionDraft, cx: &mut Context<Self>) {
        let auth = if draft.auth.is_empty() {
            default_auth()
        } else {
            draft.auth
        };
        if let Some(id) = draft.editing.clone() {
            if let Some(session) = self.session_mut(&id) {
                session.name = draft.name;
                session.host = draft.host;
                session.port = draft.port;
                session.username = draft.username;
                session.password = draft.password;
                session.key_path = draft.key_path;
                session.auth = auth;
            }
        } else {
            let id = new_id("s");
            let session = SshSession {
                id: id.clone(),
                name: draft.name,
                host: draft.host,
                port: draft.port,
                username: draft.username,
                password: draft.password,
                key_path: draft.key_path,
                auth,
            };
            match draft.parent {
                Some(parent) => match self.folder_mut(&parent) {
                    Some(children) => children.push(Node::Session(session)),
                    None => self.tree.sessions.push(Node::Session(session)),
                },
                None => self.tree.sessions.push(Node::Session(session)),
            }
            self.selected = Some(id);
        }
        self.persist(cx);
    }

    fn add_folder(&mut self, parent: Option<String>, name: String, cx: &mut Context<Self>) {
        let node = Node::Folder {
            id: new_id("f"),
            name,
            expanded: true,
            children: Vec::new(),
        };
        match parent.and_then(|id| self.folder_mut(&id)) {
            Some(children) => children.push(node),
            None => self.tree.sessions.push(node),
        }
        self.persist(cx);
    }

    fn duplicate_session(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(original) = self.session(id).cloned() else {
            return;
        };
        let copy = SshSession {
            id: new_id("s"),
            name: format!("{} (1)", original.name),
            auth: original.auth.clone(),
            ..original
        };
        let new_id = copy.id.clone();
        if let Some(index) = self
            .tree
            .sessions
            .iter()
            .position(|n| matches!(n, Node::Session(s) if s.id == id))
        {
            self.tree.sessions.insert(index + 1, Node::Session(copy));
        } else if let Some(children) = find_parent_list(&mut self.tree.sessions, id) {
            children.push(Node::Session(copy));
        } else {
            self.tree.sessions.push(Node::Session(copy));
        }
        self.selected = Some(new_id);
        self.persist(cx);
    }

    fn node_at(&self, id: &str) -> Option<Node> {
        fn find(nodes: &[Node], id: &str) -> Option<Node> {
            for n in nodes {
                if sess_node_id(n) == id {
                    return Some(n.clone());
                }
                if let Node::Folder { children, .. } = n {
                    if let Some(found) = find(children, id) {
                        return Some(found);
                    }
                }
            }
            None
        }
        find(&self.tree.sessions, id)
    }

    fn parent_of(&self, id: &str) -> Option<String> {
        fn walk(nodes: &[Node], id: &str) -> Option<String> {
            for n in nodes {
                if let Node::Folder {
                    id: fid, children, ..
                } = n
                {
                    if children.iter().any(|c| sess_node_id(c) == id) {
                        return Some(fid.clone());
                    }
                    if let Some(found) = walk(children, id) {
                        return Some(found);
                    }
                }
            }
            None
        }
        walk(&self.tree.sessions, id)
    }

    fn expand_folder(&mut self, id: &str) {
        fn walk(nodes: &mut [Node], id: &str) -> bool {
            for n in nodes.iter_mut() {
                if let Node::Folder {
                    id: fid,
                    expanded,
                    children,
                    ..
                } = n
                {
                    if fid == id {
                        *expanded = true;
                        return true;
                    }
                    if walk(children, id) {
                        return true;
                    }
                }
            }
            false
        }
        walk(&mut self.tree.sessions, id);
    }

    fn cut_session(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(node) = self.node_at(id) {
            self.sess_clip = Some(SessClip { node, cut: true });
            cx.notify();
        }
    }

    fn copy_session(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(node) = self.node_at(id) {
            self.sess_clip = Some(SessClip { node, cut: false });
            cx.notify();
        }
    }

    fn sess_paste(&mut self, dest: Option<String>, cx: &mut Context<Self>) {
        let Some(clip) = self.sess_clip.clone() else {
            return;
        };
        if clip.cut
            && dest
                .as_deref()
                .is_some_and(|d| sess_subtree_has(&clip.node, d))
        {
            self.error = Some(
                tr(
                    self.lang,
                    "不能把分组移动到它自己里",
                    "Cannot move a folder into itself",
                )
                .to_string(),
            );
            cx.notify();
            return;
        }
        if clip.cut {
            self.remove_node(sess_node_id(&clip.node));
        }
        let node = if clip.cut {
            clip.node
        } else {
            sess_clone_with_new_ids(&clip.node)
        };
        let new_id = sess_node_id(&node).to_string();
        if let Some(folder) = dest.clone() {
            self.expand_folder(&folder);
            match self.folder_mut(&folder) {
                Some(children) => children.push(node),
                None => self.tree.sessions.push(node),
            }
        } else {
            self.tree.sessions.push(node);
        }
        if clip.cut {
            self.sess_clip = None;
        }
        self.selected = Some(new_id);
        self.persist(cx);
    }

    fn tree_cut(&mut self, commands: bool, cx: &mut Context<Self>) {
        if commands {
            if let Some(id) = self.cmd_selected.clone() {
                if let Some(node) = self.cmd_node(&id).cloned() {
                    self.cmd_clip = Some(CmdClip { node, cut: true });
                }
            }
            cx.notify();
        } else if let Some(id) = self.selected.clone() {
            self.cut_session(&id, cx);
        }
    }

    fn tree_copy(&mut self, commands: bool, cx: &mut Context<Self>) {
        if commands {
            if let Some(id) = self.cmd_selected.clone() {
                if let Some(node) = self.cmd_node(&id).cloned() {
                    self.cmd_clip = Some(CmdClip { node, cut: false });
                }
            }
            cx.notify();
        } else if let Some(id) = self.selected.clone() {
            self.copy_session(&id, cx);
        }
    }

    fn tree_paste(&mut self, commands: bool, cx: &mut Context<Self>) {
        if commands {
            let dest = match self.cmd_selected.clone() {
                Some(id) if self.cmd_is_folder(&id) => Some(id),
                Some(id) => self.cmd_parent_of(&id),
                None => None,
            };
            self.cmd_paste(dest, cx);
        } else {
            let dest = match self.selected.clone() {
                Some(id) if self.is_folder(&id) => Some(id),
                Some(id) => self.parent_of(&id),
                None => None,
            };
            self.sess_paste(dest, cx);
        }
    }

    fn connect_session(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session(id).cloned() else {
            return;
        };
        let needs_password =
            session.password.trim().is_empty() && session.key_path.trim().is_empty();
        if !needs_password {
            self.open_tab(session.name.clone(), connect_spec(&session), window, cx);
            return;
        }

        let entity = cx.entity();
        let skip_entity = entity.clone();
        let for_submit = session.clone();
        let for_skip = session.clone();
        let save_id = session.id.clone();
        open_password_prompt(
            window,
            cx,
            self.lang,
            &session.username,
            move |user, password, save, window, cx| {
                let mut spec = connect_spec(&for_submit);
                spec.username = user.clone();
                spec.password = password.clone();
                let title = for_submit.name.clone();
                let save_id = save_id.clone();
                entity.update(cx, |this, cx| {
                    if save {
                        if let Some(session) = this.session_mut(&save_id) {
                            session.username = user;
                            session.password = password;
                        }
                        this.persist(cx);
                    }
                    this.open_tab(title, spec, window, cx);
                });
            },
            move |user, window, cx| {
                let mut spec = connect_spec(&for_skip);
                spec.username = user;
                let title = for_skip.name.clone();
                skip_entity.update(cx, |this, cx| this.open_tab(title, spec, window, cx));
            },
        );
    }

    fn open_tab(
        &mut self,
        title: String,
        spec: ConnectSpec,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.next_tab += 1;
        let id = format!("term-{}", self.next_tab);
        let backend = self.backend.clone();
        let theme = self.theme.clone();
        let size = self.mono_size;
        let view = cx.new(|cx| {
            let mut v = TerminalView::new(id.clone(), spec, backend, theme, size, cx);
            v.title = title.into();
            v
        });
        let sub = cx.subscribe_in(
            &view,
            window,
            |this, entity, event: &TabEvent, window, cx| {
                this.on_tab_event(entity, event, window, cx);
            },
        );
        view.update(cx, |v, cx| v.focus_terminal(window, cx));
        self.tabs.push(TabEntry { view, _sub: sub });
        self.active = Some(self.tabs.len() - 1);
        self.sync_panels(cx);
        cx.notify();
    }

    fn on_tab_event(
        &mut self,
        entity: &Entity<TerminalView>,
        event: &TabEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TabEvent::Reconnect => {
                if let Some(index) = self.tabs.iter().position(|tab| tab.view == *entity) {
                    self.reconnect_tab(index, window, cx);
                    return;
                }
            }
            TabEvent::Zoom(zoom) => {
                let size = match zoom {
                    Zoom::In => self.mono_size + 1.,
                    Zoom::Out => self.mono_size - 1.,
                    Zoom::Reset => DEFAULT_MONO,
                };
                self.set_font_size(size, window, cx);
                return;
            }
            TabEvent::QuickConnect => {
                self.quick_connect(window, cx);
                return;
            }
            _ => {}
        }
        cx.notify();
    }

    /// 字号对所有标签页生效，并写回 ui.json；范围与旧版缩放一致（9–32px）
    fn set_font_size(&mut self, size: f32, window: &mut Window, cx: &mut Context<Self>) {
        let size = size.clamp(9., 32.);
        if (size - self.mono_size).abs() < f32::EPSILON {
            return;
        }
        self.mono_size = size;
        self.theme.mono_size = px(size);
        self.prefs.font_size = size;
        let _ = settings::save(&self.prefs);
        for tab in &self.tabs {
            tab.view
                .update(cx, |v, cx| v.set_font_size(size, window, cx));
        }
        cx.notify();
    }

    /// 原地重连：关掉旧连接，用同一份参数新建 TerminalView，标签位置不变
    fn reconnect_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        let (old_id, spec, title) = {
            let view = tab.view.read(cx);
            (view.id.clone(), view.spec.clone(), view.title.to_string())
        };
        let mgr = self.backend.terminals.clone();
        self.backend.spawn(async move {
            terminal::close(mgr, old_id).await;
        });

        self.next_tab += 1;
        let term_id = format!("term-{}", self.next_tab);
        let backend = self.backend.clone();
        let theme = self.theme.clone();
        let size = self.mono_size;
        let view = cx.new(|cx| {
            let mut v = TerminalView::new(term_id, spec, backend, theme, size, cx);
            v.title = title.into();
            v
        });
        let sub = cx.subscribe_in(
            &view,
            window,
            |this, entity, event: &TabEvent, window, cx| {
                this.on_tab_event(entity, event, window, cx);
            },
        );
        view.update(cx, |v, cx| v.focus_terminal(window, cx));
        self.tabs[index] = TabEntry { view, _sub: sub };
        self.active = Some(index);
        self.sync_panels(cx);
        cx.notify();
    }

    fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        let entry = self.tabs.remove(index);
        let id = entry.view.read(cx).id.clone();
        let mgr = self.backend.terminals.clone();
        self.backend.spawn(async move {
            terminal::close(mgr, id).await;
        });
        if self.tabs.is_empty() {
            self.active = None;
        } else if let Some(active) = self.active {
            self.active = Some(active.min(self.tabs.len() - 1));
        }
        self.sync_panels(cx);
        cx.notify();
    }

    /// 从后往前关，避免先删小下标把后面待删的下标挤错位
    fn close_many(&mut self, mut indices: Vec<usize>, cx: &mut Context<Self>) {
        indices.sort_unstable();
        indices.dedup();
        for index in indices.into_iter().rev() {
            self.close_tab(index, cx);
        }
    }

    fn rename_tab(&mut self, index: usize, name: String, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get(index) {
            tab.view.update(cx, |view, cx| {
                view.title = name.into();
                cx.notify();
            });
        }
        cx.notify();
    }

    fn titlebar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme.clone();
        let lang = self.lang;
        let entity = cx.entity();

        let menu = |id: &'static str,
                    label: SharedString,
                    build: fn(
            PopupMenu,
            Entity<OneCrt>,
            Lang,
            &mut Window,
            &mut Context<PopupMenu>,
        ) -> PopupMenu|
         -> gpui::AnyElement {
            let app = entity.clone();
            menu_button(id, label, &t, move |menu, window, cx| {
                build(menu, app.clone(), lang, window, cx).min_w(px(250.))
            })
            .into_any_element()
        };

        let maximized = window.is_maximized();
        let max_icon = if maximized {
            IconName::WindowRestore
        } else {
            IconName::WindowMaximize
        };

        div()
            .id("titlebar")
            .h(px(36.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .bg(t.bg)
            .border_b_1()
            .border_color(t.line)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .pl(px(8.))
                    .pr(px(4.))
                    .child(menu("menu-file", tr(lang, "文件", "File"), file_menu))
                    .child(menu("menu-edit", tr(lang, "编辑", "Edit"), edit_menu))
                    .child(menu(
                        "menu-terminal",
                        tr(lang, "终端", "Terminal"),
                        terminal_menu,
                    ))
                    .child(menu("menu-view", tr(lang, "视图", "View"), view_menu))
                    .child(menu("menu-help", tr(lang, "帮助", "Help"), help_menu)),
            )
            .child(
                div()
                    .id("titlebar-drag")
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .overflow_hidden()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(MouseButton::Left, |event, window, cx| {
                        if event.click_count == 1 && crate::window_drag::begin_window_drag(window) {
                            cx.stop_propagation();
                        }
                    })
                    // 宽度上限只能加在标题文字上：加在 drag 容器上会让 flex_1 提前长满 560 就停手，
                    // 窗口越宽，剩下的空间越会堆到容器后面，把右侧三个窗口按钮整体往左挤。
                    .child(
                        div()
                            .max_w(px(560.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(12.5))
                            .text_color(t.text_2)
                            .child(self.window_title.clone()),
                    ),
            )
            .child(window_control(
                "win-min",
                WindowControlArea::Min,
                IconName::WindowMinimize,
                &t,
                false,
            ))
            .child(window_control(
                "win-max",
                WindowControlArea::Max,
                max_icon,
                &t,
                false,
            ))
            .child(window_control(
                "win-close",
                WindowControlArea::Close,
                IconName::X,
                &t,
                true,
            ))
    }

    fn icon_toolbar(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme.clone();
        let active = self.active.is_some();
        let files_open = self.panel_visible(PanelId::Files);
        let multi = self.tabs.len() > 1;
        let t2 = t.clone();
        let t3 = t.clone();
        let t4 = t.clone();
        let t5 = t.clone();
        let t6 = t.clone();
        let t7 = t.clone();
        let t8 = t.clone();
        let t9 = t.clone();
        let t10 = t.clone();
        let t11 = t.clone();

        div()
            .h(px(34.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(2.))
            .px(px(6.))
            .bg(t.bg)
            .border_b_1()
            .border_color(t.line)
            .child(toolbar_icon(
                "tb-quick",
                IconName::Zap,
                true,
                false,
                &t,
                cx.listener(|this, _, window, cx| this.quick_connect(window, cx)),
            ))
            .child(toolbar_icon(
                "tb-next-tab",
                IconName::Repeat,
                multi,
                false,
                &t2,
                cx.listener(|this, _, window, cx| this.cycle_tab(window, cx)),
            ))
            .child(toolbar_icon(
                "tb-reconnect",
                IconName::RefreshCw,
                active,
                false,
                &t3,
                cx.listener(|this, _, window, cx| this.reconnect_active(window, cx)),
            ))
            .child(toolbar_divider(&t))
            .child(toolbar_icon(
                "tb-copy",
                IconName::Copy,
                active,
                false,
                &t4,
                cx.listener(|this, _, _, cx| this.copy_active(cx)),
            ))
            .child(toolbar_icon(
                "tb-paste",
                IconName::Clipboard,
                active,
                false,
                &t5,
                cx.listener(|this, _, _, cx| this.paste_active(cx)),
            ))
            .child(toolbar_icon(
                "tb-print",
                IconName::Printer,
                false,
                false,
                &t6,
                |_, _, _| {},
            ))
            .child(toolbar_divider(&t))
            .child(toolbar_icon(
                "tb-files",
                IconName::ArrowUpDown,
                active,
                files_open,
                &t7,
                cx.listener(|this, _, _, cx| this.toggle_panel(PanelId::Files, cx)),
            ))
            .child(toolbar_divider(&t))
            .child(toolbar_icon(
                "tb-session-options",
                IconName::Settings,
                active,
                false,
                &t8,
                cx.listener(|this, _, window, cx| this.edit_active_session(window, cx)),
            ))
            .child(toolbar_icon(
                "tb-keymap",
                IconName::Keyboard,
                false,
                false,
                &t9,
                |_, _, _| {},
            ))
            .child(toolbar_icon(
                "tb-port-forward",
                IconName::Wrench,
                false,
                false,
                &t10,
                |_, _, _| {},
            ))
            .child(div().flex_1())
            .child(toolbar_divider(&t10))
            .child(toolbar_icon(
                "tb-theme",
                if t.mode == Mode::Light {
                    IconName::Moon
                } else {
                    IconName::Sun
                },
                true,
                false,
                &t11,
                cx.listener(|this, _, _, cx| this.toggle_theme(cx)),
            ))
    }

    fn cycle_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() < 2 {
            return;
        }
        let next = self.active.map(|i| (i + 1) % self.tabs.len()).unwrap_or(0);
        self.active = Some(next);
        self.tabs[next]
            .view
            .update(cx, |view, cx| view.focus_terminal(window, cx));
        self.sync_panels(cx);
        cx.notify();
    }

    /// 打开活动会话的属性表单；标签来自快速连接/克隆时退回会话树里的选中项。
    fn edit_active_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_selected(window, cx);
    }

    fn new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_panel == PanelId::Commands {
            let entity = cx.entity();
            let lang = self.lang;
            let parent = self
                .cmd_selected
                .clone()
                .filter(|id| self.cmd_is_folder(id));
            open_command_form(
                window,
                cx,
                lang,
                tr(lang, "新建命令", "New Command"),
                None,
                true,
                move |fields, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.cmd_apply_fields(None, parent.clone(), fields, cx)
                    });
                },
            );
            return;
        }
        let entity = cx.entity();
        let lang = self.lang;
        let parent = self.selected.clone().filter(|id| self.is_folder(id));
        open_session_form(window, cx, lang, None, parent, move |draft, _, cx| {
            entity.update(cx, |this, cx| this.apply_draft(draft, cx));
        });
    }

    fn new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let lang = self.lang;
        let entity = cx.entity();
        if self.active_panel == PanelId::Commands {
            let parent = self
                .cmd_selected
                .clone()
                .filter(|id| self.cmd_is_folder(id));
            open_text_prompt(
                window,
                cx,
                lang,
                tr(lang, "新建分组", "New Folder"),
                "",
                tr(lang, "分组名称", "Folder name").into(),
                move |name, _, cx| {
                    let node = CmdNode::Folder {
                        id: new_id("cf"),
                        name,
                        expanded: true,
                        children: Vec::new(),
                    };
                    entity.update(cx, |this, cx| {
                        this.cmd_insert(parent.clone(), node);
                        this.cmd_persist(cx);
                    });
                },
            );
            return;
        }
        let parent = self.selected.clone().filter(|id| self.is_folder(id));
        open_text_prompt(
            window,
            cx,
            lang,
            tr(lang, "新建分组", "New Folder"),
            "",
            tr(lang, "分组名称", "Folder name").into(),
            move |name, _, cx| {
                entity.update(cx, |this, cx| this.add_folder(parent.clone(), name, cx));
            },
        );
    }

    fn edit_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let lang = self.lang;
        let entity = cx.entity();
        if self.active_panel == PanelId::Commands {
            let Some(id) = self.cmd_selected.clone() else {
                return;
            };
            let Some(CmdNode::Command { name, text, .. }) = self.cmd_node(&id).cloned() else {
                return;
            };
            open_command_form(
                window,
                cx,
                lang,
                tr(lang, "命令属性", "Command Properties"),
                Some((name, text)),
                true,
                move |fields, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.cmd_apply_fields(Some(id.clone()), None, fields, cx)
                    });
                },
            );
            return;
        }
        let Some(id) = self.selected.clone() else {
            return;
        };
        let Some(session) = self.session(&id).cloned() else {
            return;
        };
        open_session_form(
            window,
            cx,
            lang,
            Some(session),
            None,
            move |draft, _, cx| {
                entity.update(cx, |this, cx| this.apply_draft(draft, cx));
            },
        );
    }

    fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let lang = self.lang;
        let entity = cx.entity();
        if self.active_panel == PanelId::Commands {
            let Some(id) = self.cmd_selected.clone() else {
                return;
            };
            let is_folder = self.cmd_is_folder(&id);
            let (title, message) = if is_folder {
                (
                    tr(lang, "删除分组", "Delete Folder"),
                    tr(
                        lang,
                        "分组及其中所有命令都会被删除。",
                        "The folder and every command inside will be removed.",
                    ),
                )
            } else {
                (
                    tr(lang, "删除命令", "Delete Command"),
                    tr(lang, "该命令将被删除。", "This command will be removed."),
                )
            };
            confirm_dialog(window, cx, lang, title, message, move |_, cx| {
                entity.update(cx, |this, cx| {
                    this.cmd_remove(&id);
                    if this.cmd_selected.as_deref() == Some(id.as_str()) {
                        this.cmd_selected = None;
                    }
                    this.cmd_persist(cx);
                });
            });
            return;
        }
        let Some(id) = self.selected.clone() else {
            return;
        };
        let is_folder = self.is_folder(&id);
        let (title, message) = if is_folder {
            (
                tr(lang, "删除分组", "Delete Folder"),
                tr(
                    lang,
                    "分组及其中所有会话都会被删除。",
                    "The folder and every session inside will be removed.",
                ),
            )
        } else {
            (
                tr(lang, "删除会话", "Delete Session"),
                tr(
                    lang,
                    "该会话配置将被删除。",
                    "This session will be removed.",
                ),
            )
        };
        confirm_dialog(window, cx, lang, title, message, move |_, cx| {
            entity.update(cx, |this, cx| {
                this.remove_node(&id);
                if this.selected.as_deref() == Some(id.as_str()) {
                    this.selected = None;
                }
                this.persist(cx);
            });
        });
    }

    fn copy_active(&self, cx: &Context<Self>) {
        if let Some(tab) = self.active.and_then(|i| self.tabs.get(i)) {
            tab.view.read(cx).copy();
        }
    }

    fn paste_active(&self, cx: &Context<Self>) {
        if let Some(tab) = self.active.and_then(|i| self.tabs.get(i)) {
            tab.view.read(cx).paste();
        }
    }

    fn reconnect_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.active {
            self.reconnect_tab(index, window, cx);
        }
    }

    fn disconnect_active(&self, cx: &Context<Self>) {
        if let Some(tab) = self.active.and_then(|i| self.tabs.get(i)) {
            tab.view.read(cx).disconnect();
        }
    }

    fn clear_active(&self, cx: &Context<Self>) {
        if let Some(tab) = self.active.and_then(|i| self.tabs.get(i)) {
            tab.view.read(cx).clear();
        }
    }

    fn interrupt_active(&self, cx: &Context<Self>) {
        if let Some(tab) = self.active.and_then(|i| self.tabs.get(i)) {
            tab.view.read(cx).interrupt();
        }
    }

    fn close_active(&mut self, cx: &mut Context<Self>) {
        if let Some(index) = self.active {
            self.close_tab(index, cx);
        }
    }

    fn rename_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.active else {
            return;
        };
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        let current = tab.view.read(cx).title.to_string();
        let lang = self.lang;
        let entity = cx.entity();
        open_text_prompt(
            window,
            cx,
            lang,
            tr(lang, "重命名标签页", "Rename Tab"),
            &current,
            tr(lang, "名称", "Name").into(),
            move |name, _, cx| {
                entity.update(cx, |this, cx| this.rename_tab(index, name, cx));
            },
        );
    }

    fn zoom_by(&mut self, delta: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.set_font_size(self.mono_size + delta, window, cx);
    }

    fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        self.theme = self.theme.toggle(self.mono_size);
        theme::sync_component(self.theme.mode, cx);
        self.prefs.theme = match self.theme.mode {
            Mode::Dark => "dark".into(),
            Mode::Light => "light".into(),
        };
        let _ = settings::save(&self.prefs);
        let theme = self.theme.clone();
        for tab in &self.tabs {
            tab.view.update(cx, |v, cx| {
                v.set_theme(theme.clone(), cx);
            });
        }
        if let Some(panel) = self.files.clone() {
            panel.update(cx, |panel, cx| panel.set_theme(theme.clone(), cx));
        }
        if let Some(panel) = self.perf.clone() {
            panel.update(cx, |panel, cx| panel.set_theme(theme.clone(), cx));
        }
        cx.notify();
    }

    fn toggle_lang(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.lang = match self.lang {
            Lang::Zh => Lang::En,
            Lang::En => Lang::Zh,
        };
        self.prefs.lang = match self.lang {
            Lang::Zh => "zh".into(),
            Lang::En => "en".into(),
        };
        let _ = settings::save(&self.prefs);
        let lang = self.lang;
        self.filter.update(cx, |state, cx| {
            state.set_placeholder(
                tr(lang, "按文件夹/会话名筛选", "Filter folders or sessions"),
                window,
                cx,
            );
        });
        self.cmd_filter.update(cx, |state, cx| {
            state.set_placeholder(
                tr(lang, "按分组/命令名筛选", "Filter folders or commands"),
                window,
                cx,
            );
        });
        if let Some(panel) = self.files.clone() {
            panel.update(cx, |panel, cx| panel.set_lang(lang, cx));
        }
        if let Some(panel) = self.perf.clone() {
            panel.update(cx, |panel, cx| panel.set_lang(lang, cx));
        }
        cx.notify();
    }

    fn panel_body(&self, panel: PanelId, cx: &mut Context<Self>) -> Div {
        let t = self.theme.clone();
        let body: gpui::AnyElement = match panel {
            PanelId::Sessions | PanelId::Commands => self.tree_panel(panel, cx).into_any_element(),
            PanelId::Files => self
                .files
                .clone()
                .map(|panel| panel.into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            PanelId::Perf => self
                .perf
                .clone()
                .map(|panel| panel.into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
        };
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(t.bg)
            .child(body)
    }

    /// 会话/命令页签共用一套工具条 + 过滤框 + 树。
    fn tree_panel(&self, panel: PanelId, cx: &mut Context<Self>) -> Div {
        let t = self.theme.clone();
        let commands = panel == PanelId::Commands;
        let (edit_enabled, delete_enabled) = if commands {
            (
                self.cmd_selected
                    .as_deref()
                    .map(|id| !self.cmd_is_folder(id))
                    .unwrap_or(false),
                self.cmd_selected.is_some(),
            )
        } else {
            (
                self.selected
                    .as_deref()
                    .map(|id| !self.is_folder(id))
                    .unwrap_or(false),
                self.selected.is_some(),
            )
        };
        let new_icon = if commands {
            IconName::Keyboard
        } else {
            IconName::Monitor
        };
        let filter = if commands {
            self.cmd_filter.clone()
        } else {
            self.filter.clone()
        };
        let content = if commands {
            self.command_pane(cx)
        } else {
            self.session_pane(cx)
        };
        let selected_any = if commands {
            self.cmd_selected.is_some()
        } else {
            self.selected.is_some()
        };
        let can_connect = !commands
            && self
                .selected
                .as_deref()
                .map(|id| !self.is_folder(id))
                .unwrap_or(false);
        let can_paste = if commands {
            self.cmd_clip.is_some()
        } else {
            self.sess_clip.is_some()
        };

        let mut tools = div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(1.))
            .px(px(8.))
            .pt(px(2.))
            .pb(px(6.))
            .border_b_1()
            .border_color(t.line);
        if !commands {
            tools = tools.child(sidebar_icon(
                "side-connect",
                IconName::Link,
                can_connect,
                &t,
                cx.listener(|this, _, window, cx| {
                    if let Some(id) = this.selected.clone() {
                        this.connect_session(&id, window, cx);
                    }
                }),
            ));
        }
        tools = tools
            .child(sidebar_icon(
                "side-new-folder",
                IconName::FolderPlus,
                true,
                &t,
                cx.listener(|this, _, window, cx| this.new_folder(window, cx)),
            ))
            .child(sidebar_icon(
                "side-new",
                new_icon,
                true,
                &t,
                cx.listener(|this, _, window, cx| this.new_session(window, cx)),
            ))
            .child(sidebar_icon(
                "side-cut",
                IconName::Scissors,
                selected_any,
                &t,
                cx.listener(move |this, _, _, cx| this.tree_cut(commands, cx)),
            ))
            .child(sidebar_icon(
                "side-copy",
                IconName::Copy,
                selected_any,
                &t,
                cx.listener(move |this, _, _, cx| this.tree_copy(commands, cx)),
            ))
            .child(sidebar_icon(
                "side-paste",
                IconName::Clipboard,
                can_paste,
                &t,
                cx.listener(move |this, _, _, cx| this.tree_paste(commands, cx)),
            ))
            .child(sidebar_icon(
                "side-edit",
                IconName::Settings,
                edit_enabled,
                &t,
                cx.listener(|this, _, window, cx| this.edit_selected(window, cx)),
            ))
            .child(sidebar_icon(
                "side-delete",
                IconName::Trash,
                delete_enabled,
                &t,
                cx.listener(|this, _, window, cx| this.delete_selected(window, cx)),
            ));

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(tools)
            .child(
                div()
                    .flex_shrink_0()
                    .px(px(10.))
                    .pt(px(8.))
                    .pb(px(6.))
                    .child(
                        div()
                            .h(px(28.))
                            .flex()
                            .items_center()
                            .px(px(10.))
                            .rounded(px(6.))
                            .bg(t.field)
                            .border_1()
                            .border_color(t.border_strong)
                            .child(Input::new(&filter).appearance(false).text_size(px(12.))),
                    ),
            )
            .child(content)
    }

    /// 一组页签：标题栏（浮动/停靠/自动隐藏/关闭）+ 内容 + 底部页签条，对齐 VS 工具窗口。
    fn dock_group(&self, zone: DockZone, cx: &mut Context<Self>) -> Div {
        let t = self.theme.clone();
        let group = self.groups[zone.index()].clone();
        let mut el = div().flex().flex_col().bg(t.bg);
        el = match zone {
            DockZone::Left => el.w(px(self.left_w)).flex_shrink_0().h_full().border_r_1(),
            DockZone::Right => el.w(px(self.right_w)).flex_shrink_0().h_full().border_l_1(),
            DockZone::Bottom => el
                .w_full()
                .h(px(self.bottom_h))
                .flex_shrink_0()
                .border_t_1(),
            DockZone::Top => el.w_full().h(px(self.top_h)).flex_shrink_0().border_b_1(),
        }
        .border_color(t.line);
        let Some(active) = group.active else {
            return el;
        };
        el.child(self.dock_header(zone, active, false, cx))
            .child(self.panel_body(active, cx))
            .child(self.dock_tabs(&group, cx))
    }

    fn dock_header(
        &self,
        zone: DockZone,
        active: PanelId,
        flyout: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.theme.clone();
        let lang = self.lang;
        let app = cx.entity();
        let pin_theme = t.clone();
        let close_theme = t.clone();
        let drag_theme = t.clone();
        let (menu_id, pin_id, close_id) = match zone {
            DockZone::Left => ("dock-menu-left", "dock-pin-left", "dock-close-left"),
            DockZone::Right => ("dock-menu-right", "dock-pin-right", "dock-close-right"),
            DockZone::Bottom => ("dock-menu-bottom", "dock-pin-bottom", "dock-close-bottom"),
            DockZone::Top => ("dock-menu-top", "dock-pin-top", "dock-close-top"),
        };
        div()
            .h(px(30.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(1.))
            .pl(px(10.))
            .pr(px(3.))
            .bg(t.bg)
            .border_b_1()
            .border_color(t.line)
            .child(
                div()
                    .id(SharedString::from(format!("dock-hdr-{}", zone.key())))
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .text_size(px(11.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(t.text)
                    .cursor(CursorStyle::OpenHand)
                    .on_drag(DockDrag { panel: active }, move |_, _, _, cx| {
                        cx.new(|_| DockGhost {
                            panel: active,
                            theme: drag_theme.clone(),
                            lang,
                        })
                    })
                    .child(active.title(lang)),
            )
            .child(
                Button::new(menu_id)
                    .ghost()
                    .compact()
                    .icon(IconName::ChevronDown)
                    .text_color(t.text_2)
                    .dropdown_menu(move |menu, window, cx| {
                        dock_menu(menu, app.clone(), active, lang, false, window, cx)
                    }),
            )
            .child(
                div()
                    .id(pin_id)
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .cursor_pointer()
                    .text_color(pin_theme.text_2)
                    .hover(move |s| s.bg(pin_theme.bg_hover).text_color(pin_theme.text))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if flyout {
                            // 浮层里的图钉 = 重新停靠。
                            this.set_collapsed(zone, false, cx);
                        } else {
                            let collapsed = !this.groups[zone.index()].collapsed;
                            this.set_collapsed(zone, collapsed, cx);
                        }
                    }))
                    .child(Icon::new(IconName::Pin).size(px(13.))),
            )
            .child(
                div()
                    .id(close_id)
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .cursor_pointer()
                    .text_color(close_theme.text_2)
                    .hover(move |s| s.bg(close_theme.bg_hover).text_color(close_theme.text))
                    .on_click(cx.listener(move |this, _, _, cx| this.hide_panel(active, cx)))
                    .child(Icon::new(IconName::X).size(px(13.))),
            )
    }

    fn dock_tabs(&self, group: &DockGroup, cx: &mut Context<Self>) -> Div {
        let t = self.theme.clone();
        let active = group.active;
        div()
            .h(px(30.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(2.))
            .px(px(6.))
            .bg(t.bg)
            .border_t_1()
            .border_color(t.line)
            .children(
                group
                    .panels
                    .iter()
                    .map(|panel| self.dock_tab(*panel, active == Some(*panel), &t, cx)),
            )
    }

    fn dock_tab(
        &self,
        panel: PanelId,
        active: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let lang = self.lang;
        let theme = t.clone();
        let idle = t.clone();
        div()
            .id(SharedString::from(format!("dock-tab-{}", panel.key())))
            .h(px(22.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(5.))
            .rounded(px(5.))
            .text_size(px(11.5))
            .font_weight(if active {
                gpui::FontWeight::SEMIBOLD
            } else {
                gpui::FontWeight::NORMAL
            })
            .text_color(if active { t.text } else { t.text_2 })
            .when(active, |s| s.bg(t.bg_selected))
            .cursor_pointer()
            .hover(move |s| s.bg(idle.bg_hover).text_color(idle.text))
            .on_click(cx.listener(move |this, _, _, cx| this.activate_panel(panel, cx)))
            .on_drag(DockDrag { panel }, move |_, _, _, cx| {
                cx.new(|_| DockGhost {
                    panel,
                    theme: theme.clone(),
                    lang,
                })
            })
            .child(Icon::new(panel.icon()).size(px(13.)))
            .child(panel.short_title(lang))
    }

    /// 自动隐藏后的贴边页签条：左右竖排、上下横排，对齐 VS 的自动隐藏页签。
    fn dock_rail(&self, zone: DockZone, cx: &mut Context<Self>) -> Div {
        let t = self.theme.clone();
        let lang = self.lang;
        let group = self.groups[zone.index()].clone();
        let vertical = matches!(zone, DockZone::Left | DockZone::Right);
        let mut el = div().flex().gap(px(2.)).bg(t.bg);
        el = match zone {
            DockZone::Left => el
                .w(px(36.))
                .h_full()
                .flex_col()
                .items_center()
                .py(px(6.))
                .border_r_1(),
            DockZone::Right => el
                .w(px(36.))
                .h_full()
                .flex_col()
                .items_center()
                .py(px(6.))
                .border_l_1(),
            DockZone::Bottom => el
                .h(px(30.))
                .w_full()
                .flex_row()
                .items_center()
                .px(px(6.))
                .border_t_1(),
            DockZone::Top => el
                .h(px(30.))
                .w_full()
                .flex_row()
                .items_center()
                .px(px(6.))
                .border_b_1(),
        }
        .border_color(t.line);
        el.children(group.panels.iter().map(|panel| {
            let panel = *panel;
            let idle = t.clone();
            let active = group.active == Some(panel);
            let mut tab = div()
                .id(SharedString::from(format!(
                    "dock-rail-{}-{}",
                    zone.key(),
                    panel.key()
                )))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(5.))
                .rounded(px(5.))
                .cursor_pointer()
                .text_color(if active { t.accent } else { t.text_2 })
                .when(active, |s| s.bg(t.bg_selected))
                .hover(move |s| s.bg(idle.bg_hover).text_color(idle.text))
                .on_hover(cx.listener(move |this, hovered: &bool, _window, cx| {
                    this.rail_hovered(zone, panel, *hovered, cx);
                }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.peek
                        == Some(DockPeek {
                            zone,
                            panel,
                            sticky: true,
                        })
                    {
                        this.peek_close(cx);
                    } else {
                        this.rail_hover = true;
                        this.peek_open(zone, panel, true, cx);
                    }
                }));
            tab = if vertical {
                tab.w(px(28.))
                    .py(px(8.))
                    .flex_col()
                    .child(Icon::new(panel.icon()).size(px(14.)))
                    .children(panel.short_title(lang).chars().map(|ch| {
                        div()
                            .text_size(px(10.5))
                            .line_height(px(11.))
                            .child(SharedString::from(ch.to_string()))
                    }))
            } else {
                tab.h(px(22.))
                    .px(px(8.))
                    .text_size(px(11.5))
                    .child(Icon::new(panel.icon()).size(px(13.)))
                    .child(panel.short_title(lang))
            };
            tab
        }))
    }

    /// 自动隐藏的悬停浮层：贴在窄条内侧，盖在内容上，不改变布局。
    fn peek_overlay(&self, t: &Theme, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let peek = self.peek?;
        let group = self.groups[peek.zone.index()].clone();
        let active = group.active?;
        let (left, right, _, _) = self.dock_insets();
        let mut el = div()
            .id("dock-peek")
            .absolute()
            .flex()
            .flex_col()
            .bg(t.bg)
            .border_1()
            .border_color(t.border_strong)
            .on_hover(cx.listener(|this, hovered: &bool, _window, cx| {
                this.flyout_hover = *hovered;
                if *hovered {
                    this.peek_gen = this.peek_gen.wrapping_add(1);
                } else {
                    this.peek_close_later(cx);
                }
            }))
            .child(self.dock_header(peek.zone, active, true, cx))
            .child(self.panel_body(active, cx))
            .child(self.dock_tabs(&group, cx));
        el = match peek.zone {
            DockZone::Left => el.left(px(36.)).top_0().bottom_0().w(px(self.left_w)),
            DockZone::Right => el.right(px(36.)).top_0().bottom_0().w(px(self.right_w)),
            DockZone::Top => el
                .left(px(left))
                .right(px(right))
                .top(px(30.))
                .h(px(self.top_h)),
            DockZone::Bottom => el
                .left(px(left))
                .right(px(right))
                .bottom(px(30.))
                .h(px(self.bottom_h)),
        };
        Some(el.into_any_element())
    }

    fn zone_span(&self, zone: DockZone) -> f32 {
        let group = &self.groups[zone.index()];
        if group.panels.is_empty() {
            return 0.;
        }
        if group.collapsed {
            return if matches!(zone, DockZone::Bottom | DockZone::Top) {
                30.
            } else {
                36.
            };
        }
        match zone {
            DockZone::Left => self.left_w,
            DockZone::Right => self.right_w,
            DockZone::Bottom => self.bottom_h,
            DockZone::Top => self.top_h,
        }
    }

    /// 落点按区域判定：左右边条、底部区域，其余视为浮动。
    fn target_at(&self, rel: Point<Pixels>, area: gpui::Size<Pixels>) -> DockTarget {
        let x = f32::from(rel.x);
        let y = f32::from(rel.y);
        let w = f32::from(area.width);
        let h = f32::from(area.height);
        let left = self.zone_span(DockZone::Left);
        let left = if left > 0. {
            left
        } else {
            (w * 0.18).min(140.)
        };
        let right = self.zone_span(DockZone::Right);
        let right = if right > 0. {
            right
        } else {
            (w * 0.18).min(140.)
        };
        let bottom = self.zone_span(DockZone::Bottom);
        let bottom = if bottom > 0. {
            bottom
        } else {
            (h * 0.2).min(140.)
        };
        let top = self.zone_span(DockZone::Top);
        let top = if top > 0. { top } else { (h * 0.2).min(140.) };
        if x < left {
            DockTarget::Zone(DockZone::Left)
        } else if x > w - right {
            DockTarget::Zone(DockZone::Right)
        } else if y < top {
            DockTarget::Zone(DockZone::Top)
        } else if y > h - bottom {
            DockTarget::Zone(DockZone::Bottom)
        } else {
            DockTarget::Float
        }
    }

    /// VS 风格拖拽导引：目标区域半透明预览 + 光标旁的四向十字。
    fn guide_overlay(&self, t: &Theme) -> Option<Div> {
        let target = self.guide?;
        let (left, right) = (
            self.zone_span(DockZone::Left),
            self.zone_span(DockZone::Right),
        );
        let bottom = self.zone_span(DockZone::Bottom);
        let top = self.zone_span(DockZone::Top);
        let preview = match target {
            DockTarget::Zone(DockZone::Left) => div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(px(left.max(160.))),
            DockTarget::Zone(DockZone::Right) => div()
                .absolute()
                .right_0()
                .top_0()
                .bottom_0()
                .w(px(right.max(160.))),
            DockTarget::Zone(DockZone::Bottom) => div()
                .absolute()
                .left(px(left))
                .right(px(right))
                .bottom_0()
                .h(px(bottom.max(120.))),
            DockTarget::Zone(DockZone::Top) => div()
                .absolute()
                .left(px(left))
                .right(px(right))
                .top_0()
                .h(px(top.max(120.))),
            DockTarget::Float => div()
                .absolute()
                .left(px(left))
                .right(px(right))
                .top(px(top))
                .bottom(px(bottom)),
        }
        .bg(t.accent.opacity(0.14))
        .border_1()
        .border_color(t.accent)
        .rounded(px(4.));
        let cell = |icon: IconName, hot: bool, x: f32, y: f32| {
            let theme = t.clone();
            let mut cell = div()
                .absolute()
                .left(px(x))
                .top(px(y))
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .border_1()
                .border_color(theme.border_strong);
            if hot {
                cell = cell
                    .bg(theme.accent)
                    .text_color(gpui::rgb(0x0d1b10))
                    .border_color(theme.accent);
            } else {
                cell = cell.bg(theme.bg_selected).text_color(theme.text_2);
            }
            cell.child(Icon::new(icon).size(px(13.)))
        };
        // 十字钉在停靠区正中（对齐 VS 的停靠导引），只让高亮格跟着落点换，
        // 不随拖拽光标漂移。
        let area = self.guide_area.unwrap_or((0., 0.));
        let cx_at = ((area.0 - 78.) / 2.).max(4.);
        let cy_at = ((area.1 - 78.) / 2.).max(4.);
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .child(preview)
                .child(
                    div()
                        .absolute()
                        .left(px(cx_at))
                        .top(px(cy_at))
                        .w(px(78.))
                        .h(px(78.))
                        .child(cell(
                            IconName::PanelTop,
                            target == DockTarget::Zone(DockZone::Top),
                            27.,
                            0.,
                        ))
                        .child(cell(
                            IconName::PanelLeft,
                            target == DockTarget::Zone(DockZone::Left),
                            0.,
                            27.,
                        ))
                        .child(cell(
                            IconName::Square,
                            target == DockTarget::Float,
                            27.,
                            27.,
                        ))
                        .child(cell(
                            IconName::PanelRight,
                            target == DockTarget::Zone(DockZone::Right),
                            54.,
                            27.,
                        ))
                        .child(cell(
                            IconName::PanelBottom,
                            target == DockTarget::Zone(DockZone::Bottom),
                            27.,
                            54.,
                        )),
                ),
        )
    }

    fn drop_panel(&mut self, panel: PanelId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.guide.take() else {
            return;
        };
        self.guide_area = None;
        match target {
            DockTarget::Zone(zone) => self.dock_panel(panel, zone, cx),
            DockTarget::Float => self.float_panel(panel, window, cx),
        }
    }

    fn cmd_is_folder(&self, id: &str) -> bool {
        matches!(self.cmd_node(id), Some(CmdNode::Folder { .. }))
    }

    fn cmd_node(&self, id: &str) -> Option<&CmdNode> {
        fn find<'a>(nodes: &'a [CmdNode], id: &str) -> Option<&'a CmdNode> {
            for n in nodes {
                if cmd_id(n) == id {
                    return Some(n);
                }
                if let CmdNode::Folder { children, .. } = n {
                    if let Some(found) = find(children, id) {
                        return Some(found);
                    }
                }
            }
            None
        }
        find(&self.commands.commands, id)
    }

    fn cmd_node_mut(&mut self, id: &str) -> Option<&mut CmdNode> {
        fn find<'a>(nodes: &'a mut [CmdNode], id: &str) -> Option<&'a mut CmdNode> {
            for n in nodes.iter_mut() {
                if cmd_id(n) == id {
                    return Some(n);
                }
                if let CmdNode::Folder { children, .. } = n {
                    if let Some(found) = find(children, id) {
                        return Some(found);
                    }
                }
            }
            None
        }
        find(&mut self.commands.commands, id)
    }

    fn cmd_parent_of(&self, id: &str) -> Option<String> {
        fn walk(nodes: &[CmdNode], id: &str) -> Option<String> {
            for n in nodes {
                if let CmdNode::Folder {
                    id: fid, children, ..
                } = n
                {
                    if children.iter().any(|c| cmd_id(c) == id) {
                        return Some(fid.clone());
                    }
                    if let Some(found) = walk(children, id) {
                        return Some(found);
                    }
                }
            }
            None
        }
        walk(&self.commands.commands, id)
    }

    fn cmd_toggle(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(CmdNode::Folder { expanded, .. }) = self.cmd_node_mut(id) {
            *expanded = !*expanded;
        }
        self.cmd_persist(cx);
    }

    fn cmd_remove(&mut self, id: &str) -> Option<CmdNode> {
        fn take(nodes: &mut Vec<CmdNode>, id: &str) -> Option<CmdNode> {
            if let Some(index) = nodes.iter().position(|n| cmd_id(n) == id) {
                return Some(nodes.remove(index));
            }
            for n in nodes.iter_mut() {
                if let CmdNode::Folder { children, .. } = n {
                    if let Some(found) = take(children, id) {
                        return Some(found);
                    }
                }
            }
            None
        }
        take(&mut self.commands.commands, id)
    }

    fn cmd_insert(&mut self, dest: Option<String>, node: CmdNode) {
        match dest {
            Some(folder) => match self.cmd_node_mut(&folder) {
                Some(CmdNode::Folder {
                    expanded, children, ..
                }) => {
                    *expanded = true;
                    children.push(node);
                }
                _ => self.commands.commands.push(node),
            },
            None => self.commands.commands.push(node),
        }
    }

    fn cmd_persist(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = sessions::save_commands(&self.commands) {
            self.error = Some(error);
        }
        cx.notify();
    }

    fn cmd_apply_fields(
        &mut self,
        editing: Option<String>,
        parent: Option<String>,
        fields: CommandFields,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = editing {
            if let Some(node) = self.cmd_node_mut(&id) {
                match node {
                    CmdNode::Folder { name, .. } => *name = fields.name,
                    CmdNode::Command { name, text, .. } => {
                        *name = fields.name;
                        *text = fields.text;
                    }
                }
            }
        } else {
            let id = new_id("c");
            let node = CmdNode::Command {
                id: id.clone(),
                name: fields.name,
                text: fields.text,
            };
            self.cmd_insert(parent, node);
            self.cmd_selected = Some(id);
        }
        self.cmd_persist(cx);
    }

    fn cmd_paste(&mut self, dest: Option<String>, cx: &mut Context<Self>) {
        let Some(clip) = self.cmd_clip.clone() else {
            return;
        };
        if clip.cut && dest.as_deref().is_some_and(|d| subtree_has(&clip.node, d)) {
            return;
        }
        if clip.cut {
            self.cmd_remove(cmd_id(&clip.node));
            self.cmd_clip = None;
        }
        let node = clone_with_new_ids(&clip.node);
        self.cmd_selected = Some(cmd_id(&node).to_string());
        self.cmd_insert(dest, node);
        self.cmd_persist(cx);
    }

    fn cmd_tab_ready(&self, index: usize, cx: &mut Context<Self>) -> bool {
        self.tabs
            .get(index)
            .is_some_and(|tab| matches!(tab.view.read(cx).state, ConnState::Ready))
    }

    fn ready_tabs(&self, cx: &mut Context<Self>) -> Vec<usize> {
        (0..self.tabs.len())
            .filter(|&index| self.cmd_tab_ready(index, cx))
            .collect()
    }

    fn send_cmd_to_active(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some(index) = self.active else {
            return;
        };
        if !self.cmd_tab_ready(index, cx) {
            return;
        }
        if let Some(tab) = self.tabs.get(index) {
            tab.view.update(cx, |view, cx| {
                view.send_command(text);
                cx.notify();
            });
        }
        cx.notify();
    }

    fn broadcast_cmd(&mut self, text: &str, cx: &mut Context<Self>) {
        for index in self.ready_tabs(cx) {
            if let Some(tab) = self.tabs.get(index) {
                tab.view.update(cx, |view, cx| {
                    view.send_command(text);
                    cx.notify();
                });
            }
        }
        cx.notify();
    }

    fn command_pane(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme.clone();
        let lang = self.lang;
        let entity = cx.entity();
        let needle = self.cmd_filter.read(cx).value().trim().to_lowercase();
        let can_send = self.active.is_some() && self.cmd_tab_ready(self.active.unwrap_or(0), cx);
        let any_ready = !self.ready_tabs(cx).is_empty();
        let has_clip = self.cmd_clip.is_some();
        let mut list = div().id("cmd-list").flex().flex_col().py(px(2.));

        for row in flatten_cmds(&self.commands.commands, 0, &needle) {
            let theme = t.clone();
            let selected = self.cmd_selected.as_deref() == Some(row.id.as_str());
            let bg = if selected { t.bg_active } else { t.bg };
            let id = row.id.clone();
            let label = row.name.clone();
            let is_folder = row.folder;
            let expanded = row.expanded;
            let depth = row.depth;
            let send_text = row.text.clone();
            let parent = if is_folder {
                Some(id.clone())
            } else {
                self.cmd_parent_of(&id)
            };

            let row_element = div()
                .id(SharedString::from(format!("cmd-{}", id)))
                .h(px(22.))
                .flex()
                .items_center()
                .gap(px(5.))
                .pl(px(4. + depth as f32 * 16.))
                .pr(px(8.))
                .bg(bg)
                .hover(move |s| s.bg(theme.bg_hover))
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener({
                        let id = id.clone();
                        let send_text = send_text.clone();
                        move |this, ev: &gpui::MouseDownEvent, _window, cx| {
                            this.cmd_selected = Some(id.clone());
                            this.active_panel = PanelId::Commands;
                            if is_folder {
                                this.cmd_toggle(&id, cx);
                            } else if ev.click_count >= 2 {
                                if let Some(text) = send_text.clone() {
                                    this.send_cmd_to_active(&text, cx);
                                }
                            }
                            cx.notify();
                        }
                    }),
                )
                .child(
                    div()
                        .w(px(14.))
                        .flex()
                        .justify_center()
                        .text_color(t.text_3)
                        .child(if is_folder {
                            Icon::new(if expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .size(px(13.))
                            .into_any_element()
                        } else {
                            div().into_any_element()
                        }),
                )
                .child(
                    div().flex_shrink_0().text_color(t.text_2).child(
                        Icon::new(if is_folder {
                            if expanded {
                                IconName::FolderOpen
                            } else {
                                IconName::FolderClosed
                            }
                        } else {
                            IconName::Keyboard
                        })
                        .size(px(14.)),
                    ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(12.5))
                        .text_color(t.text)
                        .child(label.clone()),
                );

            let menu_entity = entity.clone();
            let menu_id = id.clone();
            let menu_name = label;
            let menu_text = send_text.unwrap_or_default();
            let menu_parent = parent;
            list = list.child(row_element.context_menu(move |menu, _, _| {
                let menu_parent = menu_parent.clone();
                let paste = {
                    let e = menu_entity.clone();
                    let dest = menu_parent.clone();
                    PopupMenuItem::new(tr(lang, "粘贴", "Paste"))
                        .disabled(!has_clip)
                        .on_click(move |_, _, cx| {
                            let dest = dest.clone();
                            e.update(cx, |this, cx| this.cmd_paste(dest, cx));
                        })
                };
                let rename = {
                    let e = menu_entity.clone();
                    let id = menu_id.clone();
                    let current = menu_name.clone();
                    PopupMenuItem::new(tr(lang, "重命名", "Rename")).on_click(
                        move |_, window, cx| {
                            let target = e.clone();
                            let id = id.clone();
                            open_text_prompt(
                                window,
                                cx,
                                lang,
                                tr(lang, "重命名", "Rename"),
                                &current,
                                tr(lang, "名称", "Name").into(),
                                move |name, _, cx| {
                                    target.update(cx, |this, cx| {
                                        if let Some(node) = this.cmd_node_mut(&id) {
                                            match node {
                                                CmdNode::Folder { name: n, .. } => {
                                                    *n = name.clone()
                                                }
                                                CmdNode::Command { name: n, .. } => {
                                                    *n = name.clone()
                                                }
                                            }
                                        }
                                        this.cmd_persist(cx);
                                    });
                                },
                            );
                        },
                    )
                };
                let delete = {
                    let e = menu_entity.clone();
                    let id = menu_id.clone();
                    let (title, message) = if is_folder {
                        (
                            tr(lang, "删除分组", "Delete Folder"),
                            tr(
                                lang,
                                "分组及其中所有命令都会被删除。",
                                "The folder and every command inside will be removed.",
                            ),
                        )
                    } else {
                        (
                            tr(lang, "删除命令", "Delete Command"),
                            tr(lang, "该命令将被删除。", "This command will be removed."),
                        )
                    };
                    PopupMenuItem::new(tr(lang, "删除", "Delete")).on_click(move |_, window, cx| {
                        let target = e.clone();
                        let id = id.clone();
                        let title = title.clone();
                        let message = message.clone();
                        confirm_dialog(window, cx, lang, title, message, move |_, cx| {
                            target.update(cx, |this, cx| {
                                this.cmd_remove(&id);
                                if this.cmd_selected.as_deref() == Some(id.as_str()) {
                                    this.cmd_selected = None;
                                }
                                this.cmd_persist(cx);
                            });
                        });
                    })
                };

                if is_folder {
                    let new_cmd = {
                        let e = menu_entity.clone();
                        let parent = menu_parent.clone();
                        PopupMenuItem::new(tr(lang, "新建命令", "New command")).on_click(
                            move |_, window, cx| {
                                let target = e.clone();
                                let parent = parent.clone();
                                open_command_form(
                                    window,
                                    cx,
                                    lang,
                                    tr(lang, "新建命令", "New Command"),
                                    None,
                                    true,
                                    move |fields, _, cx| {
                                        target.update(cx, |this, cx| {
                                            this.cmd_apply_fields(None, parent.clone(), fields, cx)
                                        });
                                    },
                                );
                            },
                        )
                    };
                    let new_folder = {
                        let e = menu_entity.clone();
                        let parent = menu_parent.clone();
                        PopupMenuItem::new(tr(lang, "新建分组", "New folder")).on_click(
                            move |_, window, cx| {
                                let target = e.clone();
                                let parent = parent.clone();
                                open_text_prompt(
                                    window,
                                    cx,
                                    lang,
                                    tr(lang, "新建分组", "New Folder"),
                                    "",
                                    tr(lang, "分组名称", "Folder name").into(),
                                    move |name, _, cx| {
                                        let node = CmdNode::Folder {
                                            id: new_id("cf"),
                                            name,
                                            expanded: true,
                                            children: Vec::new(),
                                        };
                                        target.update(cx, |this, cx| {
                                            this.cmd_insert(parent.clone(), node);
                                            this.cmd_persist(cx);
                                        });
                                    },
                                );
                            },
                        )
                    };
                    menu.separator()
                        .item(new_cmd)
                        .item(new_folder)
                        .separator()
                        .item(paste)
                        .separator()
                        .item(rename)
                        .separator()
                        .item(delete)
                } else {
                    let send = {
                        let e = menu_entity.clone();
                        let text = menu_text.clone();
                        PopupMenuItem::new(tr(lang, "发送到当前会话", "Send to active session"))
                            .disabled(!can_send)
                            .on_click(move |_, _, cx| {
                                let text = text.clone();
                                e.update(cx, |this, cx| this.send_cmd_to_active(&text, cx));
                            })
                    };
                    let broadcast = {
                        let e = menu_entity.clone();
                        let text = menu_text.clone();
                        PopupMenuItem::new(tr(
                            lang,
                            "发送到全部就绪会话",
                            "Send to all ready sessions",
                        ))
                        .disabled(!any_ready)
                        .on_click(move |_, _, cx| {
                            let text = text.clone();
                            e.update(cx, |this, cx| this.broadcast_cmd(&text, cx));
                        })
                    };
                    let cut = {
                        let e = menu_entity.clone();
                        let id = menu_id.clone();
                        PopupMenuItem::new(tr(lang, "剪切", "Cut")).on_click(move |_, _, cx| {
                            let id = id.clone();
                            e.update(cx, |this, _| {
                                if let Some(node) = this.cmd_node(&id).cloned() {
                                    this.cmd_clip = Some(CmdClip { node, cut: true });
                                }
                            });
                        })
                    };
                    let copy = {
                        let e = menu_entity.clone();
                        let id = menu_id.clone();
                        PopupMenuItem::new(tr(lang, "复制", "Copy")).on_click(move |_, _, cx| {
                            let id = id.clone();
                            e.update(cx, |this, _| {
                                if let Some(node) = this.cmd_node(&id).cloned() {
                                    this.cmd_clip = Some(CmdClip { node, cut: false });
                                }
                            });
                        })
                    };
                    let props = {
                        let e = menu_entity.clone();
                        let id = menu_id.clone();
                        let name = menu_name.clone();
                        let text = menu_text.clone();
                        PopupMenuItem::new(tr(lang, "属性", "Properties")).on_click(
                            move |_, window, cx| {
                                let target = e.clone();
                                let id = id.clone();
                                open_command_form(
                                    window,
                                    cx,
                                    lang,
                                    tr(lang, "命令属性", "Command Properties"),
                                    Some((name.clone(), text.clone())),
                                    true,
                                    move |fields, _, cx| {
                                        target.update(cx, |this, cx| {
                                            this.cmd_apply_fields(
                                                Some(id.clone()),
                                                None,
                                                fields,
                                                cx,
                                            )
                                        });
                                    },
                                );
                            },
                        )
                    };
                    menu.item(send)
                        .item(broadcast)
                        .separator()
                        .item(cut)
                        .item(copy)
                        .item(paste)
                        .separator()
                        .item(rename)
                        .item(props)
                        .separator()
                        .item(delete)
                }
            }));
        }

        if self.commands.commands.is_empty() {
            list = list.child(
                div()
                    .px(px(16.))
                    .py(px(20.))
                    .text_size(px(12.))
                    .text_color(t.text_3)
                    .child(tr(lang, "还没有命令。右键新建命令。", "No commands yet.")),
            );
        }

        let empty_entity = entity.clone();
        let list = list.context_menu(move |menu, _, _| {
            let new_cmd = {
                let e = empty_entity.clone();
                PopupMenuItem::new(tr(lang, "新建命令", "New command")).on_click(
                    move |_, window, cx| {
                        let target = e.clone();
                        open_command_form(
                            window,
                            cx,
                            lang,
                            tr(lang, "新建命令", "New Command"),
                            None,
                            true,
                            move |fields, _, cx| {
                                target.update(cx, |this, cx| {
                                    this.cmd_apply_fields(None, None, fields, cx)
                                });
                            },
                        );
                    },
                )
            };
            let new_folder = {
                let e = empty_entity.clone();
                PopupMenuItem::new(tr(lang, "新建分组", "New folder")).on_click(
                    move |_, window, cx| {
                        let target = e.clone();
                        open_text_prompt(
                            window,
                            cx,
                            lang,
                            tr(lang, "新建分组", "New Folder"),
                            "",
                            tr(lang, "分组名称", "Folder name").into(),
                            move |name, _, cx| {
                                let node = CmdNode::Folder {
                                    id: new_id("cf"),
                                    name,
                                    expanded: true,
                                    children: Vec::new(),
                                };
                                target.update(cx, |this, cx| {
                                    this.cmd_insert(None, node);
                                    this.cmd_persist(cx);
                                });
                            },
                        );
                    },
                )
            };
            let paste = {
                let e = empty_entity.clone();
                PopupMenuItem::new(tr(lang, "粘贴", "Paste"))
                    .disabled(!has_clip)
                    .on_click(move |_, _, cx| {
                        let e = e.clone();
                        e.update(cx, |this, cx| this.cmd_paste(None, cx));
                    })
            };
            menu.item(new_cmd).item(new_folder).separator().item(paste)
        });

        div().flex().flex_col().flex_1().min_h(px(0.)).child(
            div()
                .id("cmd-scroll")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .child(list),
        )
    }

    fn session_pane(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme.clone();
        let lang = self.lang;
        let entity = cx.entity();
        let needle = self.filter.read(cx).value().trim().to_lowercase();
        let has_clip = self.sess_clip.is_some();
        let mut list = div().id("session-list").flex().flex_col().py(px(2.));

        for row in flatten(&self.tree.sessions, 0, &needle) {
            let theme = t.clone();
            let selected = self.selected.as_deref() == Some(row.id.as_str());
            let bg = if selected { t.bg_active } else { t.bg };
            let id = row.id.clone();
            let is_folder = row.folder;
            let expanded = row.expanded;
            let label = row.name.clone();
            let depth = row.depth;

            let row_element = div()
                .id(SharedString::from(format!("row-{}", id)))
                .h(px(22.))
                .flex()
                .items_center()
                .gap(px(4.))
                .pl(px(6. + depth as f32 * 16.))
                .pr(px(6.))
                .rounded(px(6.))
                .bg(bg)
                .hover(move |s| s.bg(theme.bg_hover))
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener({
                        let id = id.clone();
                        move |this, ev: &gpui::MouseDownEvent, window, cx| {
                            this.selected = Some(id.clone());
                            this.active_panel = PanelId::Sessions;
                            if is_folder {
                                this.toggle_folder(&id, cx);
                            } else if ev.click_count >= 2 {
                                this.connect_session(&id, window, cx);
                            }
                            cx.notify();
                        }
                    }),
                )
                .child(
                    div()
                        .w(px(14.))
                        .flex()
                        .justify_center()
                        .text_color(t.text_3)
                        .child(if is_folder {
                            Icon::new(if expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .size(px(14.))
                            .into_any_element()
                        } else {
                            div().into_any_element()
                        }),
                )
                .child(
                    div().flex_shrink_0().text_color(t.text_2).child(
                        Icon::new(if is_folder {
                            if expanded {
                                IconName::FolderOpen
                            } else {
                                IconName::FolderClosed
                            }
                        } else {
                            IconName::Monitor
                        })
                        .size(px(15.)),
                    ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(13.))
                        .text_color(t.text)
                        .child(label.clone()),
                );

            let menu_entity = entity.clone();
            let menu_id = id.clone();
            let menu_name = label;
            list = list.child(row_element.context_menu(move |menu, _, _| {
                if is_folder {
                    let new_session = {
                        let e = menu_entity.clone();
                        let parent = menu_id.clone();
                        PopupMenuItem::new(tr(lang, "新建会话", "New session")).on_click(
                            move |_, window, cx| {
                                let target = e.clone();
                                let parent = parent.clone();
                                open_session_form(
                                    window,
                                    cx,
                                    lang,
                                    None,
                                    Some(parent),
                                    move |draft, _, cx| {
                                        target.update(cx, |this, cx| this.apply_draft(draft, cx));
                                    },
                                );
                            },
                        )
                    };
                    let new_folder = {
                        let e = menu_entity.clone();
                        let parent = menu_id.clone();
                        PopupMenuItem::new(tr(lang, "新建分组", "New folder")).on_click(
                            move |_, window, cx| {
                                let target = e.clone();
                                let parent = parent.clone();
                                open_text_prompt(
                                    window,
                                    cx,
                                    lang,
                                    tr(lang, "新建分组", "New Folder"),
                                    "",
                                    tr(lang, "分组名称", "Folder name").into(),
                                    move |name, _, cx| {
                                        target.update(cx, |this, cx| {
                                            this.add_folder(Some(parent.clone()), name, cx)
                                        });
                                    },
                                );
                            },
                        )
                    };
                    let rename = {
                        let e = menu_entity.clone();
                        let id = menu_id.clone();
                        let current = menu_name.clone();
                        PopupMenuItem::new(tr(lang, "重命名", "Rename")).on_click(
                            move |_, window, cx| {
                                let target = e.clone();
                                let id = id.clone();
                                open_text_prompt(
                                    window,
                                    cx,
                                    lang,
                                    tr(lang, "重命名", "Rename"),
                                    &current,
                                    tr(lang, "名称", "Name").into(),
                                    move |name, _, cx| {
                                        target.update(cx, |this, _| this.rename_node(&id, name));
                                    },
                                );
                            },
                        )
                    };
                    let delete = {
                        let e = menu_entity.clone();
                        let id = menu_id.clone();
                        PopupMenuItem::new(tr(lang, "删除分组", "Delete folder")).on_click(
                            move |_, window, cx| {
                                let target = e.clone();
                                let id = id.clone();
                                confirm_dialog(
                                    window,
                                    cx,
                                    lang,
                                    tr(lang, "删除分组", "Delete Folder"),
                                    tr(
                                        lang,
                                        "分组及其中所有会话都会被删除。",
                                        "The folder and every session inside will be removed.",
                                    ),
                                    move |_, cx| {
                                        target.update(cx, |this, cx| {
                                            this.remove_node(&id);
                                            this.persist(cx);
                                        });
                                    },
                                );
                            },
                        )
                    };
                    let cut = {
                        let e = menu_entity.clone();
                        let id = menu_id.clone();
                        PopupMenuItem::new(tr(lang, "剪切", "Cut")).on_click(move |_, _, cx| {
                            let id = id.clone();
                            e.update(cx, |this, cx| this.cut_session(&id, cx));
                        })
                    };
                    let copy = {
                        let e = menu_entity.clone();
                        let id = menu_id.clone();
                        PopupMenuItem::new(tr(lang, "复制", "Copy")).on_click(move |_, _, cx| {
                            let id = id.clone();
                            e.update(cx, |this, cx| this.copy_session(&id, cx));
                        })
                    };
                    let paste = {
                        let e = menu_entity.clone();
                        let id = menu_id.clone();
                        PopupMenuItem::new(tr(lang, "粘贴到此分组", "Paste into folder"))
                            .disabled(!has_clip)
                            .on_click(move |_, _, cx| {
                                let dest = id.clone();
                                e.update(cx, |this, cx| this.sess_paste(Some(dest), cx));
                            })
                    };
                    menu.separator()
                        .item(new_session)
                        .item(new_folder)
                        .separator()
                        .item(cut)
                        .item(copy)
                        .item(paste)
                        .separator()
                        .item(rename)
                        .separator()
                        .item(delete)
                } else {
                    let open = {
                        let e = menu_entity.clone();
                        let id = menu_id.clone();
                        PopupMenuItem::new(tr(lang, "打开", "Open")).on_click(
                            move |_, window, cx| {
                                let id = id.clone();
                                e.update(cx, |this, cx| this.connect_session(&id, window, cx));
                            },
                        )
                    };
                    let edit = {
                        let e = menu_entity.clone();
                        let id = menu_id.clone();
                        PopupMenuItem::new(tr(lang, "编辑", "Edit")).on_click(
                            move |_, window, cx| {
                                let Some(session) = e.read(cx).session(&id).cloned() else {
                                    return;
                                };
                                let target = e.clone();
                                open_session_form(
                                    window,
                                    cx,
                                    lang,
                                    Some(session),
                                    None,
                                    move |draft, _, cx| {
                                        target.update(cx, |this, cx| this.apply_draft(draft, cx));
                                    },
                                );
                            },
                        )
                    };
                    let duplicate = {
                        let e = menu_entity.clone();
                        let id = menu_id.clone();
                        PopupMenuItem::new(tr(lang, "创建副本", "Duplicate")).on_click(
                            move |_, _, cx| {
                                let id = id.clone();
                                e.update(cx, |this, cx| this.duplicate_session(&id, cx));
                            },
                        )
                    };
                    let delete = {
                        let e = menu_entity.clone();
                        let id = menu_id.clone();
                        PopupMenuItem::new(tr(lang, "删除", "Delete")).on_click(
                            move |_, window, cx| {
                                let target = e.clone();
                                let id = id.clone();
                                confirm_dialog(
                                    window,
                                    cx,
                                    lang,
                                    tr(lang, "删除会话", "Delete Session"),
                                    tr(
                                        lang,
                                        "该会话配置将被删除。",
                                        "This session will be removed.",
                                    ),
                                    move |_, cx| {
                                        target.update(cx, |this, cx| {
                                            this.remove_node(&id);
                                            this.persist(cx);
                                        });
                                    },
                                );
                            },
                        )
                    };
                    let cut = {
                        let e = menu_entity.clone();
                        let id = menu_id.clone();
                        PopupMenuItem::new(tr(lang, "剪切", "Cut")).on_click(move |_, _, cx| {
                            let id = id.clone();
                            e.update(cx, |this, cx| this.cut_session(&id, cx));
                        })
                    };
                    let copy = {
                        let e = menu_entity.clone();
                        let id = menu_id.clone();
                        PopupMenuItem::new(tr(lang, "复制", "Copy")).on_click(move |_, _, cx| {
                            let id = id.clone();
                            e.update(cx, |this, cx| this.copy_session(&id, cx));
                        })
                    };
                    let paste = {
                        let e = menu_entity.clone();
                        let id = menu_id.clone();
                        PopupMenuItem::new(tr(lang, "粘贴到同级", "Paste beside"))
                            .disabled(!has_clip)
                            .on_click(move |_, _, cx| {
                                let id = id.clone();
                                e.update(cx, |this, cx| {
                                    let dest = this.parent_of(&id);
                                    this.sess_paste(dest, cx);
                                });
                            })
                    };
                    menu.separator()
                        .item(open)
                        .item(edit)
                        .item(duplicate)
                        .separator()
                        .item(cut)
                        .item(copy)
                        .item(paste)
                        .separator()
                        .item(delete)
                }
            }));
        }

        if self.tree.sessions.is_empty() {
            list = list.child(
                div()
                    .px(px(16.))
                    .py(px(20.))
                    .text_size(px(12.))
                    .text_color(t.text_3)
                    .child(tr(
                        lang,
                        "还没有会话。右键导入或新建会话。",
                        "No sessions yet.",
                    )),
            );
        }

        let empty_entity = entity.clone();
        let list = list.context_menu(move |menu, _, _| {
            let new_session = {
                let e = empty_entity.clone();
                PopupMenuItem::new(tr(lang, "新建会话", "New session")).on_click(
                    move |_, window, cx| {
                        let target = e.clone();
                        open_session_form(window, cx, lang, None, None, move |draft, _, cx| {
                            target.update(cx, |this, cx| this.apply_draft(draft, cx));
                        });
                    },
                )
            };
            let new_folder = {
                let e = empty_entity.clone();
                PopupMenuItem::new(tr(lang, "新建分组", "New folder")).on_click(
                    move |_, window, cx| {
                        let target = e.clone();
                        open_text_prompt(
                            window,
                            cx,
                            lang,
                            tr(lang, "新建分组", "New Folder"),
                            "",
                            tr(lang, "分组名称", "Folder name").into(),
                            move |name, _, cx| {
                                target.update(cx, |this, cx| this.add_folder(None, name, cx));
                            },
                        );
                    },
                )
            };
            let paste = {
                let e = empty_entity.clone();
                PopupMenuItem::new(tr(lang, "粘贴到根目录", "Paste at root"))
                    .disabled(!has_clip)
                    .on_click(move |_, _, cx| {
                        let target = e.clone();
                        target.update(cx, |this, cx| this.sess_paste(None, cx));
                    })
            };
            menu.item(new_session)
                .item(new_folder)
                .separator()
                .item(paste)
        });

        div().flex().flex_col().flex_1().min_h(px(0.)).child(
            div()
                .id("session-scroll")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .child(list),
        )
    }

    fn is_folder(&self, id: &str) -> bool {
        fn walk(nodes: &[Node], id: &str) -> bool {
            for n in nodes {
                if let Node::Folder {
                    id: fid, children, ..
                } = n
                {
                    if fid == id || walk(children, id) {
                        return true;
                    }
                }
            }
            false
        }
        walk(&self.tree.sessions, id)
    }

    fn tab_strip(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme.clone();
        let lang = self.lang;
        let mut strip = div()
            .h(px(34.))
            .flex_shrink_0()
            .flex()
            .items_end()
            .gap(px(1.))
            .px(px(6.))
            .bg(t.bg)
            .overflow_hidden();

        if self.tabs.is_empty() {
            return strip.child(div().flex_1()).child(tab_strip_button(&t, cx));
        }

        for (i, tab) in self.tabs.iter().enumerate() {
            let view = tab.view.clone();
            let title = view.read(cx).title.clone();
            let state = view.read(cx).state.clone();
            let active = self.active == Some(i);
            let theme = t.clone();
            let dot = match state {
                ConnState::Ready => t.accent,
                ConnState::Connecting => t.text_3,
                ConnState::Closed(_) => t.danger,
            };
            let tab_bg = if active { t.bg_content } else { t.bg };
            let menu_entity = cx.entity();
            let menu_title = title.clone();
            let strip_tab = div()
                .id(SharedString::from(format!("tab-{}", i)))
                .h(px(28.))
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(10.))
                .rounded_t_md()
                .bg(tab_bg)
                .text_size(px(12.))
                .text_color(t.text)
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &gpui::MouseDownEvent, window, cx| {
                        this.active = Some(i);
                        this.tabs[i]
                            .view
                            .update(cx, |v, cx| v.focus_terminal(window, cx));
                        this.sync_panels(cx);
                        cx.notify();
                    }),
                )
                .context_menu(move |menu, _, _| {
                    let lang = lang;
                    let e = menu_entity.clone();
                    let rename = {
                        let e = e.clone();
                        let title = menu_title.clone();
                        PopupMenuItem::new(tr(lang, "重命名", "Rename")).on_click(
                            move |_, window, cx| {
                                let target = e.clone();
                                open_text_prompt(
                                    window,
                                    cx,
                                    lang,
                                    tr(lang, "重命名标签页", "Rename Tab"),
                                    &title,
                                    tr(lang, "名称", "Name").into(),
                                    move |name, _, cx| {
                                        target.update(cx, |this, cx| this.rename_tab(i, name, cx));
                                    },
                                );
                            },
                        )
                    };
                    let reconnect = {
                        let e = e.clone();
                        PopupMenuItem::new(tr(lang, "重连", "Reconnect")).on_click(
                            move |_, window, cx| {
                                let target = e.clone();
                                target.update(cx, |this, cx| this.reconnect_tab(i, window, cx));
                            },
                        )
                    };
                    let close = {
                        let e = e.clone();
                        PopupMenuItem::new(tr(lang, "关闭", "Close")).on_click(move |_, _, cx| {
                            let target = e.clone();
                            target.update(cx, |this, cx| this.close_tab(i, cx));
                        })
                    };
                    let close_others = {
                        let e = e.clone();
                        PopupMenuItem::new(tr(lang, "关闭其他", "Close Others")).on_click(
                            move |_, _, cx| {
                                let target = e.clone();
                                target.update(cx, |this, cx| {
                                    let others = (0..this.tabs.len()).filter(|n| *n != i).collect();
                                    this.close_many(others, cx);
                                });
                            },
                        )
                    };
                    let close_all = {
                        let e = e.clone();
                        PopupMenuItem::new(tr(lang, "全部关闭", "Close All")).on_click(
                            move |_, _, cx| {
                                let target = e.clone();
                                target.update(cx, |this, cx| {
                                    let all = (0..this.tabs.len()).collect();
                                    this.close_many(all, cx);
                                });
                            },
                        )
                    };
                    menu.item(rename)
                        .item(reconnect)
                        .separator()
                        .item(close)
                        .item(close_others)
                        .item(close_all)
                })
                .child(div().size(px(6.)).rounded_full().bg(dot))
                .child(
                    div()
                        .max_w(px(180.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(title),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("tab-close-{}", i)))
                        .w(px(16.))
                        .h(px(16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_sm()
                        .text_size(px(11.))
                        .text_color(t.text_3)
                        .hover(move |s| s.bg(theme.bg_hover).text_color(theme.text))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &gpui::MouseDownEvent, _window, cx| {
                                cx.stop_propagation();
                                this.close_tab(i, cx);
                            }),
                        )
                        .child("×"),
                );
            strip = strip.child(strip_tab);
        }
        strip.child(div().flex_1()).child(tab_strip_button(&t, cx))
    }

    fn body(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme.clone();
        let lang = self.lang;
        if let Some(active) = self.active {
            if let Some(tab) = self.tabs.get(active) {
                return div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .bg(t.bg_content)
                    .child(tab.view.clone());
            }
        }
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .bg(t.bg)
            .child(
                div()
                    .size(px(44.))
                    .mb(px(14.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(t.border)
                    .bg(t.bg_hover)
                    .text_color(t.text_2)
                    .child(Icon::new(IconName::SquareTerminal).size(px(22.))),
            )
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(t.text)
                    .child(tr(lang, "暂无终端会话", "No terminal sessions")),
            )
            .child(
                div()
                    .mt(px(3.))
                    .mb(px(16.))
                    .text_size(px(12.))
                    .text_color(t.text_3)
                    .child(tr(
                        lang,
                        "双击左侧会话即可连接",
                        "Double-click a session on the left to connect",
                    )),
            )
            .child(
                div()
                    .id("empty-new")
                    .h(px(29.))
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded(px(6.))
                    .bg(t.accent)
                    .text_color(gpui::rgb(0x0d1b10))
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .cursor_pointer()
                    .hover(move |s| s.bg(t.accent_hover))
                    .on_click(cx.listener(|this, _, window, cx| this.new_session(window, cx)))
                    .child(Icon::new(IconName::Monitor).size(px(15.)))
                    .child(tr(lang, "新建连接", "New Connection")),
            )
    }

    fn status_bar(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme.clone();
        let th = t.clone();
        let lang = self.lang;
        let active_tab = self.active.and_then(|i| self.tabs.get(i));
        let (state_text, state_color, active) = match active_tab {
            Some(tab) => {
                let state = tab.view.read(cx).state.clone();
                let color = match state {
                    ConnState::Ready => t.accent,
                    ConnState::Connecting => t.dot_warn,
                    ConnState::Closed(_) => t.dot_error,
                };
                let text = match &state {
                    ConnState::Ready => tr(lang, "已连接", "Connected"),
                    ConnState::Connecting => tr(lang, "连接中…", "Connecting…"),
                    ConnState::Closed(reason) if reason.is_empty() => {
                        tr(lang, "已断开", "Disconnected")
                    }
                    ConnState::Closed(reason) => SharedString::from(reason.clone()),
                };
                (text, color, true)
            }
            None => (tr(lang, "无活动会话", "No active session"), t.text_3, false),
        };

        let active_closed = matches!(
            active_tab,
            Some(tab) if matches!(tab.view.read(cx).state, ConnState::Closed(_))
        );
        let mut bar = div()
            .h(px(24.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(10.))
            .bg(t.bg)
            .border_t_1()
            .border_color(t.line)
            .text_size(px(11.5))
            .text_color(t.text_2);
        if active {
            if let Some(tab) = active_tab {
                let view = tab.view.read(cx);
                let title = view.title.clone();
                let spec = view.spec.clone();
                let host =
                    SharedString::from(format!("{}@{}:{}", spec.username, spec.host, spec.port));
                bar = bar
                    .child(div().size(px(6.)).rounded_full().bg(state_color))
                    .child(div().text_color(t.text).child(title))
                    .child(div().text_color(t.text_3).child(host));
            }
            bar = bar.child(
                div()
                    .ml(px(6.))
                    .text_color(t.text_3)
                    .child(state_text.clone()),
            );
        } else {
            bar = bar.child(div().text_color(t.text_3).child(state_text.clone()));
        }
        if active_closed {
            bar = bar.child(
                div()
                    .id("status-reconnect")
                    .px(px(8.))
                    .h(px(18.))
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .text_color(t.accent)
                    .cursor_pointer()
                    .hover(move |s| s.bg(th.bg_hover))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(index) = this.active {
                            this.reconnect_tab(index, window, cx);
                        }
                    }))
                    .child(tr(lang, "重连", "Reconnect")),
            );
        }
        if let Some(error) = self.error.clone() {
            bar = bar.child(
                div()
                    .text_color(t.danger)
                    .max_w(px(520.))
                    .overflow_hidden()
                    .child(error),
            );
        }
        bar = bar.child(div().flex_1());
        let (cols, rows) = match active_tab {
            Some(tab) => tab.view.read(cx).grid_size(),
            None => (0, 0),
        };
        if active {
            let cursor = active_tab.and_then(|tab| tab.view.read(cx).cursor_pos());
            let pos = match cursor {
                Some((row, col)) => format!("{}, {}", row + 1, col + 1),
                None => "--, --".to_string(),
            };
            let size_text = if lang == Lang::Zh {
                format!("{rows} 行 × {cols} 列")
            } else {
                format!("{rows} Rows, {cols} Cols")
            };
            bar = bar
                .child(div().text_color(t.text_3).child(SharedString::from(pos)))
                .child(status_sep(&t))
                .child(
                    div()
                        .text_color(t.text_3)
                        .child(SharedString::from(size_text)),
                )
                .child(status_sep(&t))
                .child(div().text_color(t.text_3).child("Xterm"));
            #[cfg(windows)]
            {
                let (caps, num) = lock_keys();
                bar = bar.child(status_sep(&t)).child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .child(lock_label("CAP", caps, &t))
                        .child(lock_label("NUM", num, &t)),
                );
            }
            bar = bar.child(status_sep(&t));
        }
        bar = bar.child(div().text_color(t.text_2).child(SharedString::from(format!(
            "{} {}",
            self.tabs.len(),
            tr(lang, "个标签", "tabs")
        ))));
        bar = bar.child(status_sep(&t));
        let th = t.clone();
        bar = bar.child(
            div()
                .id("status-zoom-reset")
                .px(px(4.))
                .h(px(18.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .text_color(t.text_3)
                .cursor_pointer()
                .hover(move |s| s.bg(th.bg_hover).text_color(th.text))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.set_font_size(DEFAULT_MONO, window, cx);
                }))
                .child(SharedString::from(format!("{:.0}px", self.mono_size))),
        );
        let th = t.clone();
        let (mcp_dot, mcp_text, mcp_port) = if self.mcp.running {
            (t.accent, SharedString::from("MCP"), Some(self.mcp.port))
        } else if self.mcp.enabled {
            (t.dot_warn, SharedString::from("MCP"), None)
        } else {
            (t.text_3, tr(lang, "MCP 未启用", "MCP off"), None)
        };
        bar = bar.child(status_sep(&t)).child(
            div()
                .id("status-mcp")
                .flex()
                .items_center()
                .gap(px(5.))
                .px(px(6.))
                .mr(px(-6.))
                .h(px(20.))
                .rounded(px(6.))
                .text_color(t.text_2)
                .cursor_pointer()
                .hover(move |s| s.bg(th.bg_hover).text_color(th.text))
                .on_click(cx.listener(|this, _, window, cx| {
                    open_mcp_dialog(window, cx, this.backend.clone(), this.lang);
                }))
                .child(div().size(px(6.)).rounded_full().bg(mcp_dot))
                .child(mcp_text)
                .children(mcp_port.map(|port| {
                    div()
                        .text_color(t.text_3)
                        .child(SharedString::from(format!(":{port}")))
                })),
        );
        bar
    }
}

impl Render for OneCrt {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme.clone();
        // SecureCRT 的做法：标题栏直接带上会话名与连接状态
        let title = match self.active.and_then(|i| self.tabs.get(i)) {
            Some(tab) => {
                let view = tab.view.read(cx);
                let state = match &view.state {
                    ConnState::Ready => tr(self.lang, "已连接", "Connected"),
                    ConnState::Connecting => tr(self.lang, "连接中", "Connecting"),
                    ConnState::Closed(_) => tr(self.lang, "已断开", "Disconnected"),
                };
                format!("{} - {} - OneCRT", view.title, state)
            }
            None => "OneCRT".to_string(),
        };
        if self.window_title != title {
            window.set_window_title(&title);
            self.window_title = title;
        }
        if !cx.has_active_drag() {
            self.guide = None;
            self.guide_area = None;
        }
        let left = self.groups[DockZone::Left.index()].clone();
        let right = self.groups[DockZone::Right.index()].clone();
        let bottom = self.groups[DockZone::Bottom.index()].clone();
        let top = self.groups[DockZone::Top.index()].clone();
        let left_open = !left.panels.is_empty() && !left.collapsed;
        let right_open = !right.panels.is_empty() && !right.collapsed;
        let top_open = !top.panels.is_empty() && !top.collapsed;
        let bottom_open = !bottom.panels.is_empty() && !bottom.collapsed;

        let mut main = div()
            .id("dock-main")
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .on_drag_move::<DockDrag>(cx.listener(
                |this, ev: &gpui::DragMoveEvent<DockDrag>, _window, cx| {
                    let rel = point(
                        ev.event.position.x - ev.bounds.origin.x,
                        ev.event.position.y - ev.bounds.origin.y,
                    );
                    let target = this.target_at(rel, ev.bounds.size);
                    this.guide = Some(target);
                    this.guide_area = Some((
                        f32::from(ev.bounds.size.width),
                        f32::from(ev.bounds.size.height),
                    ));
                    cx.notify();
                },
            ))
            .on_drop::<DockDrag>(cx.listener(|this, drag: &DockDrag, window, cx| {
                this.drop_panel(drag.panel, window, cx);
            }));
        if !left.panels.is_empty() {
            main = main.child(if left.collapsed {
                self.dock_rail(DockZone::Left, cx)
            } else {
                self.dock_group(DockZone::Left, cx)
            });
        }

        let mut center = div()
            .id("dock-center")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col();
        if !top.panels.is_empty() {
            center = center
                .on_drag_move::<TopDivider>(cx.listener(
                    |this, ev: &gpui::DragMoveEvent<TopDivider>, _window, cx| {
                        let origin = f32::from(ev.bounds.origin.y);
                        let tall = f32::from(ev.bounds.size.height);
                        let y = f32::from(ev.event.position.y);
                        this.top_h = (y - origin).clamp(120., (tall - 120.).max(120.));
                        cx.notify();
                    },
                ))
                .child(if top.collapsed {
                    self.dock_rail(DockZone::Top, cx)
                } else {
                    self.dock_group(DockZone::Top, cx)
                });
        }
        center = center.child(self.tab_strip(cx)).child(self.body(cx));
        if !bottom.panels.is_empty() {
            center = center
                .on_drag_move::<VDivider>(cx.listener(
                    |this, ev: &gpui::DragMoveEvent<VDivider>, _window, cx| {
                        let bottom = f32::from(ev.bounds.origin.y + ev.bounds.size.height);
                        let tall = f32::from(ev.bounds.size.height);
                        let y = f32::from(ev.event.position.y);
                        this.bottom_h = (bottom - y).clamp(120., (tall - 120.).max(120.));
                        cx.notify();
                    },
                ))
                .child(if bottom.collapsed {
                    self.dock_rail(DockZone::Bottom, cx)
                } else {
                    self.dock_group(DockZone::Bottom, cx)
                });
        }
        main = main.child(center);

        if right_open {
            main = main.child(self.dock_group(DockZone::Right, cx));
        } else if !right.panels.is_empty() {
            main = main.child(self.dock_rail(DockZone::Right, cx));
        }

        let (inset_l, inset_r, inset_t, inset_b) = self.dock_insets();
        // 分隔条只做拖拽热区，绝对定位压在面板边界上，不占布局空间，避免面板之间露出底色缝隙。
        if left_open {
            main = main.child(
                split_handle("split-left", SplitEdge::Left).left(px(self.left_w - 3.)),
            );
        }
        if right_open {
            main = main.child(
                split_handle("split-right", SplitEdge::Right).right(px(self.right_w - 3.)),
            );
        }
        if top_open {
            main = main.child(
                v_split_handle("split-top", TopDivider)
                    .left(px(inset_l))
                    .right(px(inset_r))
                    .top(px(self.top_h - 3.)),
            );
        }
        if bottom_open {
            main = main.child(
                v_split_handle("split-bottom", VDivider)
                    .left(px(inset_l))
                    .right(px(inset_r))
                    .bottom(px(self.bottom_h - 3.)),
            );
        }
        if self.peek.map(|p| p.sticky).unwrap_or(false) {
            main = main.child(
                div()
                    .id("dock-peek-backdrop")
                    .absolute()
                    .left(px(inset_l))
                    .right(px(inset_r))
                    .top(px(inset_t))
                    .bottom(px(inset_b))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.peek_close(cx)),
                    ),
            );
        }
        if let Some(overlay) = self.peek_overlay(&t, cx) {
            main = main.child(overlay);
        }

        if let Some(overlay) = self.guide_overlay(&t) {
            main = main.child(overlay);
        }

        div()
            .id("onecrt-root")
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg_content)
            .font(t.ui_font.clone())
            .text_size(t.ui_size)
            .on_key_down(cx.listener(|_this, ev: &gpui::KeyDownEvent, window, cx| {
                if ev.keystroke.key.eq_ignore_ascii_case("f11") {
                    window.toggle_fullscreen();
                    cx.stop_propagation();
                }
            }))
            .on_drag_move::<SplitEdge>(cx.listener(
                |this, ev: &gpui::DragMoveEvent<SplitEdge>, window, cx| {
                    let edge = *ev.drag(cx);
                    let x = f32::from(ev.event.position.x);
                    let win_w = f32::from(window.bounds().size.width);
                    match edge {
                        SplitEdge::Left => this.left_w = x.clamp(240., 560.),
                        SplitEdge::Right => this.right_w = (win_w - x).clamp(280., 760.),
                    }
                    cx.notify();
                },
            ))
            .child(self.titlebar(window, cx))
            .child(self.icon_toolbar(cx))
            .child(main)
            .child(self.status_bar(cx))
    }
}

fn status_sep(t: &Theme) -> Div {
    div().w(px(1.)).h(px(11.)).bg(t.border)
}

#[cfg(windows)]
fn lock_keys() -> (bool, bool) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CAPITAL, VK_NUMLOCK};
    unsafe {
        (
            GetKeyState(VK_CAPITAL as i32) & 1 != 0,
            GetKeyState(VK_NUMLOCK as i32) & 1 != 0,
        )
    }
}

#[cfg(windows)]
fn lock_label(label: &'static str, on: bool, t: &Theme) -> impl IntoElement {
    div()
        .text_color(if on { t.text } else { t.text_3.opacity(0.5) })
        .child(SharedString::from(label))
}

fn tab_strip_button(t: &Theme, cx: &mut Context<OneCrt>) -> impl IntoElement {
    let th = t.clone();
    div()
        .id("tabs-fullscreen")
        .size(px(24.))
        .mb(px(5.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .text_color(t.text_3)
        .cursor_pointer()
        .hover(move |s| s.bg(th.bg_hover).text_color(th.text))
        .on_click(cx.listener(|_this, _, window, _cx| window.toggle_fullscreen()))
        .child(Icon::new(IconName::Expand).size(px(13.)))
}

fn menu_button(
    id: &'static str,
    label: SharedString,
    t: &Theme,
    build: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
) -> impl IntoElement {
    Button::new(id)
        .ghost()
        .compact()
        .label(label)
        .text_size(px(12.5))
        .text_color(t.text)
        .h(px(26.))
        .px(px(10.))
        .dropdown_menu(build)
}

fn window_control(
    id: &'static str,
    area: WindowControlArea,
    icon: IconName,
    t: &Theme,
    danger: bool,
) -> impl IntoElement {
    let th = t.clone();
    div()
        .id(id)
        .w(px(46.))
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .text_color(t.text_2)
        .cursor_pointer()
        .hover(move |s| {
            if danger {
                s.bg(gpui::rgb(0xc42b1c)).text_color(gpui::rgb(0xffffff))
            } else {
                s.bg(th.bg_hover).text_color(th.text)
            }
        })
        .window_control_area(area)
        .child(Icon::new(icon).size(px(14.)))
}

fn toolbar_icon(
    id: &'static str,
    icon: IconName,
    enabled: bool,
    accent: bool,
    t: &Theme,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let th = t.clone();
    let color = if accent {
        t.accent
    } else if enabled {
        t.text_2
    } else {
        t.text_2.opacity(0.3)
    };
    let mut el = div()
        .id(id)
        .w(px(30.))
        .h(px(26.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .text_color(color);
    if enabled {
        el = el
            .cursor_pointer()
            .hover(move |s| s.bg(th.bg_selected).text_color(th.text))
            .on_click(on_click);
    }
    el.child(Icon::new(icon).size(px(16.)))
}

fn toolbar_divider(t: &Theme) -> Div {
    div().w(px(1.)).h(px(18.)).mx(px(5.)).bg(t.line)
}

fn sidebar_icon(
    id: &'static str,
    icon: IconName,
    enabled: bool,
    t: &Theme,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let th = t.clone();
    let mut el = div()
        .id(id)
        .w(px(26.))
        .h(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .text_color(if enabled {
            th.text_2
        } else {
            th.text_2.opacity(0.3)
        });
    if enabled {
        el = el
            .cursor_pointer()
            .hover(move |s| s.bg(th.bg_selected).text_color(th.text))
            .on_click(on_click);
    }
    el.child(Icon::new(icon).size(px(16.)))
}

/// 停靠区分隔条：绝对定位覆盖在面板边界上，只留拖拽热区，不占布局空间。
fn split_handle(id: &'static str, edge: SplitEdge) -> Stateful<Div> {
    div()
        .id(id)
        .absolute()
        .top_0()
        .bottom_0()
        .w(px(6.))
        .cursor(CursorStyle::ResizeColumn)
        .on_drag(edge, |_, _, _, cx| cx.new(|_| SplitGhost))
}

fn v_split_handle<T>(id: &'static str, divider: T) -> Stateful<Div>
where
    T: 'static,
{
    div()
        .id(id)
        .absolute()
        .left_0()
        .right_0()
        .h(px(6.))
        .cursor(CursorStyle::ResizeRow)
        .on_drag(divider, |_, _, _, cx| cx.new(|_| SplitGhost))
}

fn file_menu(
    menu: PopupMenu,
    app: Entity<OneCrt>,
    lang: Lang,
    _window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let backend = app.read(cx).backend.clone();
    let new_session = {
        let e = app.clone();
        PopupMenuItem::new(tr(lang, "新建会话", "New Session")).on_click(move |_, window, cx| {
            e.update(cx, |this, cx| this.new_session(window, cx));
        })
    };
    let new_folder = {
        let e = app.clone();
        PopupMenuItem::new(tr(lang, "新建分组", "New Folder")).on_click(move |_, window, cx| {
            e.update(cx, |this, cx| this.new_folder(window, cx));
        })
    };
    let import = {
        let e = app.clone();
        PopupMenuItem::new(tr(lang, "导入会话…", "Import Sessions…")).on_click(
            move |_, window, cx| {
                e.update(cx, |this, cx| this.open_import(window, cx));
            },
        )
    };
    let quick = {
        let e = app.clone();
        PopupMenuItem::new(tr(lang, "快速连接…", "Quick Connect…")).on_click(
            move |_, window, cx| {
                e.update(cx, |this, cx| this.quick_connect(window, cx));
            },
        )
    };
    let mcp = PopupMenuItem::new(tr(lang, "MCP 服务…", "MCP Service…"))
        .on_click(move |_, window, cx| open_mcp_dialog(window, cx, backend.clone(), lang));
    let quit = PopupMenuItem::new(tr(lang, "退出", "Quit")).on_click(move |_, _, cx| cx.quit());
    menu.item(new_session)
        .item(new_folder)
        .separator()
        .item(import)
        .item(quick)
        .separator()
        .item(mcp)
        .separator()
        .item(quit)
}

fn edit_menu(
    menu: PopupMenu,
    app: Entity<OneCrt>,
    lang: Lang,
    _window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let active = app.read(cx).active.is_some();
    let copy = {
        let e = app.clone();
        PopupMenuItem::new(tr(lang, "复制", "Copy"))
            .disabled(!active)
            .on_click(move |_, _, cx| {
                e.update(cx, |this, cx| this.copy_active(cx));
            })
    };
    let paste = {
        let e = app.clone();
        PopupMenuItem::new(tr(lang, "粘贴", "Paste"))
            .disabled(!active)
            .on_click(move |_, _, cx| {
                e.update(cx, |this, cx| this.paste_active(cx));
            })
    };
    let clear = {
        let e = app.clone();
        PopupMenuItem::new(tr(lang, "清屏", "Clear Screen"))
            .disabled(!active)
            .on_click(move |_, _, cx| {
                e.update(cx, |this, cx| this.clear_active(cx));
            })
    };
    let interrupt = {
        let e = app.clone();
        PopupMenuItem::new(tr(lang, "发送 Ctrl+C", "Send Ctrl+C"))
            .disabled(!active)
            .on_click(move |_, _, cx| {
                e.update(cx, |this, cx| this.interrupt_active(cx));
            })
    };
    menu.item(copy)
        .item(paste)
        .separator()
        .item(clear)
        .item(interrupt)
}

fn terminal_menu(
    menu: PopupMenu,
    app: Entity<OneCrt>,
    lang: Lang,
    _window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let active = app.read(cx).active.is_some();
    let reconnect = {
        let e = app.clone();
        PopupMenuItem::new(tr(lang, "重连", "Reconnect"))
            .disabled(!active)
            .on_click(move |_, window, cx| {
                e.update(cx, |this, cx| this.reconnect_active(window, cx));
            })
    };
    let disconnect = {
        let e = app.clone();
        PopupMenuItem::new(tr(lang, "断开", "Disconnect"))
            .disabled(!active)
            .on_click(move |_, _, cx| {
                e.update(cx, |this, cx| this.disconnect_active(cx));
            })
    };
    let rename = {
        let e = app.clone();
        PopupMenuItem::new(tr(lang, "重命名标签页…", "Rename Tab…"))
            .disabled(!active)
            .on_click(move |_, window, cx| {
                e.update(cx, |this, cx| this.rename_active_tab(window, cx));
            })
    };
    let close = {
        let e = app.clone();
        PopupMenuItem::new(tr(lang, "关闭标签页", "Close Tab"))
            .disabled(!active)
            .on_click(move |_, _, cx| {
                e.update(cx, |this, cx| this.close_active(cx));
            })
    };
    let zoom_in = {
        let e = app.clone();
        PopupMenuItem::new(tr(lang, "放大字号", "Zoom In")).on_click(move |_, window, cx| {
            e.update(cx, |this, cx| this.zoom_by(1., window, cx));
        })
    };
    let zoom_out = {
        let e = app.clone();
        PopupMenuItem::new(tr(lang, "缩小字号", "Zoom Out")).on_click(move |_, window, cx| {
            e.update(cx, |this, cx| this.zoom_by(-1., window, cx));
        })
    };
    let zoom_reset = {
        let e = app.clone();
        PopupMenuItem::new(tr(lang, "重置字号", "Reset Zoom")).on_click(move |_, window, cx| {
            e.update(cx, |this, cx| this.set_font_size(DEFAULT_MONO, window, cx));
        })
    };
    let fullscreen = PopupMenuItem::new(tr(lang, "全屏", "Full Screen"))
        .on_click(move |_, window, _cx| window.toggle_fullscreen());
    menu.item(reconnect)
        .item(disconnect)
        .separator()
        .item(rename)
        .item(close)
        .separator()
        .item(zoom_in)
        .item(zoom_out)
        .item(zoom_reset)
        .separator()
        .item(fullscreen)
}

fn dock_menu(
    menu: PopupMenu,
    app: Entity<OneCrt>,
    panel: PanelId,
    lang: Lang,
    from_float: bool,
    _window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let floating = app.read(cx).float_of(panel).is_some();
    let docked = app.read(cx).zone_of(panel).is_some();

    let float = {
        let e = app.clone();
        PopupMenuItem::new(tr(lang, "浮动", "Float"))
            .disabled(floating || from_float)
            .on_click(move |_, window, cx| {
                e.update(cx, |this, cx| this.float_panel(panel, window, cx));
            })
    };
    let dock_item = |zone: DockZone| {
        let e = app.clone();
        let label = match zone {
            DockZone::Left => tr(lang, "停靠到左侧", "Dock Left"),
            DockZone::Right => tr(lang, "停靠到右侧", "Dock Right"),
            DockZone::Bottom => tr(lang, "停靠到底部", "Dock Bottom"),
            DockZone::Top => tr(lang, "停靠到顶部", "Dock Top"),
        };
        PopupMenuItem::new(label).on_click(move |_, _, cx| {
            e.update(cx, |this, cx| this.dock_panel(panel, zone, cx));
        })
    };
    let auto_hide = {
        let e = app.clone();
        let collapsed = app
            .read(cx)
            .zone_of(panel)
            .map(|zone| app.read(cx).groups[zone.index()].collapsed)
            .unwrap_or(false);
        PopupMenuItem::new(tr(lang, "自动隐藏", "Auto Hide"))
            .checked(collapsed)
            .disabled(from_float || !docked)
            .on_click(move |_, _, cx| {
                e.update(cx, |this, cx| {
                    if let Some(zone) = this.zone_of(panel) {
                        let collapsed = !this.groups[zone.index()].collapsed;
                        this.set_collapsed(zone, collapsed, cx);
                    }
                });
            })
    };
    let close = {
        let e = app.clone();
        PopupMenuItem::new(tr(lang, "关闭", "Close")).on_click(move |_, window, cx| {
            e.update(cx, |this, cx| this.forget_panel(panel, cx));
            if from_float {
                window.remove_window();
            }
        })
    };

    menu.item(float)
        .separator()
        .item(dock_item(DockZone::Left))
        .item(dock_item(DockZone::Right))
        .item(dock_item(DockZone::Bottom))
        .item(dock_item(DockZone::Top))
        .separator()
        .item(auto_hide)
        .item(close)
}

/// 浮动出来的面板窗口：自带标题条（拖动区 + 停靠菜单 + 关闭），内容直接复用主窗口的渲染。
struct PanelWindow {
    app: Entity<OneCrt>,
    panel: PanelId,
}

impl PanelWindow {
    fn new(app: Entity<OneCrt>, panel: PanelId, cx: &mut Context<Self>) -> Self {
        cx.observe(&app, |_, _, cx| cx.notify()).detach();
        Self { app, panel }
    }

    fn install_handlers(&self, window: &mut Window, cx: &gpui::App) {
        let app = self.app.clone();
        let panel = self.panel;
        window.on_window_should_close(cx, move |_, cx| {
            app.update(cx, |this, cx| this.forget_panel(panel, cx));
            true
        });
    }
}

impl Render for PanelWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = self.app.clone();
        let panel = self.panel;
        let (t, lang) = {
            let this = app.read(cx);
            (this.theme.clone(), this.lang)
        };
        let content = app.update(cx, |this, cx| this.panel_body(panel, cx));
        let menu_app = app.clone();
        let menu_theme = t.clone();
        let close_app = app.clone();

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .font(t.ui_font.clone())
            .text_size(t.ui_size)
            .child(
                div()
                    .h(px(30.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .pl(px(10.))
                    .pr(px(2.))
                    .bg(t.bg)
                    .border_b_1()
                    .border_color(t.line)
                    .child(
                        div()
                            .id("float-header-drag")
                            .flex_1()
                            .h_full()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(11.5))
                            .text_color(t.text)
                            .window_control_area(WindowControlArea::Drag)
                            .on_mouse_down(MouseButton::Left, |event, window, cx| {
                                if event.click_count == 1
                                    && crate::window_drag::begin_window_drag(window)
                                {
                                    cx.stop_propagation();
                                }
                            })
                            .child(Icon::new(panel.icon()).size(px(13.)))
                            .child(panel.title(lang)),
                    )
                    .child(
                        Button::new("float-dock-menu")
                            .ghost()
                            .compact()
                            .icon(IconName::ChevronDown)
                            .text_color(menu_theme.text_2)
                            .dropdown_menu(move |menu, window, cx| {
                                dock_menu(menu, menu_app.clone(), panel, lang, true, window, cx)
                            }),
                    )
                    .child(
                        div()
                            .id("float-dock-close")
                            .size(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(5.))
                            .cursor_pointer()
                            .text_color(t.text_2)
                            .hover(move |s| {
                                s.bg(gpui::rgb(0xc42b1c)).text_color(gpui::rgb(0xffffff))
                            })
                            .on_click(cx.listener(move |_, _, window, cx| {
                                close_app.update(cx, |app, cx| app.forget_panel(panel, cx));
                                window.remove_window();
                            }))
                            .child(Icon::new(IconName::X).size(px(13.))),
                    ),
            )
            .child(div().flex_1().min_h_0().flex().flex_col().child(content))
    }
}

fn view_menu(
    menu: PopupMenu,
    app: Entity<OneCrt>,
    lang: Lang,
    _window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let (sessions_on, commands_on, files_on, perf_on, sidebar_on, dark) = {
        let s = app.read(cx);
        (
            s.panel_visible(PanelId::Sessions),
            s.panel_visible(PanelId::Commands),
            s.panel_visible(PanelId::Files),
            s.panel_visible(PanelId::Perf),
            {
                let sidebar = &s.groups[DockZone::Left.index()];
                !sidebar.panels.is_empty() && !sidebar.collapsed
            },
            s.theme.mode == Mode::Dark,
        )
    };
    let panel_item = |panel: PanelId, checked: bool, show: bool| {
        let e = app.clone();
        PopupMenuItem::new(panel.title(lang))
            .checked(checked)
            .on_click(move |_, _, cx| {
                e.update(cx, |this, cx| {
                    if show {
                        this.show_panel(panel, cx);
                    } else {
                        this.hide_panel(panel, cx);
                    }
                });
            })
    };
    let sessions = panel_item(PanelId::Sessions, sessions_on, !sessions_on);
    let commands = panel_item(PanelId::Commands, commands_on, !commands_on);
    let files = panel_item(PanelId::Files, files_on, !files_on);
    let perf = panel_item(PanelId::Perf, perf_on, !perf_on);
    let sidebar_empty = app.read(cx).groups[DockZone::Left.index()]
        .panels
        .is_empty();
    let sidebar = {
        let e = app.clone();
        PopupMenuItem::new(tr(lang, "自动隐藏侧栏", "Auto Hide Sidebar"))
            .checked(sidebar_on)
            .disabled(sidebar_empty)
            .on_click(move |_, _, cx| {
                e.update(cx, |this, cx| {
                    let zone = DockZone::Left;
                    let collapsed = !this.groups[zone.index()].collapsed;
                    this.set_collapsed(zone, collapsed, cx);
                });
            })
    };
    let theme = {
        let e = app.clone();
        PopupMenuItem::new(if dark {
            tr(lang, "切换到浅色主题", "Switch to Light Theme")
        } else {
            tr(lang, "切换到深色主题", "Switch to Dark Theme")
        })
        .on_click(move |_, _, cx| {
            e.update(cx, |this, cx| this.toggle_theme(cx));
        })
    };
    let language = {
        let e = app.clone();
        PopupMenuItem::new(tr(lang, "切换语言", "Switch Language")).on_click(
            move |_, window, cx| {
                e.update(cx, |this, cx| this.toggle_lang(window, cx));
            },
        )
    };
    menu.item(sessions)
        .item(commands)
        .separator()
        .item(files)
        .item(perf)
        .separator()
        .item(sidebar)
        .item(theme)
        .item(language)
}

fn help_menu(
    menu: PopupMenu,
    app: Entity<OneCrt>,
    lang: Lang,
    _window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let backend = app.read(cx).backend.clone();
    let mcp = PopupMenuItem::new(tr(lang, "MCP 服务…", "MCP Service…"))
        .on_click(move |_, window, cx| open_mcp_dialog(window, cx, backend.clone(), lang));
    let about = PopupMenuItem::new(tr(lang, "关于 OneCRT", "About OneCRT"))
        .on_click(move |_, window, cx| open_about(window, cx, lang));
    menu.item(mcp).separator().item(about)
}

struct Row {
    id: String,
    name: String,
    depth: usize,
    folder: bool,
    expanded: bool,
}

fn flatten(nodes: &[Node], depth: usize, needle: &str) -> Vec<Row> {
    let mut out = Vec::new();
    let filtering = !needle.is_empty();
    for n in nodes {
        match n {
            Node::Folder {
                id,
                name,
                expanded,
                children,
            } => {
                if !filtering {
                    out.push(Row {
                        id: id.clone(),
                        name: name.clone(),
                        depth,
                        folder: true,
                        expanded: *expanded,
                    });
                    if *expanded {
                        out.extend(flatten(children, depth + 1, needle));
                    }
                } else if name.to_lowercase().contains(needle) {
                    out.push(Row {
                        id: id.clone(),
                        name: name.clone(),
                        depth,
                        folder: true,
                        expanded: true,
                    });
                    out.extend(flatten(children, depth + 1, ""));
                } else {
                    let start = out.len();
                    out.extend(flatten(children, depth + 1, needle));
                    if out.len() > start {
                        out.insert(
                            start,
                            Row {
                                id: id.clone(),
                                name: name.clone(),
                                depth,
                                folder: true,
                                expanded: true,
                            },
                        );
                    }
                }
            }
            Node::Session(s) => {
                let hit = !filtering
                    || s.name.to_lowercase().contains(needle)
                    || s.host.to_lowercase().contains(needle)
                    || s.username.to_lowercase().contains(needle);
                if hit {
                    out.push(Row {
                        id: s.id.clone(),
                        name: s.name.clone(),
                        depth,
                        folder: false,
                        expanded: false,
                    });
                }
            }
        }
    }
    out
}

pub(crate) fn new_id(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{prefix}-{nanos}")
}

fn default_auth() -> Vec<AuthMethod> {
    ["PublicKey", "Password", "Keyboard Interactive"]
        .into_iter()
        .map(|method| AuthMethod {
            method: method.into(),
            enabled: true,
        })
        .collect()
}

fn find_parent_list<'a>(nodes: &'a mut [Node], id: &str) -> Option<&'a mut Vec<Node>> {
    for n in nodes.iter_mut() {
        if let Node::Folder { children, .. } = n {
            let contains = children.iter().any(|c| match c {
                Node::Folder { id: fid, .. } => fid == id,
                Node::Session(s) => s.id == id,
            });
            if contains {
                return Some(children);
            }
            if let Some(found) = find_parent_list(children, id) {
                return Some(found);
            }
        }
    }
    None
}

pub(crate) fn confirm_dialog(
    window: &mut Window,
    cx: &mut gpui::App,
    lang: Lang,
    title: SharedString,
    message: SharedString,
    on_ok: impl Fn(&mut Window, &mut gpui::App) + 'static,
) {
    dialog_window::open_confirm(
        window,
        cx,
        lang,
        title,
        message,
        tr(lang, "删除", "Delete"),
        true,
        on_ok,
    );
}

fn connect_spec(session: &SshSession) -> ConnectSpec {
    ConnectSpec {
        host: session.host.clone(),
        port: session.port,
        username: session.username.clone(),
        password: session.password.clone(),
        key_path: session.key_path.clone(),
        methods: session
            .auth
            .iter()
            .filter(|m| m.enabled)
            .map(|m| m.method.clone())
            .collect(),
    }
}

fn cmd_id(node: &CmdNode) -> &str {
    match node {
        CmdNode::Folder { id, .. } => id,
        CmdNode::Command { id, .. } => id,
    }
}

fn sess_node_id(node: &Node) -> &str {
    match node {
        Node::Folder { id, .. } => id,
        Node::Session(s) => &s.id,
    }
}

fn sess_subtree_has(node: &Node, id: &str) -> bool {
    match node {
        Node::Session(s) => s.id == id,
        Node::Folder {
            id: fid, children, ..
        } => fid == id || children.iter().any(|c| sess_subtree_has(c, id)),
    }
}

fn sess_clone_with_new_ids(node: &Node) -> Node {
    match node {
        Node::Session(s) => Node::Session(SshSession {
            id: new_id("s"),
            ..s.clone()
        }),
        Node::Folder {
            name,
            expanded,
            children,
            ..
        } => Node::Folder {
            id: new_id("f"),
            name: name.clone(),
            expanded: *expanded,
            children: children.iter().map(sess_clone_with_new_ids).collect(),
        },
    }
}

fn subtree_has(node: &CmdNode, id: &str) -> bool {
    match node {
        CmdNode::Command { id: cid, .. } => cid == id,
        CmdNode::Folder {
            id: fid, children, ..
        } => fid == id || children.iter().any(|c| subtree_has(c, id)),
    }
}

fn clone_with_new_ids(node: &CmdNode) -> CmdNode {
    match node {
        CmdNode::Command { name, text, .. } => CmdNode::Command {
            id: new_id("c"),
            name: name.clone(),
            text: text.clone(),
        },
        CmdNode::Folder {
            name,
            expanded,
            children,
            ..
        } => CmdNode::Folder {
            id: new_id("cf"),
            name: name.clone(),
            expanded: *expanded,
            children: children.iter().map(clone_with_new_ids).collect(),
        },
    }
}

struct CmdRow {
    id: String,
    name: String,
    text: Option<String>,
    depth: usize,
    folder: bool,
    expanded: bool,
}

fn flatten_cmds(nodes: &[CmdNode], depth: usize, needle: &str) -> Vec<CmdRow> {
    let mut out = Vec::new();
    let filtering = !needle.is_empty();
    for n in nodes {
        match n {
            CmdNode::Folder {
                id,
                name,
                expanded,
                children,
            } => {
                if !filtering {
                    out.push(CmdRow {
                        id: id.clone(),
                        name: name.clone(),
                        text: None,
                        depth,
                        folder: true,
                        expanded: *expanded,
                    });
                    if *expanded {
                        out.extend(flatten_cmds(children, depth + 1, needle));
                    }
                } else if name.to_lowercase().contains(needle) {
                    out.push(CmdRow {
                        id: id.clone(),
                        name: name.clone(),
                        text: None,
                        depth,
                        folder: true,
                        expanded: true,
                    });
                    out.extend(flatten_cmds(children, depth + 1, ""));
                } else {
                    let start = out.len();
                    out.extend(flatten_cmds(children, depth + 1, needle));
                    if out.len() > start {
                        out.insert(
                            start,
                            CmdRow {
                                id: id.clone(),
                                name: name.clone(),
                                text: None,
                                depth,
                                folder: true,
                                expanded: true,
                            },
                        );
                    }
                }
            }
            CmdNode::Command { id, name, text } => {
                if !filtering
                    || name.to_lowercase().contains(needle)
                    || text.to_lowercase().contains(needle)
                {
                    out.push(CmdRow {
                        id: id.clone(),
                        name: name.clone(),
                        text: Some(text.clone()),
                        depth,
                        folder: false,
                        expanded: false,
                    });
                }
            }
        }
    }
    out
}

pub fn window_options(cx: &mut gpui::App) -> WindowOptions {
    use gpui::{size, Bounds, TitlebarOptions, WindowBounds};
    let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some("OneCRT".into()),
            appears_transparent: true,
            traffic_light_position: None,
        }),
        window_min_size: Some(size(px(880.), px(560.))),
        ..Default::default()
    }
}
