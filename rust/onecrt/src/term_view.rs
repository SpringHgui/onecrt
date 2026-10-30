//! 终端标签页：连接、VTE 解析、键盘/鼠标/选区/IME，以及把网格画到 canvas 上。

use gpui_kit as gpui;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::TermMode;
use async_channel::{unbounded, Sender};
use gpui::{
    canvas, div, fill, px, App, Bounds, Context, ElementInputHandler, EntityInputHandler,
    EventEmitter, FocusHandle, FontStyle, FontWeight, Hsla, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels,
    Point, Render, ScrollDelta, ScrollEvent, SharedString, StrikethroughStyle, Styled, TextAlign,
    TextRun, UTF16Selection, UnderlineStyle, Window,
};
use onecrt_core::terminal::{self, OutEvent};

use crate::term_engine::{
    self, encode_key, encode_mouse, encode_paste, Engine, KeyInput, SelectKind, TermEvent,
};
use crate::theme::{term_rgb, Theme};

#[derive(Clone)]
pub struct ConnectSpec {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub key_path: String,
    pub methods: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConnState {
    Connecting,
    Ready,
    Closed(String),
}

#[derive(Clone, Copy, Debug)]
pub enum Zoom {
    In,
    Out,
    Reset,
}

#[derive(Clone, Debug)]
pub enum TabEvent {
    Title,
    State,
    Reconnect,
    Zoom(Zoom),
    /// 光标位置变化（状态栏用）；按输出节奏节流发出
    Cursor,
    QuickConnect,
}

enum UiMsg {
    Input(Vec<u8>),
    Resize(usize, usize),
}

struct Metrics {
    cell_w: Pixels,
    line_h: Pixels,
    cols: usize,
    rows: usize,
}

pub struct TerminalView {
    pub id: String,
    pub spec: ConnectSpec,
    pub title: SharedString,
    pub state: ConnState,
    engine: Rc<RefCell<Engine>>,
    metrics: Rc<RefCell<Metrics>>,
    focus: FocusHandle,
    theme: Theme,
    out_tx: Sender<UiMsg>,
    _pump: gpui::Task<()>,
    ime_preedit: String,
    bounds: Rc<RefCell<Bounds<Pixels>>>,
    pub font_size: f32,
    dragging: bool,
    multi_click: bool,
    mouse_button: Option<u8>,
    mouse_cell: Option<(usize, usize)>,
    last_focused: Option<bool>,
    /// 状态栏光标位置：只在位置变化且距上次上报超过间隔时发事件
    last_cursor: Option<(usize, usize)>,
    last_cursor_emit: std::time::Instant,
    cursor_pending: bool,
    backend: Arc<onecrt_core::Backend>,
}

impl EventEmitter<TabEvent> for TerminalView {}

impl TerminalView {
    pub fn new(
        id: String,
        spec: ConnectSpec,
        backend: Arc<onecrt_core::Backend>,
        theme: Theme,
        font_size: f32,
        cx: &mut Context<Self>,
    ) -> Self {
        let engine = Rc::new(RefCell::new(Engine::new(80, 24)));
        let metrics = Rc::new(RefCell::new(Metrics {
            cell_w: px(8.),
            line_h: px(18.),
            cols: 80,
            rows: 24,
        }));
        let (in_tx, in_rx) = unbounded::<UiMsg>();
        let (out_tx, out_rx) = unbounded::<OutEvent>();
        let (open_tx, open_rx) = unbounded::<Result<(), String>>();

        let term_id = id.clone();
        let sink: terminal::Sink = Arc::new(move |ev| {
            let _ = out_tx.try_send(ev);
        });
        let args = terminal::OpenArgs {
            id: term_id.clone(),
            host: spec.host.clone(),
            port: spec.port,
            username: spec.username.clone(),
            password: spec.password.clone(),
            key_path: spec.key_path.clone(),
            methods: spec.methods.clone(),
            cols: 80,
            rows: 24,
            keepalive_secs: None,
        };
        {
            let mgr = backend.terminals.clone();
            backend.spawn(async move {
                let r = terminal::open(&mgr, args, sink).await;
                let _ = open_tx.try_send(r);
            });
        }
        {
            let mgr = backend.terminals.clone();
            let term_id = term_id.clone();
            backend.spawn(async move {
                while let Ok(msg) = in_rx.recv().await {
                    match msg {
                        UiMsg::Input(bytes) => {
                            let text = String::from_utf8_lossy(&bytes).into_owned();
                            if terminal::send_input(&mgr, term_id.clone(), text)
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        UiMsg::Resize(cols, rows) => {
                            let _ =
                                terminal::resize(&mgr, term_id.clone(), cols as u16, rows as u16)
                                    .await;
                        }
                    }
                }
            });
        }

        let engine_for_pump = engine.clone();
        let pump = cx.spawn(async move |this, cx| {
            if let Ok(Err(e)) = open_rx.recv().await {
                let _ = this.update(cx, |this, cx| {
                    this.state = ConnState::Closed(e);
                    cx.emit(TabEvent::State);
                    cx.notify();
                });
                return;
            }
            while let Ok(ev) = out_rx.recv().await {
                let updated = this.update(cx, |this, cx| {
                    this.on_output(ev, &engine_for_pump, cx);
                });
                if updated.is_err() {
                    break;
                }
            }
        });

        let weak = cx.weak_entity();
        let mut this = Self {
            id,
            spec,
            title: SharedString::default(),
            state: ConnState::Connecting,
            engine,
            metrics,
            focus: cx.focus_handle(),
            theme,
            out_tx: in_tx,
            _pump: pump,
            ime_preedit: String::new(),
            bounds: Rc::new(RefCell::new(Bounds::default())),
            font_size,
            dragging: false,
            multi_click: false,
            mouse_button: None,
            mouse_cell: None,
            last_focused: None,
            last_cursor: None,
            last_cursor_emit: std::time::Instant::now(),
            cursor_pending: false,
            backend,
        };
        this.title = this.spec.host.clone().into();
        let _ = weak;
        this
    }

    fn on_output(&mut self, ev: OutEvent, engine: &Rc<RefCell<Engine>>, cx: &mut Context<Self>) {
        match ev {
            OutEvent::Ready => {
                self.state = ConnState::Ready;
                cx.emit(TabEvent::State);
            }
            OutEvent::Output { data } => {
                engine.borrow_mut().process(data.as_bytes());
                let events = engine.borrow_mut().drain_events();
                for e in events {
                    self.handle_term_event(e, cx);
                }
            }
            OutEvent::Closed { reason } => {
                engine
                    .borrow_mut()
                    .process(b"\r\n\x1b[2m\xe6\x8c\x89\xe5\x9b\x9e\xe8\xbd\xa6\xe9\x87\x8d\xe8\xbf\x9e / press Enter to reconnect\x1b[0m\r\n");
                self.state = ConnState::Closed(reason);
                cx.emit(TabEvent::State);
            }
        }
        self.emit_cursor(cx);
        cx.notify();
    }

    /// 输出可能一次刷很多行，光标上报按 150ms 节流，避免把整窗重绘打成风暴。
    /// 被节流掉的变化补一个尾事件，保证最后一次位置不丢。
    fn emit_cursor(&mut self, cx: &mut Context<Self>) {
        const GAP: std::time::Duration = std::time::Duration::from_millis(150);
        let pos = self.cursor_pos();
        if pos == self.last_cursor {
            return;
        }
        if self.last_cursor.is_some() && self.last_cursor_emit.elapsed() < GAP {
            if !self.cursor_pending {
                self.cursor_pending = true;
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(GAP).await;
                    let _ = this.update(cx, |this, cx| {
                        this.cursor_pending = false;
                        let pos = this.cursor_pos();
                        if pos != this.last_cursor {
                            this.last_cursor = pos;
                            this.last_cursor_emit = std::time::Instant::now();
                            cx.emit(TabEvent::Cursor);
                        }
                    });
                })
                .detach();
            }
            return;
        }
        self.last_cursor = pos;
        self.last_cursor_emit = std::time::Instant::now();
        cx.emit(TabEvent::Cursor);
    }

