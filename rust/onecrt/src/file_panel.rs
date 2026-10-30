//! 右侧文件面板：本地 / 远端左右双栏，上传下载与增删改，底部固定传输队列。
//!
//! 远端走 Core 里独立的 SFTP 连接（按标签页 id 复用），所以终端断开也不影响传文件。

use std::collections::HashSet;
use std::sync::Arc;

use async_channel::{unbounded, Sender};
use gpui_kit as gpui;

use gpui::{
    div, px, relative, AppContext, Context, CursorStyle, FocusHandle, InteractiveElement,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Window,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    menu::{DropdownMenu, PopupMenuItem},
    Disableable, Icon,
};
use onecrt_core::files::{self, Entry, OpenArgs, Progress};
use onecrt_core::Backend;

use crate::app::confirm_dialog;
use crate::i18n::{tr, Lang};
use crate::session_form::open_text_prompt;
use crate::term_view::ConnectSpec;
use crate::theme::Theme;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SideKind {
    Local,
    Remote,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RemoteState {
    Idle,
    Connecting,
    Ready,
}

#[derive(Default, Clone)]
struct Side {
    path: String,
    parent: String,
    entries: Vec<Entry>,
    /// 多选按路径记，重新列目录后还能对上号
    sel: Vec<String>,
    /// Shift 范围选择与框选的锚点
    anchor: Option<String>,
}

/// 框选进行中：按下左键的那一行是起点，base 是按下前已有的选区
#[derive(Clone)]
struct DragState {
    side: SideKind,
    start: usize,
    last: usize,
    base: Vec<String>,
    /// 按下时捏着 Ctrl：框出来的并进原选区，而不是替换
    additive: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TransferStatus {
    /// 一批里的后续文件先占好队，前面的传完才轮到
    Queued,
    Running,
    Done,
    Failed,
}

struct TransferRow {
    up: bool,
    name: String,
    done: u64,
    total: u64,
    status: TransferStatus,
    error: String,
}

enum Msg {
    Opened {
        id: String,
        home: Result<String, String>,
    },
    LocalListed {
        path: String,
        parent: String,
        entries: Vec<Entry>,
    },
    RemoteListed {
        path: String,
        entries: Vec<Entry>,
    },
    Failed(String),
    Progress {
        name: String,
        done: u64,
        total: u64,
    },
    Finished {
        name: String,
        up: bool,
    },
    TransferFailed {
        name: String,
        error: String,
    },
    Reload(SideKind),
}

pub struct FilePanel {
    backend: Arc<Backend>,
    theme: Theme,
    lang: Lang,
    tx: Sender<Msg>,
    _pump: gpui::Task<()>,
    session: Option<(String, ConnectSpec)>,
    remote_state: RemoteState,
    local: Side,
    remote: Side,
    focus: SideKind,
    /// 面板拿到键盘焦点后才能接 Ctrl+A
    key_focus: FocusHandle,
    drag: Option<DragState>,
    transfers: Vec<TransferRow>,
    error: Option<String>,
    /// 传输队列高度与本地栏宽度；由分隔条拖动改变，面板重建时保留。
    queue_h: f32,
    local_w: f32,
}

/// 拖分隔条时的空载浮层：只为让 gpui 的 drag 生命周期跑起来，本身不画东西。
struct DividerGhost;

impl Render for DividerGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

struct QueueDivider;

struct PaneDivider;

const QUEUE_MIN_H: f32 = 76.;
const PANE_MIN_W: f32 = 160.;

impl FilePanel {
    pub fn new(backend: Arc<Backend>, theme: Theme, lang: Lang, cx: &mut Context<Self>) -> Self {
        let (tx, rx) = unbounded::<Msg>();
        let pump = cx.spawn(async move |this, cx| {
            while let Ok(msg) = rx.recv().await {
                if this.update(cx, |panel, cx| panel.apply(msg, cx)).is_err() {
                    break;
                }
            }
        });
        let panel = Self {
            backend,
            theme,
            lang,
            tx,
            _pump: pump,
            session: None,
            remote_state: RemoteState::Idle,
            local: Side::default(),
            remote: Side::default(),
            focus: SideKind::Local,
            key_focus: cx.focus_handle(),
            drag: None,
            transfers: Vec::new(),
            error: None,
            queue_h: 132.,
            local_w: 0.,
        };
        panel.list_local(files::local_home());
        panel
    }

    pub fn set_theme(&mut self, theme: Theme, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    pub fn set_lang(&mut self, lang: Lang, cx: &mut Context<Self>) {
        self.lang = lang;
        cx.notify();
    }

    /// 面板跟着活动标签页走：换 id 就重开一条 SFTP 连接，旧连接立刻关掉。
    pub fn set_session(&mut self, session: Option<(String, ConnectSpec)>, cx: &mut Context<Self>) {
        let new_id = session.as_ref().map(|(id, _)| id.clone());
        let old_id = self.session.as_ref().map(|(id, _)| id.clone());
        if new_id == old_id {
            return;
        }
        self.session = session.clone();
        self.remote = Side::default();
        self.remote_state = RemoteState::Idle;
        if let Some(old) = old_id {
            let mgr = self.backend.files.clone();
            self.backend.spawn(async move {
                files::close(&mgr, old).await;
            });
        }
        let Some((id, spec)) = session else {
            cx.notify();
            return;
        };
        self.remote_state = RemoteState::Connecting;
        let args = OpenArgs {
            id: id.clone(),
            host: spec.host,
            port: spec.port,
            username: spec.username,
            password: spec.password,
            key_path: spec.key_path,
            methods: spec.methods,
        };
        let mgr = self.backend.files.clone();
        let tx = self.tx.clone();
        self.backend.spawn(async move {
            let home = files::open(&mgr, args).await;
            let _ = tx.try_send(Msg::Opened { id, home });
        });
        cx.notify();
    }

    /// 关闭面板时断开 SFTP，别让后台连接常驻。
    pub fn shutdown(&mut self, cx: &mut Context<Self>) {
        if let Some((id, _)) = self.session.take() {
            let mgr = self.backend.files.clone();
            self.backend.spawn(async move {
                files::close(&mgr, id).await;
            });
        }
        self.remote = Side::default();
        self.remote_state = RemoteState::Idle;
        cx.notify();
    }

    fn apply(&mut self, msg: Msg, cx: &mut Context<Self>) {
        match msg {
            Msg::Opened { id, home } => {
                if self.session.as_ref().map(|(sid, _)| sid) != Some(&id) {
                    let mgr = self.backend.files.clone();
                    self.backend.spawn(async move {
                        files::close(&mgr, id).await;
                    });
                    return;
                }
                match home {
                    Ok(home) => {
                        self.remote_state = RemoteState::Ready;
                        self.error = None;
                        self.list_remote(home);
                    }
                    Err(e) => {
                        self.remote_state = RemoteState::Idle;
                        self.error = Some(e);
                    }
                }
            }
            Msg::LocalListed {
                path,
                parent,
                entries,
            } => {
                let (sel, anchor) = keep_sel(&self.local, &entries);
                self.local = Side {
                    path,
                    parent,
                    entries,
                    sel,
                    anchor,
                };
                self.error = None;
            }
            Msg::RemoteListed { path, entries } => {
                let (sel, anchor) = keep_sel(&self.remote, &entries);
                self.remote = Side {
                    parent: remote_parent(&path),
                    path,
                    entries,
                    sel,
                    anchor,
                };
                self.error = None;
            }
            Msg::Failed(e) => {
                self.error = Some(e);
            }
            Msg::Progress { name, done, total } => {
                if let Some(row) = self.active_row(&name) {
                    row.status = TransferStatus::Running;
                    row.done = done;
                    row.total = total.max(row.total);
                }
            }
            Msg::Finished { name, up } => {
                if let Some(row) = self.active_row(&name) {
                    row.status = TransferStatus::Done;
                    row.done = row.total.max(row.done);
                }
                if up {
                    let path = self.remote.path.clone();
                    if !path.is_empty() {
                        self.list_remote(path);
                    }
                } else {
                    let path = self.local.path.clone();
                    if !path.is_empty() {
                        self.list_local(path);
                    }
                }
            }
            Msg::TransferFailed { name, error } => {
                if let Some(row) = self.active_row(&name) {
                    row.status = TransferStatus::Failed;
                    row.error = error.clone();
                }
                self.error = Some(error);
            }
            Msg::Reload(side) => match side {
                SideKind::Local => {
                    let path = self.local.path.clone();
                    self.list_local(path);
                }
                SideKind::Remote => {
                    let path = self.remote.path.clone();
                    if !path.is_empty() {
                        self.list_remote(path);
                    }
                }
            },
        }
        cx.notify();
    }

    fn list_local(&self, path: String) {
        let tx = self.tx.clone();
        self.backend.spawn(async move {
            match files::local_list(path.clone()).await {
                Ok(list) => {
                    let _ = tx.try_send(Msg::LocalListed {
                        path,
                        parent: list.parent,
                        entries: list.entries,
                    });
                }
                Err(e) => {
                    let _ = tx.try_send(Msg::Failed(e));
                }
            }
        });
    }

    fn list_remote(&self, path: String) {
        let Some((id, _)) = self.session.clone() else {
            return;
        };
        let mgr = self.backend.files.clone();
        let tx = self.tx.clone();
        self.backend.spawn(async move {
            match files::list(&mgr, id, path.clone()).await {
                Ok(entries) => {
                    let _ = tx.try_send(Msg::RemoteListed { path, entries });
                }
                Err(e) => {
                    let _ = tx.try_send(Msg::Failed(e));
                }
            }
        });
    }

    fn side(&self, side: SideKind) -> &Side {
        match side {
            SideKind::Local => &self.local,
            SideKind::Remote => &self.remote,
        }
    }

    fn side_mut(&mut self, side: SideKind) -> &mut Side {
        match side {
            SideKind::Local => &mut self.local,
            SideKind::Remote => &mut self.remote,
        }
    }

    /// 选中的条目，按列表顺序返回（目录在前，传输顺序也跟着列表走）
    fn selected_entries(&self, kind: SideKind) -> Vec<Entry> {
        let side = self.side(kind);
        let set: HashSet<&str> = side.sel.iter().map(String::as_str).collect();
        side.entries
            .iter()
            .filter(|e| set.contains(e.path.as_str()))
            .cloned()
            .collect()
    }

    fn sel_count(&self, kind: SideKind) -> usize {
        self.selected_entries(kind).len()
    }

    /// 选中项里可传输的文件数（目录不算）
    fn sel_file_count(&self, kind: SideKind) -> usize {
        self.selected_entries(kind)
            .iter()
            .filter(|e| !e.dir)
            .count()
    }

    /// 重命名这类逐条操作只在恰好选中一个时才开放
    fn only_selected(&self, kind: SideKind) -> Option<Entry> {
        let list = self.selected_entries(kind);
        (list.len() == 1).then(|| list[0].clone())
    }

    /// 单击 / Ctrl 增删 / Shift 范围，和 Windows 资源管理器一致
    fn click_row(&mut self, kind: SideKind, index: usize, ctrl: bool, shift: bool) {
        let Some(path) = self.side(kind).entries.get(index).map(|e| e.path.clone()) else {
            return;
        };
        if shift {
            let start = self
                .side(kind)
                .anchor
                .as_ref()
                .and_then(|a| self.side(kind).entries.iter().position(|e| &e.path == a))
                .unwrap_or(index);
            let side = self.side_mut(kind);
            add_range(&mut side.sel, &side.entries, start, index, !ctrl);
        } else if ctrl {
            let side = self.side_mut(kind);
            if side.sel.iter().any(|p| p == &path) {
                side.sel.retain(|p| p != &path);
            } else {
                side.sel.push(path.clone());
            }
        } else {
            self.side_mut(kind).sel = vec![path.clone()];
        }
        // Shift 要保住锚点才能来回扩范围，其余点击把锚点挪到这一行
        let side = self.side_mut(kind);
        let anchor = if shift {
            side.anchor
                .clone()
                .filter(|a| side.sel.iter().any(|p| p == a))
                .or_else(|| Some(path.clone()))
        } else {
            Some(path.clone())
        };
        side.anchor = anchor;
    }

    /// 框选：起点到当前行整段入选；按下时捏着 Ctrl 就并到原选区上
    fn drag_to(&mut self, drag: &DragState, index: usize) -> bool {
        if drag.last == index {
            return false;
        }
        let kind = drag.side;
        let side = self.side_mut(kind);
        if drag.additive {
            side.sel = drag.base.clone();
        }
        add_range(
            &mut side.sel,
            &side.entries,
            drag.start,
            index,
            !drag.additive,
        );
        side.anchor = side.entries.get(drag.start).map(|e| e.path.clone());
        true
    }

    fn select_all(&mut self, kind: SideKind) {
        let side = self.side_mut(kind);
        side.anchor = side.entries.first().map(|e| e.path.clone());
        side.sel = side.entries.iter().map(|e| e.path.clone()).collect();
    }

    fn clear_sel(&mut self, kind: SideKind) {
        let side = self.side_mut(kind);
        side.sel.clear();
        side.anchor = None;
    }

    fn end_drag(&mut self, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            cx.notify();
        }
    }

    fn is_pending(status: TransferStatus) -> bool {
        matches!(status, TransferStatus::Queued | TransferStatus::Running)
    }

    /// 一批里的队头：还在排队或正在传的那一行，按文件名回找（同一目录内文件名唯一）
    fn active_row(&mut self, name: &str) -> Option<&mut TransferRow> {
        self.transfers
            .iter_mut()
            .rev()
            .find(|r| r.name == name && Self::is_pending(r.status))
    }

    /// 同一时刻只跑一个传输，和有进度可看、不会互相抢带宽的原版行为一致。
    fn busy(&self) -> bool {
        self.transfers.iter().any(|r| Self::is_pending(r.status))
    }

    fn upload_selected(&mut self, cx: &mut Context<Self>) {
        let files: Vec<Entry> = self
            .selected_entries(SideKind::Local)
            .into_iter()
            .filter(|e| !e.dir)
            .collect();
        self.start_transfer(files, true, cx);
    }

    fn download_selected(&mut self, cx: &mut Context<Self>) {
        let files: Vec<Entry> = self
            .selected_entries(SideKind::Remote)
            .into_iter()
            .filter(|e| !e.dir)
            .collect();
        self.start_transfer(files, false, cx);
    }

    /// 双击下载只传被点的那一行，不动多选选区
    fn download_entry(&mut self, entry: Entry, cx: &mut Context<Self>) {
        self.start_transfer(vec![entry], false, cx);
    }

    /// 一批文件顺序传，前一个结束才起下一个，保持「同一时刻只跑一个」的原版行为。
    fn start_transfer(&mut self, entries: Vec<Entry>, up: bool, cx: &mut Context<Self>) {
        if entries.is_empty() || self.busy() {
            return;
        }
        let Some((id, _)) = self.session.clone() else {
            return;
        };
        let dst_dir = if up {
            self.remote.path.clone()
        } else {
            self.local.path.clone()
        };
        if dst_dir.is_empty() || (up && self.remote_state != RemoteState::Ready) {
            return;
        }
        for (i, entry) in entries.iter().enumerate() {
            self.push_transfer(
                entry,
                up,
                if i == 0 {
                    TransferStatus::Running
                } else {
                    TransferStatus::Queued
                },
            );
        }
        let tx = self.tx.clone();
        let sink = progress_sink(tx.clone(), up);
        let mgr = self.backend.files.clone();
        self.backend.spawn(async move {
            for entry in entries {
                let name = entry.name.clone();
                let result = if up {
                    files::upload(&mgr, id.clone(), entry.path, dst_dir.clone(), sink.clone()).await
                } else {
                    files::download(&mgr, id.clone(), entry.path, dst_dir.clone(), sink.clone())
                        .await
                };
                if let Err(e) = result {
                    let _ = tx.try_send(Msg::TransferFailed { name, error: e });
                }
            }
        });
        cx.notify();
    }

    fn push_transfer(&mut self, entry: &Entry, up: bool, status: TransferStatus) {
        self.transfers.push(TransferRow {
            up,
            name: entry.name.clone(),
            done: 0,
            total: entry.size.unwrap_or(0),
            status,
            error: String::new(),
        });
    }

    fn clear_finished(&mut self, cx: &mut Context<Self>) {
        self.transfers.retain(|r| Self::is_pending(r.status));
        cx.notify();
    }

    fn new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let side = self.focus;
        let dir = self.side(side).path.clone();
        let usable = match side {
            SideKind::Local => !dir.is_empty(),
            SideKind::Remote => self.remote_state == RemoteState::Ready,
        };
        if !usable {
            return;
        }
        let entity = cx.entity();
        let lang = self.lang;
        open_text_prompt(
            window,
            cx,
            lang,
            tr(lang, "新建目录", "New Folder"),
            "",
            tr(lang, "目录名称", "Folder name").into(),
            move |name, _, cx| {
                entity.update(cx, |this, cx| {
                    this.create_folder(side, dir.clone(), name, cx)
                });
            },
        );
    }

    fn create_folder(&mut self, side: SideKind, dir: String, name: String, cx: &mut Context<Self>) {
        if name.trim().is_empty() {
            return;
        }
        let tx = self.tx.clone();
        match side {
            SideKind::Local => {
                self.backend.spawn(async move {
                    match files::local_mkdir(dir, name).await {
                        Ok(()) => {
                            let _ = tx.try_send(Msg::Reload(SideKind::Local));
                        }
                        Err(e) => {
                            let _ = tx.try_send(Msg::Failed(e));
                        }
                    }
                });
            }
            SideKind::Remote => {
                let Some((id, _)) = self.session.clone() else {
                    return;
                };
                let mgr = self.backend.files.clone();
                self.backend.spawn(async move {
                    match files::mkdir(&mgr, id, remote_child(&dir, &name)).await {
                        Ok(()) => {
                            let _ = tx.try_send(Msg::Reload(SideKind::Remote));
                        }
                        Err(e) => {
                            let _ = tx.try_send(Msg::Failed(e));
                        }
                    }
                });
            }
        }
        cx.notify();
    }

    fn rename_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let side = self.focus;
        let Some(entry) = self.only_selected(side) else {
            return;
        };
        let entity = cx.entity();
        let lang = self.lang;
        let initial = entry.name.clone();
        open_text_prompt(
            window,
            cx,
            lang,
            tr(lang, "重命名", "Rename"),
            &initial,
            tr(lang, "名称", "Name").into(),
            move |name, _, cx| {
                entity.update(cx, |this, cx| {
                    this.rename_entry(side, entry.clone(), name, cx)
                });
            },
        );
    }

    fn rename_entry(&mut self, side: SideKind, entry: Entry, name: String, cx: &mut Context<Self>) {
        let name = name.trim().to_string();
        if name.is_empty() || name == entry.name {
            return;
        }
        let tx = self.tx.clone();
        match side {
            SideKind::Local => {
                self.backend.spawn(async move {
                    match files::local_rename(entry.path, name).await {
                        Ok(()) => {
                            let _ = tx.try_send(Msg::Reload(SideKind::Local));
                        }
                        Err(e) => {
                            let _ = tx.try_send(Msg::Failed(e));
                        }
                    }
                });
            }
            SideKind::Remote => {
                let Some((id, _)) = self.session.clone() else {
                    return;
                };
                let mgr = self.backend.files.clone();
                let dest = remote_child(&self.remote.path, &name);
                self.backend.spawn(async move {
                    match files::rename(&mgr, id, entry.path, dest).await {
                        Ok(()) => {
                            let _ = tx.try_send(Msg::Reload(SideKind::Remote));
                        }
                        Err(e) => {
                            let _ = tx.try_send(Msg::Failed(e));
                        }
                    }
                });
            }
        }
        cx.notify();
    }

    fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let side = self.focus;
        let entries = self.selected_entries(side);
        if entries.is_empty() {
            return;
        }
        let entity = cx.entity();
        let lang = self.lang;
        let message: SharedString = if entries.len() == 1 {
            tr(lang, "该条目将被删除。", "This item will be removed.")
        } else if lang == Lang::Zh {
            format!("已选中的 {} 项将被删除。", entries.len()).into()
        } else {
            format!("{} selected items will be removed.", entries.len()).into()
        };
        confirm_dialog(
            window,
            cx,
            lang,
            tr(lang, "删除", "Delete"),
            message,
            move |_, cx| {
                entity.update(cx, |this, cx| {
                    this.remove_entries(side, entries.clone(), cx)
                });
            },
        );
    }

    /// 逐个删完再刷新一次，中途失败只记录错误、不中断这一批
    fn remove_entries(&mut self, side: SideKind, entries: Vec<Entry>, cx: &mut Context<Self>) {
        let tx = self.tx.clone();
        match side {
            SideKind::Local => {
                self.backend.spawn(async move {
                    for entry in entries {
                        if let Err(e) = files::local_remove(entry.path, entry.dir).await {
                            let _ = tx.try_send(Msg::Failed(e));
                        }
                    }
                    let _ = tx.try_send(Msg::Reload(SideKind::Local));
                });
            }
            SideKind::Remote => {
                let Some((id, _)) = self.session.clone() else {
                    return;
                };
                let mgr = self.backend.files.clone();
                self.backend.spawn(async move {
                    for entry in entries {
                        if let Err(e) = files::remove(&mgr, id.clone(), entry.path, entry.dir).await
                        {
                            let _ = tx.try_send(Msg::Failed(e));
                        }
                    }
                    let _ = tx.try_send(Msg::Reload(SideKind::Remote));
                });
            }
        }
        cx.notify();
    }
}

