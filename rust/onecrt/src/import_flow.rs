//! 导入 SecureCRT / WinSCP 会话：来源选择对话框 + 冲突合并规则。
//!
//! 合并规则的硬约束（沿用旧版）：冲突键 = host+port；命中项只允许覆盖口令，
//! 且导入侧口令为空时绝不覆盖（避免把用户手上的口令清空）；文件夹按名字递归合并。

use std::rc::Rc;

use gpui_kit as gpui;

use gpui::{
    div, px, AnyWindowHandle, App, AppContext, Context, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
};
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    ActiveTheme,
};
use onecrt_core::import;
use onecrt_core::sessions::{Node, SessionTree};

use crate::app::new_id;
use crate::dialog_window;
use crate::i18n::{tr, Lang};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ImportKind {
    SecureCrt,
    WinScp,
}

pub struct Conflict {
    pub ids: Vec<String>,
    pub password: String,
}

pub struct Plan {
    pub to_append: Vec<Node>,
    pub conflicts: Vec<Conflict>,
    pub new_count: usize,
}

fn session_key(host: &str, port: u16) -> String {
    format!("{}:{port}", host.trim().to_lowercase())
}

fn index_by_host_port(nodes: &[Node], out: &mut Vec<(String, String)>) {
    for n in nodes {
        match n {
            Node::Folder { children, .. } => index_by_host_port(children, out),
            Node::Session(s) => out.push((session_key(&s.host, s.port), s.id.clone())),
        }
    }
}

/// 从导入树里摘出命中现有会话的条目，剩下的（顺带裁掉变空的文件夹）才进 to_append。
fn prune(
    nodes: Vec<Node>,
    existing: &[(String, String)],
    conflicts: &mut Vec<Conflict>,
) -> Vec<Node> {
    let mut kept = Vec::new();
    for n in nodes {
        match n {
            Node::Folder {
                id,
                name,
                expanded,
                children,
            } => {
                let children = prune(children, existing, conflicts);
                if !children.is_empty() {
                    kept.push(Node::Folder {
                        id,
                        name,
                        expanded,
                        children,
                    });
                }
            }
            Node::Session(s) => {
                let key = session_key(&s.host, s.port);
                let ids: Vec<String> = existing
                    .iter()
                    .filter(|(k, _)| *k == key)
                    .map(|(_, id)| id.clone())
                    .collect();
                if ids.is_empty() {
                    kept.push(Node::Session(s));
                } else {
                    conflicts.push(Conflict {
                        ids,
                        password: s.password,
                    });
                }
            }
        }
    }
    kept
}

fn count_sessions(nodes: &[Node]) -> usize {
    nodes
        .iter()
        .map(|n| match n {
            Node::Folder { children, .. } => count_sessions(children),
            Node::Session(_) => 1,
        })
        .sum()
}

/// 制定导入计划，现有树不被修改。
pub fn plan(existing: &[Node], imported: Vec<Node>) -> Plan {
    let mut index = Vec::new();
    index_by_host_port(existing, &mut index);
    let mut conflicts = Vec::new();
    let to_append = prune(imported, &index, &mut conflicts);
    Plan {
        new_count: count_sessions(&to_append),
        conflicts,
        to_append,
    }
}

fn merge_into(target: &mut Vec<Node>, incoming: Vec<Node>) {
    for n in incoming {
        match n {
            Node::Folder {
                name,
                expanded,
                children,
                ..
            } => {
                let hit = target.iter_mut().find_map(|t| match t {
                    Node::Folder {
                        name: tname,
                        expanded: texpanded,
                        children: tchildren,
                        ..
                    } if *tname == name => Some((texpanded, tchildren)),
                    _ => None,
                });
                match hit {
                    Some((texpanded, tchildren)) => {
                        *texpanded = *texpanded || expanded;
                        merge_into(tchildren, children);
                    }
                    None => {
                        let mut kids = Vec::new();
                        merge_into(&mut kids, children);
                        target.push(Node::Folder {
                            id: new_id("f"),
                            name,
                            expanded,
                            children: kids,
                        });
                    }
                }
            }
            Node::Session(s) => {
                let mut s = s;
                s.id = new_id("s");
                target.push(Node::Session(s));
            }
        }
    }
}

/// 把计划里的新增部分并入会话树，同名文件夹递归合并。
pub fn append(tree: &mut SessionTree, to_append: Vec<Node>) {
    merge_into(&mut tree.sessions, to_append);
}

/// 只改命中会话的口令，空口令跳过。
pub fn overwrite(tree: &mut SessionTree, conflicts: &[Conflict]) {
    let mut map: Vec<(String, String)> = Vec::new();
    for c in conflicts {
        if c.password.is_empty() {
            continue;
        }
        for id in &c.ids {
            map.push((id.clone(), c.password.clone()));
        }
    }
    if map.is_empty() {
        return;
    }
    fn walk(nodes: &mut [Node], map: &[(String, String)]) {
        for n in nodes {
            match n {
                Node::Folder { children, .. } => walk(children, map),
                Node::Session(s) => {
                    if let Some((_, password)) = map.iter().rev().find(|(id, _)| *id == s.id) {
                        s.password = password.clone();
                    }
                }
            }
        }
    }
    walk(&mut tree.sessions, &map);
}

/// 按来源解析文件文本；SecureCRT 是 XML，WinSCP 是 INI。
pub fn parse(kind: ImportKind, text: &str) -> Result<Vec<Node>, String> {
    match kind {
        ImportKind::SecureCrt => import::parse_securecrt(text),
        ImportKind::WinScp => import::parse_winscp(text),
    }
}

