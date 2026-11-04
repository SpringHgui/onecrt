//! MCP 服务设置：后台服务开关（开机自启）、端口、令牌与接入配置，附带智能体会话列表。
//!
//! 状态每秒读一次配置文件和后台进程心跳，弹窗关掉后轮询任务自己退出。
//! 接入配置按客户端给出片段，复制出去的那份永远带当前真实令牌。

use std::sync::Arc;
use std::time::Duration;

use async_channel::{unbounded, Sender};
use gpui_kit as gpui;

use gpui::{
    div, px, App, AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Window,
};
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    input::{Input, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    switch::Switch,
    ActiveTheme, Disableable,
};
use onecrt_core::mcp::{self, Status};
use onecrt_core::Backend;

use crate::dialog_window;
use crate::dialog_window::ActionButton;
use crate::i18n::{tr, Lang};

enum Msg {
    Status(Status),
    Failed(String),
    CopiedDone,
}

pub struct McpDialog {
    backend: Arc<Backend>,
    lang: Lang,
    status: Option<Status>,
    port: Entity<InputState>,
    target: Target,
    show_token: bool,
    copied: bool,
    error: Option<String>,
    tx: Sender<Msg>,
    _pump: gpui::Task<()>,
}

impl McpDialog {
    pub fn new(
        backend: Arc<Backend>,
        lang: Lang,
        initial: Status,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (tx, rx) = unbounded::<Msg>();
        let pump = cx.spawn(async move |this, cx| {
            while let Ok(msg) = rx.recv().await {
                if this.update(cx, |dialog, cx| dialog.apply(msg, cx)).is_err() {
                    break;
                }
            }
        });
        let port_text = initial.port.to_string();
        let port = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("9977")
                .default_value(port_text)
        });

        let poll_tx = tx.clone();
        backend.spawn(async move {
            loop {
                let status = mcp::client_status();
                if poll_tx.try_send(Msg::Status(status)).is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });

        Self {
            backend,
            lang,
            status: Some(initial),
            port,
            target: Target::default(),
            show_token: false,
            copied: false,
            error: None,
            tx,
            _pump: pump,
        }
    }

    fn apply(&mut self, msg: Msg, cx: &mut Context<Self>) {
        match msg {
            Msg::Status(status) => {
                self.status = Some(status);
                self.error = None;
            }
            Msg::Failed(error) => {
                self.error = Some(error);
            }
            Msg::CopiedDone => {
                self.copied = false;
            }
        }
        cx.notify();
    }

    fn set_enabled(&mut self, enabled: bool) {
        let tx = self.tx.clone();
        self.backend.spawn(async move {
            match mcp::set_enabled(enabled) {
                Ok(status) => {
                    let _ = tx.try_send(Msg::Status(status));
                }
                Err(error) => {
                    let _ = tx.try_send(Msg::Failed(error));
                }
            }
        });
    }

    fn set_port(&mut self, port: u16, cx: &mut Context<Self>) {
        let tx = self.tx.clone();
        self.backend.spawn(async move {
            match mcp::set_port(port) {
                Ok(status) => {
                    let _ = tx.try_send(Msg::Status(status));
                }
                Err(error) => {
                    let _ = tx.try_send(Msg::Failed(error));
                }
            }
        });
        cx.notify();
    }

    fn rotate_token(&mut self) {
        let tx = self.tx.clone();
        self.backend.spawn(async move {
            match mcp::rotate_token() {
                Ok(status) => {
                    let _ = tx.try_send(Msg::Status(status));
                }
                Err(error) => {
                    let _ = tx.try_send(Msg::Failed(error));
                }
            }
        });
    }

    /// 复制的是当前所选格式，且永远带真实令牌——界面上为了防旁窥可以先把它隐去
    fn copy_snippet(&mut self, cx: &mut Context<Self>) {
        let Some(status) = self.status.clone() else {
            return;
        };
        let snippet = self.target.snippet(&status.url, &status.token);
        match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(snippet)) {
            Ok(()) => {
                self.copied = true;
                let tx = self.tx.clone();
                self.backend.spawn(async move {
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                    let _ = tx.try_send(Msg::CopiedDone);
                });
            }
            Err(_) => self.copied = false,
        }
        cx.notify();
    }
}

/// 屏幕上没展开令牌时代码块里显示的这个占位值
const TOKEN_PLACEHOLDER: &str = "<TOKEN>";

