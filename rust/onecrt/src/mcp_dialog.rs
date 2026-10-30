//! MCP 服务设置：开关、端口、令牌与接入配置，附带当前智能体会话列表。
//!
//! 状态每秒从 Core 拉一次，弹窗关掉后轮询任务自己退出。

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
    switch::Switch,
    ActiveTheme, Disableable,
};
use onecrt_core::mcp::{self, Status};
use onecrt_core::Backend;

use crate::dialog_window;
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

        let mgr = backend.mcp.clone();
        let poll_tx = tx.clone();
        backend.spawn(async move {
            loop {
                let status = mcp::status(&mgr).await;
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
        let mgr = self.backend.mcp.clone();
        let tx = self.tx.clone();
        self.backend.spawn(async move {
            match mcp::set_enabled(&mgr, enabled).await {
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
        let mgr = self.backend.mcp.clone();
        let tx = self.tx.clone();
        self.backend.spawn(async move {
            match mcp::set_port(&mgr, port).await {
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
        let mgr = self.backend.mcp.clone();
        let tx = self.tx.clone();
        self.backend.spawn(async move {
            match mcp::rotate_token(&mgr).await {
                Ok(status) => {
                    let _ = tx.try_send(Msg::Status(status));
                }
                Err(error) => {
                    let _ = tx.try_send(Msg::Failed(error));
                }
            }
        });
    }

    fn copy_snippet(&mut self, cx: &mut Context<Self>) {
        let Some(status) = self.status.clone() else {
            return;
        };
        let snippet = snippet(&status);
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

fn snippet(status: &Status) -> String {
    let value = serde_json::json!({
        "mcpServers": {
            "onecrt": {
                "type": "http",
                "url": status.url,
                "headers": { "Authorization": format!("Bearer {}", status.token) },
            }
        }
    });
    serde_json::to_string_pretty(&value).unwrap_or_default()
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

        let Some(status) = self.status.clone() else {
            return div().text_size(px(12.)).text_color(muted).child("…");
        };

        let port_value = self.port.read(cx).value().trim().to_string();
        let port_next = port_value.parse::<u16>().ok();
        let port_changed = port_next.is_some_and(|p| p != status.port);

        let toggle = Switch::new("mcp-enabled")
            .checked(status.enabled)
            .label(tr(lang, "启用 MCP 服务", "Enable MCP server"))
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
                        "把已保存的会话开放给外部智能体当 MCP 工具调用。默认关闭，只监听本机回环地址，调用必须带令牌；智能体看不到口令本身。",
                        "Expose saved sessions to external agents as MCP tools. Off by default, bound to loopback only, every call requires the token; agents never see passwords.",
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
                            .label(tr(lang, "重新生成", "Regenerate"))
                            .on_click(cx.listener(|this, _, _, _cx| this.rotate_token())),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.))
                            .text_color(if status.running { success } else { muted })
                            .child(if status.running {
                                SharedString::from(format!(
                                    "{} {}",
                                    tr(lang, "监听中", "Listening"),
                                    status.url
                                ))
                            } else {
                                tr(lang, "未监听", "Not listening")
                            }),
                    )
                    .child(
                        Button::new("mcp-copy")
                            .label(if self.copied {
                                tr(lang, "已复制", "Copied")
                            } else {
                                tr(lang, "复制接入配置", "Copy config")
                            })
                            .disabled(status.token.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| this.copy_snippet(cx))),
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
    let mgr = backend.mcp.clone();
    let initial = backend
        .handle()
        .block_on(async move { mcp::status(&mgr).await });
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
                    .primary()
                    .label(tr(self.lang, "关闭", "Close"))
                    .on_click(|_, window, _| window.remove_window()),
            ),
        )
    }
}