    fn handle_term_event(&mut self, event: TermEvent, cx: &mut Context<Self>) {
        match event {
            TermEvent::ClipboardStore(text) => {
                if let Ok(mut clip) = arboard::Clipboard::new() {
                    let _ = clip.set_text(text);
                }
            }
            TermEvent::ClipboardLoad => {
                if let Ok(mut clip) = arboard::Clipboard::new() {
                    if let Ok(text) = clip.get_text() {
                        self.write_bytes(encode_paste(&text, self.mode()));
                    }
                }
            }
            TermEvent::PtyWrite(text) => self.write_bytes(text.into_bytes()),
            TermEvent::Title(title) => {
                if !title.is_empty() {
                    self.title = title.into();
                    cx.emit(TabEvent::Title);
                }
            }
            TermEvent::Bell => {}
        }
    }

    fn mode(&self) -> TermMode {
        self.engine.borrow().mode()
    }

    fn write_bytes(&self, bytes: Vec<u8>) {
        let _ = self.out_tx.try_send(UiMsg::Input(bytes));
    }

    fn cell_at_point(&self, pos: Point<Pixels>, bounds: Bounds<Pixels>) -> (usize, usize) {
        let m = self.metrics.borrow();
        let x = (pos.x - bounds.origin.x).max(px(0.));
        let y = (pos.y - bounds.origin.y).max(px(0.));
        let col = (x / m.cell_w).floor() as usize;
        let row = (y / m.line_h).floor() as usize;
        (
            row.min(m.rows.saturating_sub(1)),
            col.min(m.cols.saturating_sub(1)),
        )
    }