/// 接入配置要面向的客户端：同一个地址各家的键名不一样，只能一份一份给
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Target {
    /// `mcpServers` + `type: "http"`
    #[default]
    General,
    ClaudeCode,
    CodexCli,
    Cursor,
    Windsurf,
    Cline,
    RooCode,
    GeminiCli,
    Zed,
    OpenCode,
}

impl Target {
    const ALL: [Target; 10] = [
        Target::General,
        Target::ClaudeCode,
        Target::CodexCli,
        Target::Cursor,
        Target::Windsurf,
        Target::Cline,
        Target::RooCode,
        Target::GeminiCli,
        Target::Zed,
        Target::OpenCode,
    ];

    fn label(self) -> &'static str {
        match self {
            Target::General => "通用 JSON",
            Target::ClaudeCode => "Claude Code",
            Target::CodexCli => "Codex CLI",
            Target::Cursor => "Cursor",
            Target::Windsurf => "Windsurf",
            Target::Cline => "Cline",
            Target::RooCode => "Roo Code",
            Target::GeminiCli => "Gemini CLI",
            Target::Zed => "Zed",
            Target::OpenCode => "opencode",
        }
    }

    /// 这份片段贴到哪儿去
    fn hint(self, lang: Lang) -> SharedString {
        match self {
            Target::General => tr(
                lang,
                "贴进按 mcpServers 结构配置的客户端（Claude Code 的 ~/.claude.json、Qoder 等）",
                "For clients configured through mcpServers (Claude Code's ~/.claude.json, Qoder, …)",
            ),
            Target::ClaudeCode => tr(
                lang,
                "在终端执行，写进 ~/.claude.json 的用户作用域",
                "Run it in a terminal; it lands in ~/.claude.json at user scope",
            ),
            Target::CodexCli => tr(
                lang,
                "追加到 ~/.codex/config.toml",
                "Append to ~/.codex/config.toml",
            ),
            Target::Cursor => tr(
                lang,
                "项目用 .cursor/mcp.json，全局用 ~/.cursor/mcp.json",
                "Project: .cursor/mcp.json; global: ~/.cursor/mcp.json",
            ),
            Target::Windsurf => tr(
                lang,
                "Cascade 面板里点 Open MCP Config 打开的那个文件",
                "The file behind Open MCP Config in the Cascade panel",
            ),
            Target::Cline => tr(
                lang,
                "VS Code 的 MCP 面板 → Edit MCP Settings（mcp_settings.json）",
                "VS Code MCP panel → Edit MCP Settings (mcp_settings.json)",
            ),
            Target::RooCode => tr(
                lang,
                "项目用 .roo/mcp.json，或全局 mcp_settings.json",
                "Project: .roo/mcp.json, or the global mcp_settings.json",
            ),
            Target::GeminiCli => tr(
                lang,
                "写到 ~/.gemini/settings.json（远程走 httpUrl，url 是旧的 SSE）",
                "Write into ~/.gemini/settings.json (httpUrl is the remote transport; url means legacy SSE)",
            ),
            Target::Zed => tr(
                lang,
                "写到 ~/.config/zed/settings.json 的 context_servers",
                "Add to context_servers in ~/.config/zed/settings.json",
            ),
            Target::OpenCode => tr(
                lang,
                "写到项目根的 opencode.json",
                "Write into opencode.json at the project root",
            ),
        }
    }

    fn snippet(self, url: &str, token: &str) -> String {
        let remote = |wrapper: &str, fields: &[(&str, serde_json::Value)]| {
            remote_json(wrapper, "onecrt", fields, token)
        };
        match self {
            Target::General => remote(
                "mcpServers",
                &[("type", serde_json::json!("http")), ("url", url.into())],
            ),
            Target::ClaudeCode => format!(
                "claude mcp add --transport http onecrt {url} --header \"Authorization: Bearer {token}\" --scope user"
            ),
            Target::CodexCli => format!(
                "[mcp_servers.onecrt]\nurl = \"{url}\"\nhttp_headers = {{ Authorization = \"Bearer {token}\" }}\n"
            ),
            Target::Cursor => remote("mcpServers", &[("url", url.into())]),
            Target::Windsurf => remote("mcpServers", &[("serverUrl", url.into())]),
            Target::Cline => remote(
                "mcpServers",
                &[
                    ("type", serde_json::json!("streamableHttp")),
                    ("url", url.into()),
                ],
            ),
            Target::RooCode => remote(
                "mcpServers",
                &[
                    ("type", serde_json::json!("streamable-http")),
                    ("url", url.into()),
                ],
            ),
            Target::GeminiCli => remote("mcpServers", &[("httpUrl", url.into())]),
            Target::Zed => remote("context_servers", &[("url", url.into())]),
            Target::OpenCode => remote(
                "mcp",
                &[
                    ("type", serde_json::json!("remote")),
                    ("url", url.into()),
                    ("enabled", serde_json::json!(true)),
                ],
            ),
        }
    }
}

