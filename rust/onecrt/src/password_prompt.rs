//! 口令对话框：会话里没存口令时，连接前先问一次，可选“保存到会话”。

use gpui_kit as gpui;

use gpui::{
    div, px, AnyWindowHandle, AppContext, Context, Entity, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
};
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    input::{Input, InputState},
    ActiveTheme,
};

use crate::dialog_window;
use crate::i18n::{tr, Lang};

pub struct PasswordPrompt {
    lang: Lang,
    user: Entity<InputState>,
    password: Entity<InputState>,
    save: bool,
}

impl PasswordPrompt {
    fn new(window: &mut Window, cx: &mut Context<Self>, lang: Lang, username: &str) -> Self {
        let username = username.to_string();
        let user = cx.new(|cx| InputState::new(window, cx).default_value(username));
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));
        Self {
            lang,
            user,
            password,
            save: false,
        }
    }

    fn values(&self, cx: &gpui::App) -> (String, String) {
        (
            self.user.read(cx).value().trim().to_string(),
            self.password.read(cx).value().to_string(),
        )
    }
}

impl Render for PasswordPrompt {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let primary = theme.primary;
        let hover_bg = theme.secondary;
        let radius = theme.radius;
        let lang = self.lang;
        let save = self.save;
        let toggle = div()
            .id("pwd-save")
            .h(px(24.))
            .px(px(6.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(radius)
            .cursor_pointer()
            .hover(move |s| s.bg(hover_bg))
            .on_click(cx.listener(|this, _, _, cx| {
                this.save = !this.save;
                cx.notify();
            }))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(if save { primary } else { muted })
                    .child(if save { "☑" } else { "☐" }),
            )
            .child(div().text_size(px(12.)).text_color(fg).child(tr(
                lang,
                "保存到会话",
                "Save to session",
            )));

        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(div().text_size(px(12.)).text_color(muted).child(tr(
                lang,
                "该会话没有保存口令，请输入后继续。",
                "This session has no saved password. Enter one to continue.",
            )))
            .child(field(muted, tr(lang, "用户名", "Username"), &self.user))
            .child(field(muted, tr(lang, "口令", "Password"), &self.password))
            .child(toggle)
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

/// `on_submit` 收到 (用户名, 口令, 是否保存)；`on_skip` 只带用户名，交给后端决定能不能免密。
pub fn open_password_prompt(
    window: &mut Window,
    cx: &mut gpui::App,
    lang: Lang,
    username: &str,
    on_submit: impl Fn(String, String, bool, &mut Window, &mut gpui::App) + 'static,
    on_skip: impl Fn(String, &mut Window, &mut gpui::App) + 'static,
) {
    let parent = window.window_handle();
    let submit: std::rc::Rc<dyn Fn(String, String, bool, &mut Window, &mut gpui::App)> =
        std::rc::Rc::new(on_submit);
    let skip: std::rc::Rc<dyn Fn(String, &mut Window, &mut gpui::App)> = std::rc::Rc::new(on_skip);
    let username = username.to_string();
    dialog_window::open(
        window,
        cx,
        tr(lang, "需要口令", "Password Required"),
        400.,
        330.,
        move |window, cx| {
            let form = cx.new(|cx| PasswordPrompt::new(window, cx, lang, &username));
            form.read(cx)
                .password
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
            cx.new(|_| PasswordWindow {
                form,
                lang,
                parent,
                submit,
                skip,
            })
        },
    );
}

struct PasswordWindow {
    form: Entity<PasswordPrompt>,
    lang: Lang,
    parent: AnyWindowHandle,
    submit: std::rc::Rc<dyn Fn(String, String, bool, &mut Window, &mut gpui::App)>,
    skip: std::rc::Rc<dyn Fn(String, &mut Window, &mut gpui::App)>,
}

impl Render for PasswordWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = self.lang;
        let parent = self.parent;
        let for_submit = self.form.clone();
        let for_skip = self.form.clone();
        let submit = self.submit.clone();
        let skip = self.skip.clone();
        dialog_window::page(
            cx,
            self.form.clone(),
            dialog_window::buttons()
                .child(
                    Button::new("pwd-cancel")
                        .label(tr(lang, "取消", "Cancel"))
                        .on_click(|_, window, _| window.remove_window()),
                )
                .child(
                    Button::new("pwd-skip")
                        .label(tr(lang, "跳过", "Skip"))
                        .on_click(move |_, window, cx| {
                            let (user, _) = for_skip.read(cx).values(cx);
                            if !user.is_empty() {
                                dialog_window::run_in_parent(parent, cx, |window, cx| {
                                    skip(user, window, cx)
                                });
                            }
                            window.remove_window();
                        }),
                )
                .child(
                    Button::new("pwd-ok")
                        .primary()
                        .label(tr(lang, "确定", "OK"))
                        .on_click(move |_, window, cx| {
                            let (user, password) = for_submit.read(cx).values(cx);
                            if user.is_empty() || password.is_empty() {
                                return;
                            }
                            let save = for_submit.read(cx).save;
                            dialog_window::run_in_parent(parent, cx, |window, cx| {
                                submit(user, password, save, window, cx)
                            });
                            window.remove_window();
                        }),
                ),
        )
    }
}