    fn copy_selection(&self) {
        if let Some(text) = self.engine.borrow().selection_text() {
            if let Ok(mut clip) = arboard::Clipboard::new() {
                let _ = clip.set_text(text);
            }
        }
    }

    fn paste_clipboard(&self) {
        if let Ok(mut clip) = arboard::Clipboard::new() {
            if let Ok(text) = clip.get_text() {
                self.engine.borrow_mut().scroll_to_bottom();
                self.write_bytes(encode_paste(&text, self.mode()));
            }
        }
    }

    pub fn focus_terminal(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
    }

    /// 命令库发来的整条命令：按回车提交，和手敲一致。
    pub fn send_command(&self, text: &str) {
        self.engine.borrow_mut().scroll_to_bottom();
        self.write_bytes(format!("{text}\r").into_bytes());
    }

    pub fn copy(&self) {
        self.copy_selection();
    }

    pub fn paste(&self) {
        self.paste_clipboard();
    }

    pub fn clear(&self) {
        self.write_bytes(b"\x0c".to_vec());
    }

    pub fn interrupt(&self) {
        self.write_bytes(b"\x03".to_vec());
    }

    pub fn disconnect(&self) {
        let backend = self.backend.clone();
        let mgr = backend.terminals.clone();
        let id = self.id.clone();
        backend.spawn(async move {
            terminal::close(mgr, id).await;
        });
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let ctrl = ks.modifiers.control;
        let alt = ks.modifiers.alt;
        let shift = ks.modifiers.shift;
        let key = ks.key.as_str();

        if key.eq_ignore_ascii_case("f11") {
            window.toggle_fullscreen();
            cx.stop_propagation();
            return;
        }

        // 和 SecureCRT 一致：Alt+R 直接开快速连接
        if alt && !ctrl && key.eq_ignore_ascii_case("r") {
            cx.emit(TabEvent::QuickConnect);
            cx.stop_propagation();
            return;
        }

        // 复制粘贴和原版一致：Ctrl+V / Ctrl+Shift+V 都是粘贴，Ctrl+Shift+C 纯复制；
        // Ctrl+C 有选区时复制，没有选区才交给远端发 SIGINT。
        if ctrl && !alt {
            if key.eq_ignore_ascii_case("v") {
                self.paste_clipboard();
                cx.stop_propagation();
                return;
            }
            if key.eq_ignore_ascii_case("c") && (shift || self.engine.borrow().has_selection()) {
                self.copy_selection();
                cx.stop_propagation();
                return;
            }
        }
        if ctrl && !alt {
            let zoom = match key {
                "=" | "+" => Some(Zoom::In),
                "-" | "_" => Some(Zoom::Out),
                "0" => Some(Zoom::Reset),
                _ => None,
            };
            if let Some(zoom) = zoom {
                cx.emit(TabEvent::Zoom(zoom));
                cx.stop_propagation();
                return;
            }
        }
        if ctrl && !alt && !shift && matches!(key, "insert") {
            self.copy_selection();
            cx.stop_propagation();
            return;
        }
        // 和 Linux 终端一致：Shift+Insert 粘贴，Shift+PageUp/PgDn/Home/End/↑/↓ 翻回滚
        if !ctrl && !alt {
            if key == "insert" && shift {
                self.paste_clipboard();
                cx.stop_propagation();
                return;
            }
            if shift {
                match key {
                    "pageup" | "page_up" => {
                        self.engine.borrow_mut().scroll_page(-1);
                        cx.notify();
                        cx.stop_propagation();
                        return;
                    }
                    "pagedown" | "page_down" => {
                        self.engine.borrow_mut().scroll_page(1);
                        cx.notify();
                        cx.stop_propagation();
                        return;
                    }
                    "home" => {
                        self.engine.borrow_mut().scroll_to_top();
                        cx.notify();
                        cx.stop_propagation();
                        return;
                    }
                    "end" => {
                        self.engine.borrow_mut().scroll_to_bottom();
                        cx.notify();
                        cx.stop_propagation();
                        return;
                    }
                    "up" | "arrow_up" => {
                        self.engine.borrow_mut().scroll(-1);
                        cx.notify();
                        cx.stop_propagation();
                        return;
                    }
                    "down" | "arrow_down" => {
                        self.engine.borrow_mut().scroll(1);
                        cx.notify();
                        cx.stop_propagation();
                        return;
                    }
                    _ => {}
                }
            }
        }

        let ch = ks.key_char.as_ref().and_then(|s| s.chars().next());
        if matches!(self.state, ConnState::Closed(_)) && key == "enter" {
            cx.emit(TabEvent::Reconnect);
            cx.stop_propagation();
            return;
        }
        let input = KeyInput {
            key,
            ch,
            ctrl,
            alt,
            shift,
        };
        if let Some(bytes) = encode_key(&input, self.mode()) {
            // 打字即回到最新一行，和 Linux 终端一致
            self.engine.borrow_mut().scroll_to_bottom();
            self.write_bytes(bytes);
        }
        cx.stop_propagation();
    }