/// 手写缩进而不用 serde_json 的 pretty：它的键按字母序输出，会把 `type` 挤到 `headers` 后面，
/// 和各家文档的读法对不上。这里插值的只有本程序自己产出的 url 与十六进制令牌，无需转义。
fn remote_json(
    wrapper: &str,
    name: &str,
    fields: &[(&str, serde_json::Value)],
    token: &str,
) -> String {
    let mut out = String::from("{\n");
    out.push_str(&format!("  \"{wrapper}\": {{\n    \"{name}\": {{\n"));
    let mut lines: Vec<String> = fields
        .iter()
        .map(|(k, v)| format!("      \"{k}\": {v}"))
        .collect();
    lines.push(format!(
        "      \"headers\": {{\n        \"Authorization\": \"Bearer {token}\"\n      }}"
    ));
    out.push_str(&lines.join(",\n"));
    out.push_str("\n    }\n  }\n}\n");
    out
}

impl Render for McpDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let danger = theme.danger;
        let success = theme.success;
        let lang = self.lang;
        let radius = theme.radius;
        let hover = theme.secondary;
        let code_bg = theme.muted;
        let mono = theme.mono_font_family.clone();

        let Some(status) = self.status.clone() else {
            return div().text_size(px(12.)).text_color(muted).child("…");
        };

        let port_value = self.port.read(cx).value().trim().to_string();
        let port_next = port_value.parse::<u16>().ok();
        let port_changed = port_next.is_some_and(|p| p != status.port);

        let toggle = Switch::new("mcp-enabled")
            .checked(status.enabled)
            .label(tr(
                lang,
                "后台运行 MCP 服务（开机自启）",
                "Run MCP server in background (starts at login)",
            ))
            .on_change(cx.listener(|this, checked: &bool, _, _cx| {
                this.set_enabled(*checked);
            }));

        let token_text = if self.show_token {
            status.token.clone()
        } else {
            "•".repeat(status.token.chars().count().min(24))
        };

        let mut root = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(muted)
                    .child(tr(
                        lang,
                        "把已保存的会话开放给外部智能体当 MCP 工具调用。服务跑在独立的后台进程里，主程序开没开都能被调用；这个开关同时决定登录时是否自启。只监听本机回环地址，调用必须带令牌；智能体看不到口令本身。",
                        "Expose saved sessions to external agents as MCP tools. The server lives in its own background process, reachable whether or not OneCRT is open; this switch also decides whether it starts at login. Loopback only, every call requires the token; agents never see passwords.",
                    )),
            )
            .child(div().h(px(24.)).flex().items_center().child(toggle))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().text_size(px(11.)).text_color(muted).child(tr(lang, "端口", "Port")))
                    .child(div().w(px(90.)).child(Input::new(&self.port)))
                    .child(
                        Button::new("mcp-port-apply")
                            .label(tr(lang, "应用", "Apply"))
                            .disabled(!port_changed)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let text = this.port.read(cx).value().trim().to_string();
                                if let Ok(port) = text.parse::<u16>() {
                                    this.set_port(port, cx);
                                }
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().text_size(px(11.)).text_color(muted).child(tr(lang, "令牌", "Token")))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .px(px(8.))
                            .py(px(5.))
                            .rounded(radius)
                            .bg(code_bg)
                            .text_size(px(11.))
                            .text_color(fg)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(SharedString::from(token_text)),
                    )
                    .child(
                        Button::new("mcp-token-show")
                            .action_height()
                            .label(if self.show_token {
                                tr(lang, "隐藏", "Hide")
                            } else {
                                tr(lang, "显示", "Show")
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_token = !this.show_token;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("mcp-token-rotate")
                            .action_height()
                            .label(tr(lang, "重新生成", "Regenerate"))
                            .on_click(cx.listener(|this, _, _, _cx| this.rotate_token())),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(if status.running { success } else { muted })
                    .child(if status.running {
                        SharedString::from(format!(
                            "{} {}",
                            tr(lang, "监听中", "Listening"),
                            status.url
                        ))
                    } else {
                        tr(lang, "后台服务未监听", "Background service not listening")
                    }),
            );

        // 接入配置：复制之前先看到片段本身，换客户端只切格式，不用重开弹窗
        let current = self.target;
        let dialog = cx.entity();
        let shown = current.snippet(
            &status.url,
            if self.show_token {
                &status.token
            } else {
                TOKEN_PLACEHOLDER
            },
        );
        root = root.child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().text_size(px(11.)).text_color(muted).child(tr(
                            lang,
                            "接入配置",
                            "Connect config",
                        )))
                        .child(
                            Button::new("mcp-target")
                                .compact()
                                .label(current.label())
                                .dropdown_menu(move |menu, _, _| {
                                    let mut menu = menu.min_w(px(170.));
                                    for target in Target::ALL {
                                        let dialog = dialog.clone();
                                        menu = menu.item(
                                            PopupMenuItem::new(target.label())
                                                .checked(target == current)
                                                .on_click(move |_, _, cx| {
                                                    dialog.update(cx, |this, cx| {
                                                        this.target = target;
                                                        cx.notify();
                                                    });
                                                }),
                                        );
                                    }
                                    menu
                                }),
                        )
                        .child(div().flex_1())
                        .child(
                            Button::new("mcp-copy")
                                .action_height()
                                .label(if self.copied {
                                    tr(lang, "已复制", "Copied")
                                } else {
                                    tr(lang, "复制", "Copy")
                                })
                                .disabled(status.token.is_empty())
                                .on_click(cx.listener(|this, _, _, cx| this.copy_snippet(cx))),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .px(px(8.))
                        .py(px(6.))
                        .rounded(radius)
                        .bg(code_bg)
                        .children(shown.lines().map(|line| {
                            div()
                                .text_size(px(11.))
                                .font_family(mono.clone())
                                .text_color(fg)
                                .child(SharedString::from(line.to_string()))
                        })),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(muted)
                        .child(if self.show_token {
                            current.hint(lang)
                        } else {
                            // 屏上写着 <TOKEN>，得说清复制走的不是这个占位值
                            SharedString::from(format!(
                                "{} · {}",
                                current.hint(lang),
                                tr(lang, "复制带真实令牌", "copy carries the real token")
                            ))
                        }),
                ),
        );

        if let Some(error) = status.error.clone().or_else(|| self.error.clone()) {
            root = root.child(
                div()
                    .text_size(px(11.))
                    .text_color(danger)
                    .child(SharedString::from(error)),
            );
        }

        root = root.child(
            div()
                .mt(px(2.))
                .text_size(px(11.))
                .text_color(muted)
                .child(tr(lang, "智能体会话", "Agent sessions")),
        );
        if status.agents.is_empty() {
            root = root.child(div().text_size(px(11.)).text_color(muted).child(tr(
                lang,
                "暂无智能体连接",
                "No agent connected",
            )));
        } else {
            for agent in &status.agents {
                root = root.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(8.))
                        .py(px(3.))
                        .rounded(radius)
                        .hover(move |s| s.bg(hover))
                        .child(
                            div()
                                .w(px(120.))
                                .text_size(px(11.))
                                .text_color(fg)
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(SharedString::from(agent.session_name.clone())),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .text_size(px(11.))
                                .text_color(muted)
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(if agent.last_command.is_empty() {
                                    "—".to_string()
                                } else {
                                    agent.last_command.clone()
                                }),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(if agent.closed { muted } else { success })
                                .child(if agent.closed {
                                    tr(lang, "已结束", "Closed")
                                } else {
                                    tr(lang, "在线", "Live")
                                }),
                        ),
                );
            }
        }

        root.child(
            div()
                .mt(px(2.))
                .text_size(px(11.))
                .text_color(muted)
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(SharedString::from(format!(
                    "{} {}",
                    tr(lang, "配置文件", "Config file"),
                    status.config_file
                ))),
        )
    }
}

