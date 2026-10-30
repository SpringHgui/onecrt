//! 终端仿真层：alacritty 的 VTE 状态机 + 按键编码。
//!
//! 这一层不依赖 GUI：界面把 SSH 通道的字节喂进来、把按键交进去编码，
//! 渲染时读 `term()` 暴露的网格。

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{NamedColor, Processor, Rgb};
use std::sync::{Arc, Mutex};

/// alacritty 的 TermSize 在 test 模块里，公开发布的 Dimensions 要实现三个方法
struct GridSize {
    columns: usize,
    screen_lines: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

/// 终端自己需要的非文本事件：界面据此写剪贴板、改标题、回写输入。
#[derive(Clone, Debug)]
pub enum TermEvent {
    ClipboardStore(String),
    ClipboardLoad,
    PtyWrite(String),
    Title(String),
    Bell,
}

#[derive(Default)]
pub struct Listener {
    events: Arc<Mutex<Vec<TermEvent>>>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let mapped = match event {
            Event::ClipboardStore(_, text) => Some(TermEvent::ClipboardStore(text)),
            Event::ClipboardLoad(_, _) => Some(TermEvent::ClipboardLoad),
            Event::PtyWrite(text) => Some(TermEvent::PtyWrite(text)),
            Event::Title(title) => Some(TermEvent::Title(title)),
            Event::ResetTitle => Some(TermEvent::Title(String::new())),
            Event::Bell => Some(TermEvent::Bell),
            _ => None,
        };
        if let Some(ev) = mapped {
            self.events.lock().unwrap().push(ev);
        }
    }
}

pub struct Engine {
    term: Term<Listener>,
    parser: Processor,
    events: Arc<Mutex<Vec<TermEvent>>>,
    cols: usize,
    rows: usize,
}

impl Engine {
    pub fn new(cols: usize, rows: usize) -> Self {
        let cols = cols.max(2);
        let rows = rows.max(2);
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut config = alacritty_terminal::term::Config::default();
        // 回滚：比 xterm.js 默认的 1000 行宽裕，长日志也翻得回去
        config.scrolling_history = 10_000;
        let term = Term::new(
            config,
            &GridSize {
                columns: cols,
                screen_lines: rows,
            },
            Listener {
                events: events.clone(),
            },
        );
        Self {
            term,
            parser: Processor::new(),
            events,
            cols,
            rows,
        }
    }

    pub fn process(&mut self, data: &[u8]) {
        self.parser.advance(&mut self.term, data);
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        let cols = cols.max(2);
        let rows = rows.max(2);
        if (cols, rows) == (self.cols, self.rows) {
            return;
        }
        self.cols = cols;
        self.rows = rows;
        self.term.resize(GridSize {
            columns: cols,
            screen_lines: rows,
        });
    }

    pub fn term(&self) -> &Term<Listener> {
        &self.term
    }

    pub fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    pub fn scroll(&mut self, delta: i32) {
        self.term.scroll_display(Scroll::Delta(delta));
    }

    pub fn scroll_page(&mut self, dir: i32) {
        self.term.scroll_display(if dir > 0 {
            Scroll::PageDown
        } else {
            Scroll::PageUp
        });
    }

    pub fn scroll_to_top(&mut self) {
        self.term.scroll_display(Scroll::Top);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    pub fn drain_events(&mut self) -> Vec<TermEvent> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }

    pub fn cell_at(&self, row: usize, col: usize) -> Option<&alacritty_terminal::term::cell::Cell> {
        let point = Point::new(
            Line(row as i32 - self.display_offset() as i32),
            Column(col.min(self.cols - 1)),
        );
        let grid = self.term.grid();
        // 历史区/视口外不取，Index 不做越界钳制
        let line = point.line.0;
        let top = -(grid.display_offset() as i32);
        let bottom = self.rows as i32 - grid.display_offset() as i32;
        (top..bottom).contains(&line).then(|| &grid[point])
    }

    /// 屏幕坐标（0 起）转网格点，供选择与鼠标上报使用
    pub fn point_at(&self, row: usize, col: usize) -> Point {
        Point::new(
            Line(row as i32 - self.display_offset() as i32),
            Column(col.min(self.cols.saturating_sub(1))),
        )
    }

    pub fn start_selection(&mut self, row: usize, col: usize, kind: SelectKind) {
        let point = self.point_at(row, col);
        let ty = match kind {
            SelectKind::Char => SelectionType::Simple,
            SelectKind::Word => SelectionType::Semantic,
            SelectKind::Line => SelectionType::Lines,
        };
        self.term.selection = Some(Selection::new(ty, point, Side::Left));
    }