    fn on_mouse_down(
        &mut self,
        ev: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
        bounds: Bounds<Pixels>,
    ) {
        self.focus.focus(window, cx);
        let (row, col) = self.cell_at_point(ev.position, bounds);
        let mode = self.mode();

        if mode.contains(TermMode::MOUSE_MODE) {
            if ev.button == MouseButton::Left
                || ev.button == MouseButton::Middle
                || ev.button == MouseButton::Right
            {
                let button = match ev.button {
                    MouseButton::Left => 0,
                    MouseButton::Middle => 1,
                    _ => 2,
                };
                self.mouse_button = Some(button);
                self.mouse_cell = Some((row, col));
                let bytes = encode_mouse(
                    row,
                    col,
                    button,
                    true,
                    false,
                    ev.modifiers.control,
                    ev.modifiers.alt,
                    ev.modifiers.shift,
                    mode.contains(TermMode::SGR_MOUSE),
                );
                self.write_bytes(bytes);
                cx.stop_propagation();
            }
            return;
        }

        match ev.button {
            MouseButton::Left => {
                self.dragging = false;
                self.multi_click = ev.click_count >= 2;
                let kind = match ev.click_count {
                    0 | 1 => SelectKind::Char,
                    2 => SelectKind::Word,
                    _ => SelectKind::Line,
                };
                self.engine.borrow_mut().clear_selection();
                self.engine.borrow_mut().start_selection(row, col, kind);
                cx.notify();
            }
            MouseButton::Right => {
                // 有选区先复制，再粘贴：旧版右键就是这个语义
                self.copy_selection();
                self.engine.borrow_mut().clear_selection();
                self.paste_clipboard();
                cx.notify();
            }
            // Linux 终端的中键粘贴
            MouseButton::Middle => {
                self.paste_clipboard();
                cx.notify();
            }
            _ => {}
        }
    }