pub fn open_mcp_dialog(window: &mut Window, cx: &mut App, backend: Arc<Backend>, lang: Lang) {
    let initial = mcp::client_status();
    dialog_window::open(
        window,
        cx,
        tr(lang, "MCP 服务", "MCP Server"),
        460.,
        580.,
        move |window, cx| {
            let panel = cx.new(|cx| McpDialog::new(backend, lang, initial, window, cx));
            cx.new(|_| McpWindow { lang, panel })
        },
    );
}

struct McpWindow {
    lang: Lang,
    panel: Entity<McpDialog>,
}

impl Render for McpWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        dialog_window::page(
            cx,
            div()
                .id("mcp-body")
                .size_full()
                .overflow_y_scroll()
                .child(self.panel.clone()),
            dialog_window::buttons().child(
                Button::new("mcp-close")
                    .action_height()
                    .primary()
                    .label(tr(self.lang, "关闭", "Close"))
                    .on_click(|_, window, _| window.remove_window()),
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const URL: &str = "http://127.0.0.1:39117/mcp";
    const TOKEN: &str = "deadbeef00000000";

    /// 片段是手写缩进拼出来的：少个逗号整份就废，所以先要求它解析得过
    fn entry(target: Target) -> Value {
        let got = target.snippet(URL, TOKEN);
        let wrapper = match target {
            Target::Zed => "context_servers",
            Target::OpenCode => "mcp",
            _ => "mcpServers",
        };
        let json: Value = serde_json::from_str(&got)
            .unwrap_or_else(|e| panic!("{} 的片段不是合法 JSON: {e}\n{got}", target.label()));
        let entry = &json[wrapper]["onecrt"];
        assert!(
            !entry.is_null(),
            "{} 的片段里没有 {wrapper}.onecrt:\n{got}",
            target.label()
        );
        entry.clone()
    }

    /// 各家真正的区别就在这一行：同一个地址落在不同的键名下
    #[test]
    fn each_client_uses_its_own_url_key() {
        let cases = [
            (Target::General, "url"),
            (Target::Cursor, "url"),
            (Target::Cline, "url"),
            (Target::RooCode, "url"),
            (Target::OpenCode, "url"),
            (Target::Zed, "url"),
            (Target::Windsurf, "serverUrl"),
            (Target::GeminiCli, "httpUrl"),
        ];
        for (target, key) in cases {
            let entry = entry(target);
            assert_eq!(
                entry[key].as_str(),
                Some(URL),
                "{} 该把地址放在 {key}:\n{}",
                target.label(),
                target.snippet(URL, TOKEN)
            );
            for other in ["url", "serverUrl", "httpUrl"] {
                if other != key {
                    assert!(
                        entry.get(other).is_none(),
                        "{} 只认 {key}，不该同时出现 {other}",
                        target.label()
                    );
                }
            }
        }
    }

    #[test]
    fn every_json_snippet_carries_the_bearer_header() {
        for target in Target::ALL {
            if matches!(target, Target::ClaudeCode | Target::CodexCli) {
                continue;
            }
            assert_eq!(
                entry(target)["headers"]["Authorization"].as_str(),
                Some(format!("Bearer {TOKEN}")).as_deref(),
                "{} 的鉴权头写法不对",
                target.label()
            );
        }
    }

    /// type 字符串各家自己定，写错一个字母客户端就按旧的 SSE 去连
    #[test]
    fn transport_type_matches_each_clients_docs() {
        assert_eq!(entry(Target::General)["type"].as_str(), Some("http"));
        assert_eq!(
            entry(Target::Cline)["type"].as_str(),
            Some("streamableHttp")
        );
        assert_eq!(
            entry(Target::RooCode)["type"].as_str(),
            Some("streamable-http")
        );
        assert_eq!(entry(Target::OpenCode)["type"].as_str(), Some("remote"));
        assert_eq!(entry(Target::OpenCode)["enabled"], Value::Bool(true));
        for target in [
            Target::Cursor,
            Target::Windsurf,
            Target::GeminiCli,
            Target::Zed,
        ] {
            assert!(
                entry(target).get("type").is_none(),
                "{} 的远程写法里没有 type",
                target.label()
            );
        }
    }

    #[test]
    fn claude_code_snippet_is_a_runnable_command() {
        assert_eq!(
            Target::ClaudeCode.snippet(URL, TOKEN),
            format!("claude mcp add --transport http onecrt {URL} --header \"Authorization: Bearer {TOKEN}\" --scope user")
        );
    }

    #[test]
    fn codex_snippet_is_a_toml_table() {
        assert_eq!(
            Target::CodexCli.snippet(URL, TOKEN),
            format!("[mcp_servers.onecrt]\nurl = \"{URL}\"\nhttp_headers = {{ Authorization = \"Bearer {TOKEN}\" }}\n")
        );
    }

    /// 下拉里出现重名格式就没法选；弹窗一开必须落在通用的那份上
    #[test]
    fn the_menu_labels_are_unique_and_start_on_generic() {
        let mut labels: Vec<&str> = Target::ALL.iter().map(|t| t.label()).collect();
        labels.sort_unstable();
        let total = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), total, "下拉里有重名格式");
        assert_eq!(Target::default(), Target::General);
    }
}
