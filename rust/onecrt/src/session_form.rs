//! 会话属性表单：新建 / 编辑共用，作为实体渲染在独立的弹窗窗口里。

use std::rc::Rc;

use gpui_kit as gpui;

use gpui::{
    div, px, AnyWindowHandle, App, AppContext, Context, Entity, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
};
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    input::{Input, InputState},
    ActiveTheme,
};
use onecrt_core::sessions::{AuthMethod, SshSession};

use crate::dialog_window;
use crate::i18n::{tr, Lang};

const METHODS: [(&str, &str, &str); 3] = [
    ("PublicKey", "公钥", "Public key"),
    ("Password", "密码", "Password"),
    ("Keyboard Interactive", "键盘交互", "Keyboard interactive"),
];

/// 表单提交出来的纯数据，由主界面负责落到会话树上。
pub struct SessionDraft {
    pub editing: Option<String>,
    pub parent: Option<String>,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub key_path: String,
    pub auth: Vec<AuthMethod>,
}

pub struct SessionForm {
    lang: Lang,
    name: Entity<InputState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    key_path: Entity<InputState>,
    auth: Vec<(String, bool)>,
    editing: Option<String>,
    parent: Option<String>,
    error: Option<SharedString>,
}

impl SessionForm {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        lang: Lang,
        initial: Option<&SshSession>,
        parent: Option<String>,
    ) -> Self {
        let mut input = |cx: &mut Context<Self>, value: &str, placeholder: &str, masked: bool| {
            let value = value.to_string();
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

        let auth = METHODS
            .iter()
            .map(|(method, _, _)| {
                let enabled = initial
                    .and_then(|s| s.auth.iter().find(|a| a.method == *method))
                    .map(|a| a.enabled)
                    .unwrap_or(true);
                (method.to_string(), enabled)
            })
            .collect();

        Self {
            lang,
            name: input(
                cx,
                initial.map(|s| s.name.as_str()).unwrap_or(""),
                &tr(lang, "名称", "Name"),
                false,
            ),
            host: input(
                cx,
                initial.map(|s| s.host.as_str()).unwrap_or(""),
                &tr(lang, "host 或 IP", "host or IP"),
                false,
            ),
            port: input(
                cx,
                &initial
                    .map(|s| s.port.to_string())
                    .unwrap_or_else(|| "22".into()),
                "22",
                false,
            ),
            username: input(
                cx,
                initial.map(|s| s.username.as_str()).unwrap_or("root"),
                &tr(lang, "用户名", "Username"),
                false,
            ),
            password: input(
                cx,
                initial.map(|s| s.password.as_str()).unwrap_or(""),
                &tr(lang, "密码（可留空）", "Password (optional)"),
                true,
            ),
            key_path: input(
                cx,
                initial.map(|s| s.key_path.as_str()).unwrap_or(""),
                &tr(lang, "~/.ssh/id_rsa（可留空）", "~/.ssh/id_rsa (optional)"),
                false,
            ),
            auth,
            editing: initial.map(|s| s.id.clone()),
            parent,
            error: None,
        }
    }

    pub fn draft(&self, cx: &App) -> Result<SessionDraft, SharedString> {
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
        let name = self.name.read(cx).value().trim().to_string();
        Ok(SessionDraft {
            editing: self.editing.clone(),
            parent: self.parent.clone(),
            name: if name.is_empty() { host.clone() } else { name },
            host,
            port,
            username: self.username.read(cx).value().trim().to_string(),
            password: self.password.read(cx).value().to_string(),
            key_path: self.key_path.read(cx).value().trim().to_string(),
            auth: self
                .auth
                .iter()
                .map(|(method, enabled)| AuthMethod {
                    method: method.clone(),
                    enabled: *enabled,
                })
                .collect(),
        })
    }

    pub fn set_error(&mut self, error: SharedString, cx: &mut Context<Self>) {
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

impl Render for SessionForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let primary = theme.primary;
        let hover_bg = theme.secondary;
        let radius = theme.radius;
        let danger = theme.danger;
        let lang = self.lang;
        let mut auth_rows = div().flex().flex_col().gap(px(2.));
        for (index, (method, enabled)) in self.auth.iter().enumerate() {
            let (_, zh, en) = METHODS
                .iter()
                .find(|(m, _, _)| m == method)
                .copied()
                .unwrap_or((method.as_str(), "", ""));
            let title = tr(lang, zh, en);
            let method = method.clone();
            let row = div()
                .id(SharedString::from(format!("auth-{index}")))
                .h(px(26.))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(radius)
                .cursor_pointer()
                .hover(move |s| s.bg(hover_bg))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(entry) = this.auth.iter_mut().find(|(m, _)| m == &method) {
                        entry.1 = !entry.1;
                    }
                    cx.notify();
                }))
                .child(
                    div()
                        .w(px(14.))
                        .text_size(px(12.))
                        .text_color(if *enabled { primary } else { muted })
                        .child(if *enabled { "✓" } else { "·" }),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12.))
                        .text_color(if *enabled { fg } else { muted })
                        .child(title),
                );
            auth_rows = auth_rows.child(row);
        }

        let mut root = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(field(muted, tr(lang, "名称", "Name"), &self.name))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .child(field(muted, tr(lang, "主机", "Host"), &self.host)),
                    )
                    .child(div().w(px(90.)).child(field(
                        muted,
                        tr(lang, "端口", "Port"),
                        &self.port,
                    ))),
            )
            .child(field(muted, tr(lang, "用户名", "Username"), &self.username))
            .child(field(muted, tr(lang, "密码", "Password"), &self.password))
            .child(field(
                muted,
                tr(lang, "身份文件", "Identity file"),
                &self.key_path,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(div().text_size(px(11.)).text_color(muted).child(tr(
                        lang,
                        "认证顺序（自上而下依次尝试）",
                        "Authentication order (tried top to bottom)",
                    )))
                    .child(auth_rows),
            );
        if let Some(error) = self.error.clone() {
            root = root.child(div().text_size(px(11.)).text_color(danger).child(error));
        }
        root
    }
}