    fn on_mouse_move(
        &mut self,
        ev: &MouseMoveEvent,
        cx: &mut Context<Self>,
        bounds: Bounds<Pixels>,
    ) {
        let mode = self.mode();
        if mode.contains(TermMode::MOUSE_MODE) {
            let pressed = self.mouse_button.is_some();
            let any_motion = mode.contains(TermMode::MOUSE_MOTION);
            let drag_motion = pressed && mode.contains(TermMode::MOUSE_DRAG);
            if any_motion || drag_motion {
                let (row, col) = self.cell_at_point(ev.position, bounds);
                if self.mouse_cell != Some((row, col)) {
                    self.mouse_cell = Some((row, col));
                    let bytes = encode_mouse(
                        row,
                        col,
                        self.mouse_button.unwrap_or(3),
                        true,
                        true,
                        ev.modifiers.control,
                        ev.modifiers.alt,
                        ev.modifiers.shift,
                        mode.contains(TermMode::SGR_MOUSE),
                    );
                    self.write_bytes(bytes);
                }
            }
            return;
        }
        if ev.pressed_button == Some(MouseButton::Left) {
            let (row, col) = self.cell_at_point(ev.position, bounds);
            self.dragging = true;
            self.engine.borrow_mut().update_selection(row, col);
            cx.notify();
        }
    }

    fn on_mouse_up(&mut self, ev: &MouseUpEvent, cx: &mut Context<Self>, bounds: Bounds<Pixels>) {
        let mode = self.mode();
        if mode.contains(TermMode::MOUSE_MODE) {
            if let Some(button) = self.mouse_button.take() {
                let (row, col) = self.cell_at_point(ev.position, bounds);
                let bytes = encode_mouse(
                    row,
                    col,
                    button,
                    false,
                    false,
                    ev.modifiers.control,
                    ev.modifiers.alt,
                    ev.modifiers.shift,
                    mode.contains(TermMode::SGR_MOUSE),
                );
                self.write_bytes(bytes);
            }
            self.mouse_cell = None;
            cx.stop_propagation();
            return;
        }
        // 单击只是定位光标，不该留下一个格子的“选区”；双击/三击的按词/按行选区要保留
        if ev.button == MouseButton::Left && !self.dragging && !self.multi_click {
            self.engine.borrow_mut().clear_selection();
            cx.notify();
        }
        self.dragging = false;
        self.multi_click = false;
    }

    fn on_scroll(&mut self, ev: &ScrollEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let lines = match ev.delta {
            ScrollDelta::Lines(p) => p.y,
            // 像素滚动（触控板）向下为正，取反后和历史区的滚动方向一致
            ScrollDelta::Pixels(p) => (-p.y / self.metrics.borrow().line_h).round(),
        };
        let lines = lines as i32;
        if lines == 0 {
            return;
        }
        let mode = self.mode();
        if mode.contains(TermMode::MOUSE_MODE) {
            let b = if lines > 0 { 64 } else { 65 };
            let bytes = encode_mouse(
                0,
                0,
                b,
                true,
                false,
                ev.modifiers.control,
                ev.modifiers.alt,
                ev.modifiers.shift,
                mode.contains(TermMode::SGR_MOUSE),
            );
            self.write_bytes(bytes);
        } else if mode.contains(TermMode::ALT_SCREEN) {
            // 备用屏（vim/less/man）没有本地回滚，滚轮改成方向键，和 Linux 终端一致
            let app = mode.contains(TermMode::APP_CURSOR);
            // 向上滚（lines > 0）= 上方向键
            let seq: &[u8] = if lines > 0 {
                if app {
                    b"\x1bOA"
                } else {
                    b"\x1b[A"
                }
            } else if app {
                b"\x1bOB"
            } else {
                b"\x1b[B"
            };
            let count = (lines.unsigned_abs() as usize).min(15);
            let mut out = Vec::with_capacity(count * seq.len());
            for _ in 0..count {
                out.extend_from_slice(seq);
            }
            self.write_bytes(out);
        } else {
            // 一格滚轮 3 行，和 xterm.js 默认的滚动灵敏度一致
            self.engine.borrow_mut().scroll(lines);
            cx.notify();
        }
        cx.stop_propagation();
    }