fn progress_sink(tx: Sender<Msg>, up: bool) -> files::Sink {
    Arc::new(move |p: Progress| match p {
        Progress::Tick { name, done, total } => {
            let _ = tx.try_send(Msg::Progress { name, done, total });
        }
        Progress::Done { name } => {
            let _ = tx.try_send(Msg::Finished { name, up });
        }
    })
}

/// 把 from..=to 这段连续行加进选区（两端顺序不限）；replace 时先丢掉原有选区
fn add_range(sel: &mut Vec<String>, entries: &[Entry], from: usize, to: usize, replace: bool) {
    if replace {
        sel.clear();
    }
    if entries.is_empty() {
        return;
    }
    let last = entries.len() - 1;
    let lo = from.min(to).min(last);
    let hi = from.max(to).min(last);
    let mut added: Vec<String> = Vec::new();
    {
        let mut seen: HashSet<&str> = sel.iter().map(String::as_str).collect();
        for entry in &entries[lo..=hi] {
            if seen.insert(entry.path.as_str()) {
                added.push(entry.path.clone());
            }
        }
    }
    sel.extend(added);
}

/// 重新列目录后只保留还在列表里的选中项，传输途中刷新才不会把选区抹掉
fn keep_sel(old: &Side, entries: &[Entry]) -> (Vec<String>, Option<String>) {
    let sel: Vec<String> = old
        .sel
        .iter()
        .filter(|p| entries.iter().any(|e| &e.path == *p))
        .cloned()
        .collect();
    let anchor = old.anchor.clone().filter(|a| sel.iter().any(|p| p == a));
    (sel, anchor)
}