/// 打开会话属性对话框；保存成功后通过 `on_save` 交回主界面。
pub fn open_session_form(
    window: &mut Window,
    cx: &mut App,
    lang: Lang,
    initial: Option<SshSession>,
    parent: Option<String>,
    on_save: impl Fn(SessionDraft, &mut Window, &mut App) + 'static,
) {
    let parent_window = window.window_handle();
    let on_save: Rc<dyn Fn(SessionDraft, &mut Window, &mut App)> = Rc::new(on_save);
    dialog_window::open(
        window,
        cx,
        tr(lang, "会话属性", "Session Properties"),
        480.,
        548.,
        move |window, cx| {
            let form = cx.new(|cx| SessionForm::new(window, cx, lang, initial.as_ref(), parent));
            form.read(cx)
                .name
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
            cx.new(|_| SessionFormWindow {
                form,
                lang,
                parent: parent_window,
                on_save,
            })
        },
    );
}

struct SessionFormWindow {
    form: Entity<SessionForm>,
    lang: Lang,
    parent: AnyWindowHandle,
    on_save: Rc<dyn Fn(SessionDraft, &mut Window, &mut App)>,
}

impl Render for SessionFormWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = self.lang;
        let parent = self.parent;
        let for_save = self.form.clone();
        let save = self.on_save.clone();
        dialog_window::page(
            cx,
            self.form.clone(),
            dialog_window::buttons()
                .child(
                    Button::new("session-cancel")
                        .label(tr(lang, "取消", "Cancel"))
                        .on_click(|_, window, _| window.remove_window()),
                )
                .child(
                    Button::new("session-save")
                        .primary()
                        .label(tr(lang, "保存", "Save"))
                        .on_click(move |_, window, cx| match for_save.read(cx).draft(cx) {
                            Ok(draft) => {
                                dialog_window::run_in_parent(parent, cx, |window, cx| {
                                    save(draft, window, cx)
                                });
                                window.remove_window();
                            }
                            Err(error) => {
                                for_save.update(cx, |form, cx| form.set_error(error, cx));
                            }
                        }),
                ),
        )
    }
}

/// 单行文本对话框（新建分组、重命名都用它）
pub fn open_text_prompt(
    window: &mut Window,
    cx: &mut App,
    lang: Lang,
    title: SharedString,
    initial: &str,
    placeholder: SharedString,
    on_save: impl Fn(String, &mut Window, &mut App) + 'static,
) {
    let parent = window.window_handle();
    let initial = initial.to_string();
    let on_save: Rc<dyn Fn(String, &mut Window, &mut App)> = Rc::new(on_save);
    dialog_window::open(window, cx, title, 360., 146., move |window, cx| {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .default_value(initial)
        });
        input.read(cx).focus_handle(cx).focus(window, cx);
        cx.new(|_| TextPromptWindow {
            input,
            lang,
            parent,
            on_save,
        })
    });
}

struct TextPromptWindow {
    input: Entity<InputState>,
    lang: Lang,
    parent: AnyWindowHandle,
    on_save: Rc<dyn Fn(String, &mut Window, &mut App)>,
}

impl Render for TextPromptWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = self.lang;
        let parent = self.parent;
        let save_input = self.input.clone();
        let save = self.on_save.clone();
        dialog_window::page(
            cx,
            Input::new(&self.input),
            dialog_window::buttons()
                .child(
                    Button::new("prompt-cancel")
                        .label(tr(lang, "取消", "Cancel"))
                        .on_click(|_, window, _| window.remove_window()),
                )
                .child(
                    Button::new("prompt-ok")
                        .primary()
                        .label(tr(lang, "确定", "OK"))
                        .on_click(move |_, window, cx| {
                            let value = save_input.read(cx).value().trim().to_string();
                            if !value.is_empty() {
                                dialog_window::run_in_parent(parent, cx, |window, cx| {
                                    save(value, window, cx)
                                });
                            }
                            window.remove_window();
                        }),
                ),
        )
    }
}