    pub fn set_theme(&mut self, theme: Theme, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    pub fn set_font_size(&mut self, size: f32, window: &mut Window, cx: &mut Context<Self>) {
        if (size - self.font_size).abs() < f32::EPSILON {
            return;
        }
        self.font_size = size;
        let bounds = *self.bounds.borrow();
        if bounds.size.width > px(0.) && bounds.size.height > px(0.) {
            let (cell_w, line_h, cols, rows) = cell_metrics(
                &self.theme,
                size,
                bounds.size.width,
                bounds.size.height,
                window,
            );
            let mut m = self.metrics.borrow_mut();
            m.cell_w = cell_w;
            m.line_h = line_h;
            if (m.cols, m.rows) != (cols, rows) {
                m.cols = cols;
                m.rows = rows;
                self.engine.borrow_mut().resize(cols, rows);
                let _ = self.out_tx.try_send(UiMsg::Resize(cols, rows));
            }
        }
        cx.notify();
    }

    pub fn grid_size(&self) -> (usize, usize) {
        let m = self.metrics.borrow();
        (m.cols, m.rows)
    }

    /// 屏幕坐标（0 起），状态栏显示时再转 1 起；滚到历史区时返回 None。
    pub fn cursor_pos(&self) -> Option<(usize, usize)> {
        self.engine
            .borrow()
            .cursor()
            .map(|(row, col, _)| (row, col))
    }
}

fn cell_metrics(
    theme: &Theme,
    font_size: f32,
    width: Pixels,
    height: Pixels,
    window: &mut Window,
) -> (Pixels, Pixels, usize, usize) {
    let font = theme.mono_font.clone();
    let font_id = window.text_system().resolve_font(&font);
    let cell_w = window
        .text_system()
        .advance(font_id, px(font_size), 'M')
        .map(|s| s.width)
        .unwrap_or(px(8.));
    let cell_w = cell_w.max(px(4.)).ceil();
    // 14px 字体约 16px 行距，和原版 xterm.js 的网格密度一致
    let line_h = px((font_size * 1.15).round());
    let cols = ((width / cell_w).floor() as usize).max(2);
    let rows = ((height / line_h).floor() as usize).max(2);
    (cell_w, line_h, cols, rows)
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let engine = self.engine.clone();
        let metrics = self.metrics.clone();
        let theme = self.theme.clone();
        let font_size = self.font_size;
        let preedit = self.ime_preedit.clone();
        let focused = self.focus.is_focused(window);
        if self.last_focused != Some(focused) {
            self.last_focused = Some(focused);
            // 程序开了 1004 焦点上报（vim/tmux 会用到）才发
            if matches!(self.state, ConnState::Ready)
                && self.mode().contains(TermMode::FOCUS_IN_OUT)
            {
                self.write_bytes(if focused {
                    b"\x1b[I".to_vec()
                } else {
                    b"\x1b[O".to_vec()
                });
            }
        }
        let bounds_rc = self.bounds.clone();
        let entity = cx.entity();
        let focus = self.focus.clone();
        let out_tx = self.out_tx.clone();

        div()
            .size_full()
            .relative()
            .bg(theme.term_bg)
            .key_context("Terminal")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                this.on_key_down(ev, window, cx);
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    let b = *this.bounds.borrow();
                    this.on_mouse_down(ev, window, cx, b);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    let b = *this.bounds.borrow();
                    this.on_mouse_down(ev, window, cx, b);
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    let b = *this.bounds.borrow();
                    this.on_mouse_down(ev, window, cx, b);
                }),
            )
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _window, cx| {
                let b = *this.bounds.borrow();
                this.on_mouse_move(ev, cx, b);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseUpEvent, _window, cx| {
                    let b = *this.bounds.borrow();
                    this.on_mouse_up(ev, cx, b);
                }),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|this, ev: &MouseUpEvent, _window, cx| {
                    let b = *this.bounds.borrow();
                    this.on_mouse_up(ev, cx, b);
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|this, ev: &MouseUpEvent, _window, cx| {
                    let b = *this.bounds.borrow();
                    this.on_mouse_up(ev, cx, b);
                }),
            )
            .on_scroll(cx.listener(|this, ev: &ScrollEvent, window, cx| {
                this.on_scroll(ev, window, cx);
            }))
            .child(
                canvas(
                    move |bounds, _window, _cx| {
                        *bounds_rc.borrow_mut() = bounds;
                    },
                    move |bounds, _prepaint, window, cx| {
                        {
                            let (cell_w, line_h, cols, rows) = cell_metrics(
                                &theme,
                                font_size,
                                bounds.size.width,
                                bounds.size.height,
                                window,
                            );
                            let mut m = metrics.borrow_mut();
                            m.cell_w = cell_w;
                            m.line_h = line_h;
                            if (m.cols, m.rows) != (cols, rows) {
                                m.cols = cols;
                                m.rows = rows;
                                engine.borrow_mut().resize(cols, rows);
                                // 远端 PTY 也要跟着改窗口尺寸，否则 vim/top 按 80x24 排版
                                let _ = out_tx.try_send(UiMsg::Resize(cols, rows));
                                drop(m);
                                // 状态栏的列×行要跟着视口走，本帧结束后再通知父视图
                                let entity = entity.clone();
                                window.defer(cx, move |_window, cx| {
                                    let _ = entity.update(cx, |_this, cx| cx.emit(TabEvent::State));
                                });
                            }
                        }
                        // IME 输入句柄只能在 paint 阶段登记（prepaint 会 debug_assert 失败）
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, entity.clone()),
                            cx,
                        );
                        paint_grid(
                            &engine, &metrics, &theme, font_size, focused, &preedit, bounds,
                            window, cx,
                        );
                    },
                )
                .size_full(),
            )
    }
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _range: std::ops::Range<usize>,
        _adjusted_range: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        None
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        (!self.ime_preedit.is_empty()).then(|| 0..self.ime_preedit.len())
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.ime_preedit.is_empty() {
            self.ime_preedit.clear();
            cx.notify();
        }
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<std::ops::Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !text.is_empty() {
            self.engine.borrow_mut().scroll_to_bottom();
            self.write_bytes(text.as_bytes().to_vec());
        }
        self.ime_preedit.clear();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<std::ops::Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<std::ops::Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ime_preedit = new_text.to_string();
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(element_bounds)
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(0)
    }
}

