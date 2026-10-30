//! 快速连接：不走会话树，填完主机直接开一个标签页。

use std::rc::Rc;

use gpui_kit as gpui;

use gpui::{
    div, px, AnyWindowHandle, App, AppContext, Context, Entity, Focusable, IntoElement,
    ParentElement, Render, SharedString, Styled, Window,
};
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    input::{Input, InputState},
    ActiveTheme,
};
use onecrt_core::sessions::{default_auth, SshSession};

use crate::dialog_window;
use crate::i18n::{tr, Lang};
use crate::term_view::ConnectSpec;

pub struct QuickConnect {
    lang: Lang,
    host: Entity<InputState>,
    port: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    key_path: Entity<InputState>,
    error: Option<SharedString>,
}

impl QuickConnect {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        lang: Lang,
        initial: Option<&SshSession>,
    ) -> Self {
        let mut input = |cx: &mut Context<Self>, value: String, placeholder: &str, masked: bool| {
            let placeholder = placeholder.to_string();
            cx.new(|cx| {
                let state = InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(value);
                if masked {
                    state.masked(true)
                } else {
                    state
                }
            })
        };
        Self {
            lang,
            host: input(
                cx,
                initial.map(|s| s.host.clone()).unwrap_or_default(),
                &tr(lang, "host 或 IP", "host or IP"),
                false,
            ),
            port: input(
                cx,
                initial
                    .map(|s| s.port.to_string())
                    .unwrap_or_else(|| "22".into()),
                "22",
                false,
            ),
            username: input(
                cx,
                initial
                    .map(|s| s.username.clone())
                    .unwrap_or_else(|| "root".into()),
                &tr(lang, "用户名", "Username"),
                false,
            ),
            password: input(
                cx,
                initial.map(|s| s.password.clone()).unwrap_or_default(),
                &tr(lang, "密码（可留空）", "Password (optional)"),
                true,
            ),
            key_path: input(
                cx,
                initial.map(|s| s.key_path.clone()).unwrap_or_default(),
                &tr(lang, "~/.ssh/id_rsa（可留空）", "~/.ssh/id_rsa (optional)"),
                false,
            ),
            error: None,
        }
    }

    fn spec(&self, cx: &App) -> Result<(String, ConnectSpec), SharedString> {
        let host = self.host.read(cx).value().trim().to_string();
        if host.is_empty() {
            return Err(tr(self.lang, "请填写主机地址", "Host is required"));
        }
        let port = self
            .port
            .read(cx)
            .value()
            .trim()
            .parse::<u16>()
            .unwrap_or(22);
        Ok((
            host.clone(),
            ConnectSpec {
                host,
                port,
                username: self.username.read(cx).value().trim().to_string(),
                password: self.password.read(cx).value().to_string(),
                key_path: self.key_path.read(cx).value().trim().to_string(),
                methods: default_auth()
                    .into_iter()
                    .filter(|m| m.enabled)
                    .map(|m| m.method)
                    .collect(),
            },
        ))
    }

    fn set_error(&mut self, error: SharedString, cx: &mut Context<Self>) {
        self.error = Some(error);
        cx.notify();
    }
}

fn field(muted: gpui::Hsla, text: SharedString, input: &Entity<InputState>) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(div().text_size(px(11.)).text_color(muted).child(text))
        .child(Input::new(input))
}

impl Render for QuickConnect {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let danger = theme.danger;
        let lang = self.lang;
        let mut root = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(div().flex_1().child(field(muted, tr(lang, "主机", "Host"), &self.host)))
                    .child(div().w(px(90.)).child(field(muted, tr(lang, "端口", "Port"), &self.port))),
            )
            .child(field(muted, tr(lang, "用户名", "Username"), &self.username))
            .child(field(muted, tr(lang, "密码", "Password"), &self.password))
            .child(field(muted, tr(lang, "身份文件", "Identity file"), &self.key_path))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(muted)
                    .child(tr(
                        lang,
                        "认证顺序与会话属性一致：公钥 → 密码 → 键盘交互。",
                        "Authentication order follows session properties: public key → password → keyboard interactive.",
                    )),
            );
        if let Some(error) = self.error.clone() {
            root = root.child(div().text_size(px(11.)).text_color(danger).child(error));
        }
        root
    }
}

/// 打开快速连接对话框；成功时把 (标题, 连接参数) 交给主界面开标签页。
pub fn open_quick_connect(
    window: &mut Window,
    cx: &mut App,
    lang: Lang,
    initial: Option<SshSession>,
    on_connect: impl Fn(String, ConnectSpec, &mut Window, &mut App) + 'static,
) {
    let parent = window.window_handle();
    let on_connect: Rc<dyn Fn(String, ConnectSpec, &mut Window, &mut App)> = Rc::new(on_connect);
    dialog_window::open(
        window,
        cx,
        tr(lang, "快速连接", "Quick Connect"),
        440.,
        404.,
        move |window, cx| {
            let form = cx.new(|cx| QuickConnect::new(window, cx, lang, initial.as_ref()));
            form.read(cx)
                .host
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
            cx.new(|_| QuickConnectWindow {
                form,
                lang,
                parent,
                on_connect,
            })
        },
    );
}

struct QuickConnectWindow {
    form: Entity<QuickConnect>,
    lang: Lang,
    parent: AnyWindowHandle,
    on_connect: Rc<dyn Fn(String, ConnectSpec, &mut Window, &mut App)>,
}

impl Render for QuickConnectWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = self.lang;
        let parent = self.parent;
        let for_connect = self.form.clone();
        let connect = self.on_connect.clone();
        dialog_window::page(
            cx,
            self.form.clone(),
            dialog_window::buttons()
                .child(
                    Button::new("qc-cancel")
                        .label(tr(lang, "取消", "Cancel"))
                        .on_click(|_, window, _| window.remove_window()),
                )
                .child(
                    Button::new("qc-connect")
                        .primary()
                        .label(tr(lang, "连接", "Connect"))
                        .on_click(move |_, window, cx| match for_connect.read(cx).spec(cx) {
                            Ok((title, spec)) => {
                                dialog_window::run_in_parent(parent, cx, |window, cx| {
                                    connect(title, spec, window, cx)
                                });
                                window.remove_window();
                            }
                            Err(error) => {
                                for_connect.update(cx, |form, cx| form.set_error(error, cx));
                            }
                        }),
                ),
        )
    }
}