fn remote_parent(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) => "/".into(),
        Some(i) => trimmed[..i].to_string(),
        None => "/".into(),
    }
}

fn remote_child(dir: &str, name: &str) -> String {
    format!("{}/{}", dir.trim_end_matches('/'), name)
}

/// 面包屑：Windows 走盘符分支（C:\a\b → C: › a › b），远端 POSIX 路径从 / 起。
fn crumbs(path: &str) -> Vec<(String, String)> {
    if path.trim().is_empty() {
        return Vec::new();
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        let drive = &path[..2];
        let rest: Vec<&str> = path[2..]
            .split(['\\', '/'])
            .filter(|s| !s.is_empty())
            .collect();
        let mut out = vec![(drive.to_string(), format!("{drive}\\"))];
        for (i, name) in rest.iter().enumerate() {
            out.push((
                name.to_string(),
                format!("{drive}\\{}", rest[..=i].join("\\")),
            ));
        }
        return out;
    }
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let mut out = vec![("/".to_string(), "/".to_string())];
    for (i, name) in parts.iter().enumerate() {
        out.push((name.to_string(), format!("/{}", parts[..=i].join("/"))));
    }
    out
}

fn fmt_size(size: Option<u64>) -> String {
    let Some(n) = size else {
        return "—".into();
    };
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024. && unit < UNITS.len() - 1 {
        value /= 1024.;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn fmt_date(mtime: Option<i64>) -> String {
    let Some(secs) = mtime else {
        return String::new();
    };
    match chrono::DateTime::from_timestamp(secs, 0) {
        Some(utc) => utc
            .with_timezone(&chrono::Local)
            .format("%m-%d %H:%M")
            .to_string(),
        None => String::new(),
    }
}

impl Render for FilePanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme.clone();
        let lang = self.lang;

        let local_files = self.sel_file_count(SideKind::Local);
        let remote_files = self.sel_file_count(SideKind::Remote);
        let target_sel = self.sel_count(self.focus);
        let target_path = match self.focus {
            SideKind::Local => !self.local.path.is_empty(),
            SideKind::Remote => self.remote_state == RemoteState::Ready,
        };
        let busy = self.busy();
        let can_upload = local_files > 0
            && !busy
            && self.remote_state == RemoteState::Ready
            && !self.remote.path.is_empty();
        let can_download = remote_files > 0 && !busy && !self.local.path.is_empty();
        let title: SharedString = self
            .session
            .as_ref()
            .map(|(_, spec)| format!("{}@{}", spec.username, spec.host).into())
            .unwrap_or_else(|| tr(lang, "远端", "Remote"));

        let toolbar = div()
            .h(px(30.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(1.))
            .px(px(6.))
            .child(
                Button::new("fp-upload")
                    .ghost()
                    .compact()
                    .icon(IconName::FileUp)
                    .tooltip(tr(
                        lang,
                        "上传：本地选中项 → 远端当前目录",
                        "Upload: local selection → remote current directory",
                    ))
                    .disabled(!can_upload)
                    .on_click(cx.listener(|this, _, _, cx| this.upload_selected(cx))),
            )
            .child(
                Button::new("fp-download")
                    .ghost()
                    .compact()
                    .icon(IconName::FileDown)
                    .tooltip(tr(
                        lang,
                        "下载：远端选中项 → 本地当前目录",
                        "Download: remote selection → local current directory",
                    ))
                    .disabled(!can_download)
                    .on_click(cx.listener(|this, _, _, cx| this.download_selected(cx))),
            )
            .child(
                div()
                    .w(px(1.))
                    .h(px(16.))
                    .mx(px(4.))
                    .flex_shrink_0()
                    .bg(t.line),
            )
            .child(
                Button::new("fp-newdir")
                    .ghost()
                    .compact()
                    .icon(IconName::FolderPlus)
                    .tooltip(match self.focus {
                        SideKind::Local => tr(
                            lang,
                            "在本地当前目录新建子目录",
                            "New subdirectory in the local current directory",
                        ),
                        SideKind::Remote => tr(
                            lang,
                            "在远端当前目录新建子目录",
                            "New subdirectory in the remote current directory",
                        ),
                    })
                    .disabled(!target_path)
                    .on_click(cx.listener(|this, _, window, cx| this.new_folder(window, cx))),
            )
            .child(
                Button::new("fp-rename")
                    .ghost()
                    .compact()
                    .icon(IconName::Pencil)
                    .tooltip(tr(lang, "重命名选中项", "Rename selection"))
                    .disabled(target_sel != 1)
                    .on_click(cx.listener(|this, _, window, cx| this.rename_selected(window, cx))),
            )
            .child(
                Button::new("fp-delete")
                    .ghost()
                    .compact()
                    .icon(IconName::Trash)
                    .tooltip(tr(lang, "删除选中项", "Delete selection"))
                    .disabled(target_sel == 0)
                    .on_click(cx.listener(|this, _, window, cx| this.delete_selected(window, cx))),
            );

        let hover_bg = t.bg_hover;
        let panes = div()
            .id("fp-panes")
            .flex_1()
            .min_h(px(0.))
            .flex()
            .on_drag_move::<PaneDivider>(cx.listener(
                |this, ev: &gpui::DragMoveEvent<PaneDivider>, _window, cx| {
                    let left = f32::from(ev.bounds.origin.x);
                    let wide = f32::from(ev.bounds.size.width);
                    let x = f32::from(ev.event.position.x);
                    this.local_w =
                        (x - left).clamp(PANE_MIN_W, (wide - PANE_MIN_W).max(PANE_MIN_W));
                    cx.notify();
                },
            ))
            .child(self.render_side(
                SideKind::Local,
                tr(lang, "本地", "Local"),
                (self.local_w > 0.).then_some(self.local_w),
                cx,
            ))
            .child(
                div()
                    .id("fp-split-pane")
                    .w(px(5.))
                    .h_full()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor(CursorStyle::ResizeColumn)
                    .hover(move |s| s.bg(hover_bg))
                    .on_drag(PaneDivider, |_, _, _, cx| cx.new(|_| DividerGhost))
                    .child(div().w(px(1.)).h_full().bg(t.line)),
            )
            .child(self.render_side(SideKind::Remote, title, None, cx));

        let any_finished = self.transfers.iter().any(|r| !Self::is_pending(r.status));
        let mut queue_list = div()
            .id("fp-transfers")
            .flex()
            .flex_col()
            .py(px(2.))
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll();
        if self.transfers.is_empty() {
            queue_list = queue_list.child(
                div()
                    .px(px(10.))
                    .py(px(8.))
                    .text_size(px(11.))
                    .text_color(t.text_3)
                    .child(tr(
                        lang,
                        "暂无传输：在文件面板里选中文件后点上传或下载",
                        "No transfers yet: select a file in the file panel and click upload or download",
                    )),
            );
        }
        for row in &self.transfers {
            let pct = if row.total == 0 {
                if row.status == TransferStatus::Done {
                    100
                } else {
                    0
                }
            } else {
                (row.done * 100 / row.total).min(100)
            };
            let (status_text, status_color) = match row.status {
                TransferStatus::Queued => (tr(lang, "排队", "Queued"), t.text_3),
                TransferStatus::Running => (tr(lang, "传输中", "Transferring"), t.text_2),
                TransferStatus::Done => (tr(lang, "完成", "Done"), t.accent),
                TransferStatus::Failed => (tr(lang, "失败", "Failed"), t.danger),
            };
            let bar_color = match row.status {
                TransferStatus::Queued => t.border,
                TransferStatus::Running => t.text_2,
                TransferStatus::Done => t.accent,
                TransferStatus::Failed => t.danger,
            };
            let name_color = if row.status == TransferStatus::Failed {
                t.danger
            } else {
                t.text
            };
            queue_list = queue_list.child(
                div()
                    .h(px(22.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .text_size(px(11.))
                    .child(
                        div()
                            .w(px(12.))
                            .flex()
                            .justify_center()
                            .text_color(status_color)
                            .child(
                                Icon::new(if row.up {
                                    IconName::ArrowUp
                                } else {
                                    IconName::ArrowDown
                                })
                                .size(px(12.)),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(name_color)
                            .child(row.name.clone()),
                    )
                    .child(
                        div()
                            .w(px(64.))
                            .flex_shrink_0()
                            .h(px(4.))
                            .rounded(px(2.))
                            .bg(t.bg_hover)
                            .child(
                                div()
                                    .h(px(4.))
                                    .w(relative(pct as f32 / 100.))
                                    .rounded(px(2.))
                                    .bg(bar_color),
                            ),
                    )
                    .child(
                        div()
                            .w(px(30.))
                            .flex_shrink_0()
                            .text_size(px(10.))
                            .text_color(t.text_3)
                            .child(format!("{pct}%")),
                    )
                    .child(
                        div()
                            .w(px(44.))
                            .flex_shrink_0()
                            .text_size(px(10.))
                            .text_color(status_color)
                            .child(status_text),
                    ),
            );
        }

        let queue = div()
            .id("fp-queue")
            .h(px(self.queue_h))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .on_drag_move::<QueueDivider>(cx.listener(
                |this, ev: &gpui::DragMoveEvent<QueueDivider>, _window, cx| {
                    let bottom = f32::from(ev.bounds.origin.y + ev.bounds.size.height);
                    let y = f32::from(ev.event.position.y);
                    this.queue_h = (bottom - y).clamp(QUEUE_MIN_H, 420.);
                    cx.notify();
                },
            ))
            .child(
                div()
                    .h(px(26.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .pl(px(10.))
                    .pr(px(4.))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(t.text)
                            .child(tr(lang, "传输", "Transfers")),
                    )
                    .child(
                        Button::new("fp-clear-finished")
                            .ghost()
                            .compact()
                            .icon(IconName::Eraser)
                            .tooltip(tr(lang, "清除已完成的记录", "Clear finished"))
                            .disabled(!any_finished)
                            .on_click(cx.listener(|this, _, _, cx| this.clear_finished(cx))),
                    ),
            )
            .child(
                div()
                    .h(px(18.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .text_size(px(10.))
                    .text_color(t.text_3)
                    .child(div().w(px(12.)))
                    .child(div().flex_1().child(tr(lang, "文件", "File")))
                    .child(div().w(px(64.)).child(tr(lang, "进度", "Progress")))
                    .child(div().w(px(30.)).child("%"))
                    .child(div().w(px(44.)).child(tr(lang, "状态", "Status"))),
            )
            .child(queue_list);

        let conn: SharedString = self
            .session
            .as_ref()
            .map(|(_, spec)| format!("{}@{}", spec.username, spec.host).into())
            .unwrap_or_else(|| tr(lang, "未连接", "Not connected"));
        let side_label = match self.focus {
            SideKind::Local => tr(lang, "本地", "Local"),
            SideKind::Remote => tr(lang, "远端", "Remote"),
        };
        let sel = self.selected_entries(self.focus);
        let foot: SharedString = if sel.len() == 1 {
            format!(
                "{} {} · {}",
                tr(lang, "已选中", "Selected"),
                sel[0].name,
                side_label
            )
            .into()
        } else if sel.is_empty() {
            format!(
                "{conn} · {}",
                tr(
                    lang,
                    "Ctrl/Shift 点选、拖拽框选、Ctrl+A 全选",
                    "Ctrl/Shift click, drag to box-select, Ctrl+A for all"
                )
            )
            .into()
        } else if lang == Lang::Zh {
            format!("已选中 {} 项 · {}", sel.len(), side_label).into()
        } else {
            format!("{} selected · {}", sel.len(), side_label).into()
        };
        let footer = div()
            .h(px(24.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .px(px(8.))
            .text_size(px(11.))
            .text_color(t.text_3)
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .child(foot);

        let queue_handle = div()
            .id("fp-split-queue")
            .h(px(5.))
            .w_full()
            .flex_shrink_0()
            .flex()
            .items_center()
            .cursor(CursorStyle::ResizeRow)
            .hover(move |s| s.bg(hover_bg))
            .on_drag(QueueDivider, |_, _, _, cx| cx.new(|_| DividerGhost))
            .child(div().w_full().h(px(1.)).bg(t.line));

        let key_focus = self.key_focus.clone();
        let mut body = div()
            .w_full()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(t.bg)
            .key_context("FilePanel")
            .track_focus(&key_focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                let ks = &ev.keystroke;
                if ks.modifiers.control && !ks.modifiers.alt && ks.key.eq_ignore_ascii_case("a") {
                    this.select_all(this.focus);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(toolbar)
            .child(panes)
            .child(queue_handle)
            .child(queue)
            .child(footer);

        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .px(px(8.))
                    .py(px(4.))
                    .flex_shrink_0()
                    .text_size(px(10.))
                    .text_color(t.danger)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(SharedString::from(error.clone())),
            );
        }

        body
    }
}

impl FilePanel {
    fn render_side(
        &self,
        side: SideKind,
        title: SharedString,
        fixed_w: Option<f32>,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let t = self.theme.clone();
        let side_el = match fixed_w {
            Some(w) => div().w(px(w)).flex_shrink_0(),
            None => div().flex_1(),
        };
        side_el
            .flex()
            .flex_col()
            .min_w(px(0.))
            .min_h(px(0.))
            .child(div().h(px(1.)).flex_shrink_0().bg(if self.focus == side {
                t.accent
            } else {
                t.line
            }))
            .child(self.render_side_header(side, title, cx))
            .child(self.render_side_path(side, cx))
            .child(self.render_side_list(side, cx))
    }

    fn render_side_header(
        &self,
        side: SideKind,
        title: SharedString,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let t = self.theme.clone();
        let lang = self.lang;
        let data = self.side(side).clone();
        let ready = side == SideKind::Local
            || (side == SideKind::Remote && self.remote_state == RemoteState::Ready);

        let mut header = div()
            .h(px(24.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(1.))
            .pl(px(8.))
            .pr(px(2.))
            .bg(t.bg)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.focus = side;
                    window.focus(&this.key_focus, cx);
                    cx.notify();
                }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(11.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(if side == SideKind::Remote && ready {
                        t.accent
                    } else {
                        t.text
                    })
                    .child(title),
            );

        let up_target: Option<String> = match side {
            SideKind::Local => {
                if data.path.is_empty() {
                    None
                } else {
                    Some(data.parent.clone())
                }
            }
            SideKind::Remote => {
                if data.path.is_empty() || data.path == "/" {
                    None
                } else {
                    Some(data.parent.clone())
                }
            }
        };
        let up_enabled = up_target.is_some();
        let up_next = up_target.clone();
        header = header.child(
            Button::new(if side == SideKind::Local {
                "fp-up-local"
            } else {
                "fp-up-remote"
            })
            .ghost()
            .compact()
            .icon(IconName::ArrowUp)
            .tooltip(tr(lang, "上一级", "Up one level"))
            .disabled(!up_enabled)
            .on_click(cx.listener(move |this, _, _, _cx| {
                let Some(target) = up_next.clone() else {
                    return;
                };
                match side {
                    SideKind::Local => this.list_local(target),
                    SideKind::Remote => this.list_remote(target),
                }
            })),
        );

        let refresh_target = data.path.clone();
        let refresh_enabled = ready && !data.path.is_empty();
        header = header.child(
            Button::new(if side == SideKind::Local {
                "fp-refresh-local"
            } else {
                "fp-refresh-remote"
            })
            .ghost()
            .compact()
            .icon(IconName::RefreshCw)
            .tooltip(tr(lang, "刷新", "Refresh"))
            .disabled(!refresh_enabled)
            .on_click(cx.listener(move |this, _, _, _cx| match side {
                SideKind::Local => this.list_local(refresh_target.clone()),
                SideKind::Remote => this.list_remote(refresh_target.clone()),
            })),
        );

        match side {
            SideKind::Local => {
                header = header.child(
                    Button::new("fp-home-local")
                        .ghost()
                        .compact()
                        .icon(IconName::House)
                        .tooltip(tr(lang, "主目录", "Home directory"))
                        .on_click(cx.listener(|this, _, _, _cx| {
                            this.focus = SideKind::Local;
                            this.list_local(files::local_home());
                        })),
                );
                let shortcuts = files::local_shortcuts();
                let quick_enabled = !shortcuts.is_empty();
                if quick_enabled {
                    let entity = cx.entity();
                    header = header.child(
                        Button::new("fp-quick")
                            .ghost()
                            .compact()
                            .icon(IconName::FolderBookmark)
                            .tooltip(tr(
                                lang,
                                "快速访问：常用文件夹和驱动器",
                                "Quick access: common folders and drives",
                            ))
                            .dropdown_menu(move |menu, _, _| {
                                let mut menu = menu;
                                for entry in shortcuts.clone() {
                                    let panel = entity.clone();
                                    menu =
                                        menu.item(PopupMenuItem::new(entry.name.clone()).on_click(
                                            move |_, _, cx| {
                                                let path = entry.path.clone();
                                                panel.update(cx, |this, cx| {
                                                    this.focus = SideKind::Local;
                                                    this.list_local(path);
                                                    cx.notify();
                                                });
                                            },
                                        ));
                                }
                                menu
                            }),
                    );
                }
            }
            SideKind::Remote => {
                header = header.child(
                    Button::new("fp-root-remote")
                        .ghost()
                        .compact()
                        .icon(IconName::Server)
                        .tooltip(tr(lang, "服务器根目录", "Server root"))
                        .disabled(!ready)
                        .on_click(cx.listener(|this, _, _, _cx| {
                            this.focus = SideKind::Remote;
                            this.list_remote("/".to_string());
                        })),
                );
            }
        }

        header
    }

    fn render_side_path(&self, side: SideKind, cx: &mut Context<Self>) -> gpui::Div {
        let t = self.theme.clone();
        let lang = self.lang;
        let data = self.side(side).clone();

        let path_text: SharedString =
            if side == SideKind::Remote && self.remote_state != RemoteState::Ready {
                "—".into()
            } else if data.path.is_empty() {
                tr(lang, "此电脑", "This PC")
            } else {
                data.path.clone().into()
            };
        let path_box = div()
            .h(px(22.))
            .flex_shrink_0()
            .mx(px(6.))
            .flex()
            .items_center()
            .px(px(6.))
            .rounded(px(4.))
            .bg(t.field)
            .border_1()
            .border_color(t.border_strong)
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .text_size(px(11.))
            .text_color(t.text_2)
            .child(path_text);

        let mut crumb_row = div()
            .h(px(18.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(2.))
            .px(px(6.))
            .overflow_hidden()
            .whitespace_nowrap()
            .text_size(px(10.))
            .text_color(t.text_3);
        for (index, (label, target)) in crumbs(&data.path).into_iter().enumerate() {
            if index > 0 {
                crumb_row = crumb_row.child(div().flex_shrink_0().child("›"));
            }
            let theme = t.clone();
            crumb_row = crumb_row.child(
                div()
                    .id(SharedString::from(format!(
                        "fp-crumb-{}-{}",
                        if side == SideKind::Local { "l" } else { "r" },
                        index
                    )))
                    .flex_shrink_0()
                    .cursor_pointer()
                    .hover(move |s| s.text_color(theme.text))
                    .on_click(cx.listener(move |this, _, _, _cx| {
                        this.focus = side;
                        match side {
                            SideKind::Local => this.list_local(target.clone()),
                            SideKind::Remote => this.list_remote(target.clone()),
                        }
                    }))
                    .child(label),
            );
        }

        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .child(path_box)
            .child(crumb_row)
    }

    fn render_side_list(&self, side: SideKind, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme.clone();
        let lang = self.lang;
        let data = self.side(side).clone();
        let connecting = side == SideKind::Remote && self.remote_state == RemoteState::Connecting;
        let idle_remote = side == SideKind::Remote
            && self.remote_state == RemoteState::Idle
            && self.session.is_none();

        let mut list = div()
            .id(if side == SideKind::Local {
                "fp-local-list"
            } else {
                "fp-remote-list"
            })
            .flex()
            .flex_col()
            .gap(px(1.))
            .py(px(2.))
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    // 点列表空白处：切到这一侧的焦点并清空选区
                    this.focus = side;
                    this.clear_sel(side);
                    this.drag = None;
                    window.focus(&this.key_focus, cx);
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.end_drag(cx)),
            );

        let sel_set: HashSet<&str> = data.sel.iter().map(String::as_str).collect();

        for (index, entry) in data.entries.iter().enumerate() {
            let theme = t.clone();
            let entry = entry.clone();
            let selected = sel_set.contains(entry.path.as_str());
            let bg = if selected { t.bg_selected } else { t.bg };
            let path = entry.path.clone();
            let dir = entry.dir;
            let name = entry.name.clone();
            let size = if entry.dir {
                String::new()
            } else {
                fmt_size(entry.size)
            };
            let date = fmt_date(entry.mtime);
            let icon = if dir {
                IconName::Folder
            } else {
                IconName::File
            };
            list = list.child(
                div()
                    .id(SharedString::from(format!(
                        "fp-row-{}-{}",
                        if side == SideKind::Local { "l" } else { "r" },
                        index
                    )))
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .pl(px(10.))
                    .pr(px(8.))
                    .bg(bg)
                    .hover(move |s| s.bg(theme.bg_hover))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                            // 别让列表容器的「点空白处清空选区」跟着触发
                            cx.stop_propagation();
                            this.focus = side;
                            this.click_row(side, index, ev.modifiers.control, ev.modifiers.shift);
                            if ev.click_count >= 2 {
                                this.drag = None;
                                if dir {
                                    if side == SideKind::Local {
                                        this.list_local(path.clone());
                                    } else {
                                        this.list_remote(path.clone());
                                    }
                                } else if side == SideKind::Remote {
                                    this.download_entry(entry.clone(), cx);
                                }
                            } else {
                                let base = this.side(side).sel.clone();
                                this.drag = Some(DragState {
                                    side,
                                    start: index,
                                    last: index,
                                    base,
                                    additive: ev.modifiers.control,
                                });
                            }
                            window.focus(&this.key_focus, cx);
                            cx.notify();
                        }),
                    )
                    .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                        if ev.pressed_button != Some(MouseButton::Left) {
                            this.end_drag(cx);
                            return;
                        }
                        let Some(drag) = this.drag.clone() else {
                            return;
                        };
                        if drag.side != side {
                            return;
                        }
                        // 框选：起点到这一行整段入选
                        if this.drag_to(&drag, index) {
                            if let Some(state) = this.drag.as_mut() {
                                state.last = index;
                            }
                            cx.notify();
                        }
                    }))
                    .child(
                        div()
                            .w(px(12.))
                            .flex()
                            .justify_center()
                            .text_color(t.text_3)
                            .child(Icon::new(icon).size(px(12.))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(12.))
                            .text_color(t.text)
                            .child(name),
                    )
                    .child(
                        div()
                            .w(px(48.))
                            .flex_shrink_0()
                            .text_size(px(10.))
                            .text_color(t.text_3)
                            .child(size),
                    )
                    .child(
                        div()
                            .w(px(64.))
                            .flex_shrink_0()
                            .text_size(px(10.))
                            .text_color(t.text_3)
                            .child(date),
                    ),
            );
        }

        if data.entries.is_empty() {
            let hint = if connecting {
                tr(lang, "连接中…", "Connecting…")
            } else if idle_remote {
                tr(lang, "没有活动标签页", "No active tab")
            } else if side == SideKind::Remote && self.remote_state != RemoteState::Ready {
                tr(lang, "未连接", "Not connected")
            } else {
                tr(lang, "空目录", "Empty")
            };
            list = list.child(
                div()
                    .px(px(12.))
                    .py(px(10.))
                    .text_size(px(11.))
                    .text_color(t.text_3)
                    .child(hint),
            );
        }

        list
    }
}