fn paint_grid(
    engine: &Rc<RefCell<Engine>>,
    metrics: &Rc<RefCell<Metrics>>,
    theme: &Theme,
    font_size: f32,
    focused: bool,
    preedit: &str,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let m = metrics.borrow();
    let cell_w = m.cell_w;
    let line_h = m.line_h;
    let cols = m.cols;
    let rows = m.rows;
    let engine = engine.borrow();

    window.paint_quad(fill(bounds, theme.term_bg));

    let selection = engine.selection_screen();
    let cursor = engine.cursor();
    let colors = engine.term().colors();
    let palette = theme.ansi;
    let font_id = window.text_system().resolve_font(&theme.mono_font);
    let _ = font_id;

    for row in 0..rows {
        let top = bounds.origin.y + line_h * row as f32;
        let mut col = 0usize;
        while col < cols {
            let Some(cell) = engine.cell_at(row, col) else {
                col += 1;
                continue;
            };
            let flags = cell.flags;
            let wide = flags.contains(Flags::WIDE_CHAR);
            let span = if wide { 2 } else { 1 };

            let mut fg = term_engine::named_color(
                colors,
                &palette,
                rgb_of(theme.term_fg),
                rgb_of(theme.term_bg),
                rgb_of(theme.term_cursor),
                cell.fg,
            );
            let mut bg = term_engine::named_color(
                colors,
                &palette,
                rgb_of(theme.term_fg),
                rgb_of(theme.term_bg),
                rgb_of(theme.term_cursor),
                cell.bg,
            );
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            let fg_hsla = term_rgb(fg);
            let bg_hsla = term_rgb(bg);

            // 背景：默认底不画，省掉整屏的矩形
            if bg != rgb_of(theme.term_bg) {
                window.paint_quad(fill(
                    Bounds::new(
                        Point::new(bounds.origin.x + cell_w * col as f32, top),
                        gpui::size(cell_w * span as f32, line_h),
                    ),
                    bg_hsla,
                ));
            }
            if let Some(sel) = selection {
                if sel.contains(row as i32, col) {
                    window.paint_quad(fill(
                        Bounds::new(
                            Point::new(bounds.origin.x + cell_w * col as f32, top),
                            gpui::size(cell_w * span as f32, line_h),
                        ),
                        theme.term_selection,
                    ));
                }
            }

            // 文本：宽字符单独居中，普通字符按 run 合并；空格不画也能保留 bg
            if cell.c != ' ' && cell.c != '\0' && !flags.contains(Flags::HIDDEN) {
                let mut text = String::new();
                text.push(cell.c);
                if let Some(zero) = cell.zerowidth() {
                    text.extend(zero.iter());
                }
                let mut font = theme.mono_font.clone();
                if flags.intersects(Flags::BOLD | Flags::BOLD_ITALIC) {
                    font.weight = FontWeight::BOLD;
                }
                if flags.intersects(Flags::ITALIC | Flags::BOLD_ITALIC) {
                    font.style = FontStyle::Italic;
                }
                let underline = flags
                    .intersects(Flags::ALL_UNDERLINES)
                    .then(|| UnderlineStyle {
                        thickness: px(1.),
                        color: Some(fg_hsla),
                        wavy: flags.contains(Flags::UNDERCURL),
                    });
                let strikethrough = flags
                    .contains(Flags::STRIKEOUT)
                    .then(|| StrikethroughStyle {
                        thickness: px(1.),
                        color: Some(fg_hsla),
                    });
                let run = TextRun {
                    len: text.len(),
                    font,
                    color: fg_hsla,
                    background_color: None,
                    underline,
                    strikethrough,
                };
                let line =
                    window
                        .text_system()
                        .shape_line(text.into(), px(font_size), &[run], None);
                let origin = Point::new(bounds.origin.x + cell_w * col as f32, top);
                if wide {
                    let _ = line.paint(
                        origin,
                        line_h,
                        TextAlign::Center,
                        Some(cell_w * 2.),
                        window,
                        cx,
                    );
                } else {
                    let _ = line.paint(origin, line_h, TextAlign::Left, None, window, cx);
                }
            }

            col += span;
        }
    }

    // 光标：方块 + 反色字符；有输入法编码时把编码本身贴着光标画出来
    if let Some((row, col, show)) = cursor {
        if show || focused {
            let top = bounds.origin.y + line_h * row as f32;
            let x = bounds.origin.x + cell_w * col as f32;
            if focused {
                window.paint_quad(fill(
                    Bounds::new(Point::new(x, top), gpui::size(cell_w, line_h)),
                    theme.term_cursor,
                ));
                if let Some(cell) = engine.cell_at(row, col) {
                    if cell.c != ' ' && cell.c != '\0' {
                        let text = SharedString::from(cell.c.to_string());
                        let run = TextRun {
                            len: text.len(),
                            font: theme.mono_font.clone(),
                            color: theme.term_bg,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        };
                        let line =
                            window
                                .text_system()
                                .shape_line(text, px(font_size), &[run], None);
                        let _ = line.paint(
                            Point::new(x, top),
                            line_h,
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    }
                }
            } else {
                window.paint_quad(gpui::quad(
                    Bounds::new(Point::new(x, top), gpui::size(cell_w, line_h)),
                    px(0.),
                    gpui::transparent_black(),
                    px(1.),
                    theme.term_fg,
                    gpui::BorderStyle::Solid,
                ));
            }
            if !preedit.is_empty() {
                let text = SharedString::from(preedit.to_string());
                let run = TextRun {
                    len: text.len(),
                    font: theme.mono_font.clone(),
                    color: theme.term_fg,
                    background_color: None,
                    underline: Some(UnderlineStyle {
                        thickness: px(1.),
                        color: Some(theme.term_fg),
                        wavy: false,
                    }),
                    strikethrough: None,
                };
                let line = window
                    .text_system()
                    .shape_line(text, px(font_size), &[run], None);
                let _ = line.paint(
                    Point::new(x, top),
                    line_h,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
        }
    }
}

fn rgb_of(color: Hsla) -> alacritty_terminal::vte::ansi::Rgb {
    let rgba: gpui::Rgba = color.into();
    alacritty_terminal::vte::ansi::Rgb {
        r: (rgba.r * 255.) as u8,
        g: (rgba.g * 255.) as u8,
        b: (rgba.b * 255.) as u8,
    }
}