    pub fn update_selection(&mut self, row: usize, col: usize) {
        let point = self.point_at(row, col);
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, Side::Right);
        }
    }

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    pub fn has_selection(&self) -> bool {
        self.term.selection.is_some()
    }

    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string().filter(|s| !s.is_empty())
    }

    pub fn cursor(&self) -> Option<(usize, usize, bool)> {
        let grid = self.term.grid();
        let point = grid.cursor.point;
        // 选区内或失焦时通常不画光标；滚动到历史里也隐藏
        if self.term.selection.is_some() {
            return None;
        }
        let row = point.line.0 + grid.display_offset() as i32;
        if !(0..self.rows as i32).contains(&row) {
            return None;
        }
        Some((
            row as usize,
            point.column.0,
            self.term.mode().contains(TermMode::SHOW_CURSOR),
        ))
    }

    /// 选区换算到屏幕坐标（0 起）。渲染只关心视口，历史区行号为负，画的时候裁掉。
    pub fn selection_screen(&self) -> Option<ScreenSelection> {
        let range = self.term.selection.as_ref()?.to_range(&self.term)?;
        let offset = self.term.grid().display_offset() as i32;
        Some(ScreenSelection {
            start: (range.start.line.0 + offset, range.start.column.0),
            end: (range.end.line.0 + offset, range.end.column.0),
            block: range.is_block,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectKind {
    Char,
    Word,
    Line,
}

#[derive(Clone, Copy, Debug)]
pub struct ScreenSelection {
    pub start: (i32, usize),
    pub end: (i32, usize),
    pub block: bool,
}

impl ScreenSelection {
    /// (row, col) 是否落在选区内；行号是屏幕坐标
    pub fn contains(&self, row: i32, col: usize) -> bool {
        let (sr, sc) = self.start;
        let (er, ec) = self.end;
        if self.block {
            return row >= sr.min(er)
                && row <= sr.max(er)
                && col >= sc.min(ec)
                && col <= sc.max(ec);
        }
        if row < sr || row > er {
            return false;
        }
        if sr == er {
            return col >= sc && col <= ec;
        }
        if row == sr {
            return col >= sc;
        }
        if row == er {
            return col <= ec;
        }
        true
    }
}

/// alacritty 的 Colors 默认只有被 OSC 4/10/11 改过的槽位，
/// 其余要由界面主题补；这里把「命名色 → 主题调色板」的映射收在一处。
pub fn named_color(
    colors: &alacritty_terminal::term::color::Colors,
    palette: &[Rgb; 16],
    fg: Rgb,
    bg: Rgb,
    cursor: Rgb,
    color: alacritty_terminal::vte::ansi::Color,
) -> Rgb {
    use alacritty_terminal::vte::ansi::Color;
    match color {
        Color::Named(named) => {
            let idx = named as usize;
            if idx < 16 {
                colors[named].unwrap_or(palette[idx])
            } else {
                colors[named].unwrap_or(match named {
                    NamedColor::Foreground => fg,
                    NamedColor::Background => bg,
                    NamedColor::Cursor => cursor,
                    _ => fg,
                })
            }
        }
        Color::Indexed(i) => colors[i as usize].unwrap_or_else(|| {
            if i < 16 {
                palette[i as usize]
            } else if i < 232 {
                let i = i - 16;
                let steps = [0u8, 95, 135, 175, 215, 255];
                Rgb {
                    r: steps[(i / 36) as usize % 6],
                    g: steps[(i / 6) as usize % 6],
                    b: steps[i as usize % 6],
                }
            } else {
                let level = 8 + (i - 232) * 10;
                Rgb {
                    r: level,
                    g: level,
                    b: level,
                }
            }
        }),
        Color::Spec(rgb) => rgb,
    }
}

/// 把按键编码成终端要的字节。`key` 用 gpui 的键名（`enter` / `up` / `a` …）。
pub struct KeyInput<'a> {
    pub key: &'a str,
    pub ch: Option<char>,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

fn csi_mod(ctrl: bool, alt: bool, shift: bool) -> u8 {
    1 + (shift as u8) + ((alt as u8) << 1) + ((ctrl as u8) << 2)
}

pub fn encode_key(input: &KeyInput<'_>, mode: TermMode) -> Option<Vec<u8>> {
    let app = mode.contains(TermMode::APP_CURSOR);
    let m = csi_mod(input.ctrl, input.alt, input.shift);
    let with_mod = |base: &str| -> Vec<u8> {
        if m == 1 {
            format!("\x1b[{base}").into_bytes()
        } else {
            format!("\x1b[1;{m}{base}").into_bytes()
        }
    };
    // 带数字的键是 CSI code;mod~，不能写成 CSI 1;mod code~
    let tilde = |code: u8| -> Vec<u8> {
        if m == 1 {
            format!("\x1b[{code}~").into_bytes()
        } else {
            format!("\x1b[{code};{m}~").into_bytes()
        }
    };

    // 带修饰的方向/编辑键先走 CSI 1;<mod>
    let modified = input.ctrl || input.alt || input.shift;
    match input.key {
        "up" | "arrow_up" => {
            return Some(if modified {
                with_mod("A")
            } else if app {
                b"\x1bOA".to_vec()
            } else {
                b"\x1b[A".to_vec()
            })
        }
        "down" | "arrow_down" => {
            return Some(if modified {
                with_mod("B")
            } else if app {
                b"\x1bOB".to_vec()
            } else {
                b"\x1b[B".to_vec()
            })
        }
        "right" | "arrow_right" => {
            return Some(if modified {
                with_mod("C")
            } else if app {
                b"\x1bOC".to_vec()
            } else {
                b"\x1b[C".to_vec()
            })
        }
        "left" | "arrow_left" => {
            return Some(if modified {
                with_mod("D")
            } else if app {
                b"\x1bOD".to_vec()
            } else {
                b"\x1b[D".to_vec()
            })
        }
        "home" => {
            return Some(if modified {
                with_mod("H")
            } else if app {
                b"\x1bOH".to_vec()
            } else {
                b"\x1b[H".to_vec()
            })
        }
        "end" => {
            return Some(if modified {
                with_mod("F")
            } else if app {
                b"\x1bOF".to_vec()
            } else {
                b"\x1b[F".to_vec()
            })
        }
        "insert" => return Some(tilde(2)),
        "delete" => return Some(tilde(3)),
        "pageup" | "page_up" => return Some(tilde(5)),
        "pagedown" | "page_down" => return Some(tilde(6)),
        "tab" if input.shift => return Some(b"\x1b[Z".to_vec()),
        _ => {}
    }

    let seq: Vec<u8> = match input.key {
        "enter" => vec![b'\r'],
        "tab" => vec![b'\t'],
        "backspace" if input.ctrl => vec![0x08],
        "backspace" => vec![0x7f],
        "escape" => vec![0x1b],
        "space" if input.ctrl => vec![0x00],
        "space" => vec![b' '],
        "f1" => b"\x1bOP".to_vec(),
        "f2" => b"\x1bOQ".to_vec(),
        "f3" => b"\x1bOR".to_vec(),
        "f4" => b"\x1bOS".to_vec(),
        "f5" => b"\x1b[15~".to_vec(),
        "f6" => b"\x1b[17~".to_vec(),
        "f7" => b"\x1b[18~".to_vec(),
        "f8" => b"\x1b[19~".to_vec(),
        "f9" => b"\x1b[20~".to_vec(),
        "f10" => b"\x1b[21~".to_vec(),
        "f11" => b"\x1b[23~".to_vec(),
        "f12" => b"\x1b[24~".to_vec(),
        _ => {
            // gpui 把 Ctrl+字母 的 key_char 过滤成了 None（ToUnicode 得到的是控制字符），
            // 这里按 Linux 终端的老规矩从键名还原成 Ctrl+A..Z 对应的 0x01..0x1a。
            let from_key = {
                let mut it = input.key.chars();
                match (it.next(), it.next()) {
                    (Some(c), None) if c.is_ascii() && !c.is_ascii_control() => Some(c),
                    _ => None,
                }
            };
            let ch = match input.ch {
                Some(c) => Some(c),
                None if input.ctrl || input.alt => from_key,
                None => None,
            }?;
            let mut out = Vec::new();
            if input.ctrl {
                // Ctrl+A..Z、Ctrl+[ \ ] ^ _ @ 的经典映射
                let lower = ch.to_ascii_lowercase();
                let byte = match lower {
                    'a'..='z' => Some((lower as u8) - b'a' + 1),
                    '[' | '3' => Some(0x1b),
                    '\\' | '4' => Some(0x1c),
                    ']' | '5' => Some(0x1d),
                    '^' | '6' => Some(0x1e),
                    '_' | '7' | '/' => Some(0x1f),
                    '@' | '2' => Some(0x00),
                    '8' => Some(0x7f),
                    ' ' => Some(0x00),
                    _ => None,
                };
                if let Some(b) = byte {
                    if input.alt {
                        out.push(0x1b);
                    }
                    out.push(b);
                    return Some(out);
                }
            }
            if input.alt {
                out.push(0x1b);
            }
            let mut buf = [0u8; 4];
            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            return Some(out);
        }
    };

    if input.alt {
        let mut out = vec![0x1b];
        out.extend_from_slice(&seq);
        Some(out)
    } else {
        Some(seq)
    }
}

/// 交给 PTY 的粘贴文本：统一换行，开 bracketed paste 时包上标记。
pub fn encode_paste(text: &str, mode: TermMode) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
    if mode.contains(TermMode::BRACKETED_PASTE) {
        format!("\x1b[200~{normalized}\x1b[201~").into_bytes()
    } else {
        normalized.into_bytes()
    }
}

/// 鼠标上报。SGR（xterm 1006）用 CSI < b;x;y M/m；老式 X10 用 CSI M 加三个字节。
/// 按钮：0 左、1 中、2 右、64/65 滚轮。
pub fn encode_mouse(
    row: usize,
    col: usize,
    button: u8,
    press: bool,
    motion: bool,
    ctrl: bool,
    alt: bool,
    shift: bool,
    sgr: bool,
) -> Vec<u8> {
    let mut b = button;
    if motion {
        b |= 32;
    }
    if !press {
        b |= 3;
    }
    if shift {
        b |= 4;
    }
    if alt {
        b |= 8;
    }
    if ctrl {
        b |= 16;
    }
    if sgr {
        format!("\x1b[<{b};{};{}M", col + 1, row + 1).into_bytes()
    } else {
        // X10 只有按下/松开，坐标从 1 起加 32，最大 223
        let x = (col + 1).min(223) as u8 + 32;
        let y = (row + 1).min(223) as u8 + 32;
        vec![0x1b, b'[', b'M', 32 + b, x, y]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: &str, ch: Option<char>, ctrl: bool, alt: bool, shift: bool) -> Option<Vec<u8>> {
        encode_key(
            &KeyInput {
                key,
                ch,
                ctrl,
                alt,
                shift,
            },
            TermMode::empty(),
        )
    }

    #[test]
    fn ctrl_letters_survive_missing_key_char() {
        // gpui 会把 Ctrl+字母 的 key_char 过滤掉，编码层必须从键名还原控制字节
        for (k, byte) in [
            ("a", 0x01),
            ("c", 0x03),
            ("d", 0x04),
            ("e", 0x05),
            ("k", 0x0b),
            ("l", 0x0c),
            ("r", 0x12),
            ("u", 0x15),
            ("w", 0x17),
            ("z", 0x1a),
        ] {
            assert_eq!(
                key(k, None, true, false, false),
                Some(vec![byte]),
                "ctrl+{k}"
            );
        }
    }

    #[test]
    fn ctrl_shift_letters_are_still_control_bytes() {
        assert_eq!(key("c", None, true, false, true), Some(vec![0x03]));
        assert_eq!(key("v", None, true, false, true), Some(vec![0x16]));
    }

    #[test]
    fn ctrl_symbols_follow_xterm() {
        assert_eq!(key("space", None, true, false, false), Some(vec![0x00]));
        assert_eq!(key("/", None, true, false, false), Some(vec![0x1f]));
        assert_eq!(key("[", None, true, false, false), Some(vec![0x1b]));
        assert_eq!(key("2", None, true, false, false), Some(vec![0x00]));
        assert_eq!(key("8", None, true, false, false), Some(vec![0x7f]));
    }

    #[test]
    fn backspace_matches_linux() {
        assert_eq!(
            key("backspace", None, false, false, false),
            Some(vec![0x7f])
        );
        assert_eq!(key("backspace", None, true, false, false), Some(vec![0x08]));
        assert_eq!(
            key("backspace", None, false, true, false),
            Some(vec![0x1b, 0x7f])
        );
    }

    #[test]
    fn alt_prefixes_escape() {
        assert_eq!(
            key("b", Some('b'), false, true, false),
            Some(vec![0x1b, b'b'])
        );
        assert_eq!(
            key("b", Some('b'), true, true, false),
            Some(vec![0x1b, 0x02])
        );
    }

    #[test]
    fn modified_editing_keys_use_csi_param() {
        assert_eq!(
            key("tab", None, false, false, true),
            Some(b"\x1b[Z".to_vec())
        );
        assert_eq!(
            key("home", None, true, false, false),
            Some(b"\x1b[1;5H".to_vec())
        );
        assert_eq!(
            key("delete", None, false, false, true),
            Some(b"\x1b[3;2~".to_vec())
        );
        assert_eq!(
            key("pageup", None, false, false, false),
            Some(b"\x1b[5~".to_vec())
        );
    }

    #[test]
    fn app_cursor_mode_uses_ss3() {
        let mode = TermMode::APP_CURSOR;
        let up = encode_key(
            &KeyInput {
                key: "up",
                ch: None,
                ctrl: false,
                alt: false,
                shift: false,
            },
            mode,
        );
        assert_eq!(up, Some(b"\x1bOA".to_vec()));
    }

    #[test]
    fn mouse_encodes_sgr_and_legacy() {
        assert_eq!(
            encode_mouse(0, 0, 0, true, false, false, false, false, true),
            b"\x1b[<0;1;1M".to_vec()
        );
        assert_eq!(
            encode_mouse(1, 2, 0, true, false, false, false, false, false),
            vec![0x1b, b'[', b'M', 32, 32 + 3, 32 + 2]
        );
    }
}