/// 配置文件可能是 UTF-8（带不带 BOM）或 UTF-16LE，按 BOM 判定后再交给解析器。
pub fn read_text(path: &std::path::Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读取文件失败: {e}"))?;
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16(&units).map_err(|e| format!("UTF-16 解码失败: {e}"));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// 来源选择对话框。
pub fn open_import_picker(
    window: &mut Window,
    cx: &mut App,
    lang: Lang,
    on_pick: impl Fn(ImportKind, &mut Window, &mut App) + 'static,
) {
    let parent = window.window_handle();
    let on_pick: Rc<dyn Fn(ImportKind, &mut Window, &mut App)> = Rc::new(on_pick);
    dialog_window::open(
        window,
        cx,
        tr(lang, "导入会话", "Import Sessions"),
        440.,
        272.,
        move |_, cx| {
            cx.new(|_| ImportPicker {
                lang,
                parent,
                on_pick,
            })
        },
    );
}

struct ImportPicker {
    lang: Lang,
    parent: AnyWindowHandle,
    on_pick: Rc<dyn Fn(ImportKind, &mut Window, &mut App)>,
}

impl Render for ImportPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = self.lang;
        let parent = self.parent;
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let hover = theme.secondary;
        let border = theme.border;
        let radius = theme.radius;
        let sources = [
            (
                ImportKind::SecureCrt,
                "SecureCRT",
                tr(
                    lang,
                    "选择导出目录里的 Sessions 配置（XML）",
                    "Pick the exported Sessions config (XML)",
                ),
            ),
            (
                ImportKind::WinScp,
                "WinSCP",
                tr(
                    lang,
                    "选择 WinSCP.ini（含 [Sessions\\...] 段落）",
                    "Pick WinSCP.ini (with [Sessions\\...] sections)",
                ),
            ),
        ];
        let mut rows = div().flex().flex_col().gap(px(6.));
        for (kind, name, hint) in sources {
            let pick = self.on_pick.clone();
            rows = rows.child(
                div()
                    .id(name)
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .px(px(10.))
                    .py(px(8.))
                    .rounded(radius)
                    .border_1()
                    .border_color(border)
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .on_click(move |_, window, cx| {
                        let pick = pick.clone();
                        // 选完马上要弹文件选择框和冲突确认，等这个窗口真正关掉再继续。
                        dialog_window::run_after_close(parent, cx, move |window, cx| {
                            pick(kind, window, cx)
                        });
                        window.remove_window();
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(div().text_size(px(13.)).text_color(fg).child(name))
                            .child(div().text_size(px(11.)).text_color(muted).child(hint)),
                    )
                    .child(div().text_size(px(12.)).text_color(muted).child("→")),
            );
        }
        dialog_window::page(
            cx,
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(div().text_size(px(11.)).text_color(muted).child(tr(
                    lang,
                    "同主机同端口的会话视为同一条：默认只新增，冲突时可选择把导入的口令覆盖过去。",
                    "Sessions with the same host and port are treated as the same: new ones are added, and conflicts may optionally overwrite passwords.",
                )))
                .child(rows),
            dialog_window::buttons().child(
                Button::new("import-cancel")
                    .label(tr(lang, "取消", "Cancel"))
                    .on_click(|_, window, _| window.remove_window()),
            ),
        )
    }
}

/// 冲突三选：只新增 / 新增并覆盖口令 / 取消；`on_resolve` 的 bool 表示是否覆盖口令。
pub fn open_conflict_dialog(
    window: &mut Window,
    cx: &mut App,
    lang: Lang,
    new_count: usize,
    conflict_count: usize,
    on_resolve: impl Fn(bool, &mut Window, &mut App) + 'static,
) {
    let parent = window.window_handle();
    let summary: SharedString = if lang == Lang::Zh {
        format!("{new_count} 条新增；{conflict_count} 条与现有会话同主机同端口。").into()
    } else {
        format!("{new_count} new; {conflict_count} already exist with the same host and port.")
            .into()
    };
    let on_resolve: Rc<dyn Fn(bool, &mut Window, &mut App)> = Rc::new(on_resolve);
    dialog_window::open(
        window,
        cx,
        tr(lang, "导入冲突", "Import Conflicts"),
        460.,
        180.,
        move |_, cx| {
            cx.new(|_| ConflictWindow {
                lang,
                parent,
                summary,
                on_resolve,
            })
        },
    );
}

struct ConflictWindow {
    lang: Lang,
    parent: AnyWindowHandle,
    summary: SharedString,
    on_resolve: Rc<dyn Fn(bool, &mut Window, &mut App)>,
}

impl Render for ConflictWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = self.lang;
        let parent = self.parent;
        let append = self.on_resolve.clone();
        let overwrite = self.on_resolve.clone();
        dialog_window::page(
            cx,
            div()
                .text_size(px(12.))
                .text_color(cx.theme().muted_foreground)
                .child(self.summary.clone()),
            dialog_window::buttons()
                .child(
                    Button::new("import-cancel")
                        .label(tr(lang, "取消", "Cancel"))
                        .on_click(|_, window, _| window.remove_window()),
                )
                .child(
                    Button::new("import-append")
                        .label(tr(lang, "只新增", "Append only"))
                        .on_click(move |_, window, cx| {
                            dialog_window::run_in_parent(parent, cx, |window, cx| {
                                append(false, window, cx)
                            });
                            window.remove_window();
                        }),
                )
                .child(
                    Button::new("import-overwrite")
                        .primary()
                        .label(tr(lang, "新增并覆盖口令", "Append + overwrite passwords"))
                        .on_click(move |_, window, cx| {
                            dialog_window::run_in_parent(parent, cx, |window, cx| {
                                overwrite(true, window, cx)
                            });
                            window.remove_window();
                        }),
                ),
        )
    }
}
