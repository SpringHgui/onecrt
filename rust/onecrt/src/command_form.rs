//! 命令属性对话框：显示名与实际发送文本分开存储，命令文本可多行。

use std::rc::Rc;

use gpui_kit as gpui;

use gpui::{
    div, px, AnyWindowHandle, App, AppContext, Context, Entity, Focusable, IntoElement,
    ParentElement, Render, SharedString, Styled, Window,
};
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    input::{Input, InputState, Textarea, TextareaState},
    ActiveTheme,
};

use crate::dialog_window;
use crate::i18n::{tr, Lang};

pub struct CommandFields {
    pub name: String,
    pub text: String,
}

pub struct CommandForm {
    lang: Lang,
    show_text: bool,
    name: Entity<InputState>,
    text: Entity<TextareaState>,
    error: Option<SharedString>,
}

impl CommandForm {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        lang: Lang,
        initial: Option<&(String, String)>,
        show_text: bool,
    ) -> Self {
        let name_value = initial.map(|(name, _)| name.clone()).unwrap_or_default();
        let text_value = initial.map(|(_, text)| text.clone()).unwrap_or_default();
        Self {
            lang,
            show_text,
            name: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(tr(lang, "显示名称", "Display name"))
                    .default_value(name_value)
            }),
            text: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .placeholder(tr(
                        lang,
                        "发往终端的命令，可用多行",
                        "Command sent to the terminal, multi-line allowed",
                    ))
                    .default_value(text_value)
            }),
            error: None,
        }
    }

    fn fields(&self, cx: &App) -> Result<CommandFields, SharedString> {
        let name = self.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            return Err(tr(self.lang, "请填写名称", "Name is required"));
        }
        let text = self.text.read(cx).value().trim().to_string();
        Ok(CommandFields {
            text: if text.is_empty() { name.clone() } else { text },
            name,
        })
    }

    fn set_error(&mut self, error: SharedString, cx: &mut Context<Self>) {
        self.error = Some(error);
        cx.notify();
    }
}

fn field(muted: gpui::Hsla, label: SharedString, child: impl IntoElement) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(div().text_size(px(11.)).text_color(muted).child(label))
        .child(child)
}

impl Render for CommandForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let danger = theme.danger;
        let lang = self.lang;
        let mut root = div().flex().flex_col().gap(px(10.)).child(field(
            muted,
            tr(lang, "名称", "Name"),
            Input::new(&self.name),
        ));
        if self.show_text {
            root = root.child(field(
                muted,
                tr(lang, "命令", "Command"),
                Textarea::new(&self.text).h(px(96.)),
            ));
        }
        if let Some(error) = self.error.clone() {
            root = root.child(div().text_size(px(11.)).text_color(danger).child(error));
        }
        root
    }
}

/// 打开命令属性对话框；文件夹传 `show_text = false`，只改名字。
pub fn open_command_form(
    window: &mut Window,
    cx: &mut App,
    lang: Lang,
    title: SharedString,
    initial: Option<(String, String)>,
    show_text: bool,
    on_save: impl Fn(CommandFields, &mut Window, &mut App) + 'static,
) {
    let parent = window.window_handle();
    let height = if show_text { 300. } else { 170. };
    let on_save: Rc<dyn Fn(CommandFields, &mut Window, &mut App)> = Rc::new(on_save);
    dialog_window::open(window, cx, title, 440., height, move |window, cx| {
        let form = cx.new(|cx| CommandForm::new(window, cx, lang, initial.as_ref(), show_text));
        form.read(cx)
            .name
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
        cx.new(|_| CommandFormWindow {
            form,
            lang,
            parent,
            on_save,
        })
    });
}

struct CommandFormWindow {
    form: Entity<CommandForm>,
    lang: Lang,
    parent: AnyWindowHandle,
    on_save: Rc<dyn Fn(CommandFields, &mut Window, &mut App)>,
}

impl Render for CommandFormWindow {
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
                    Button::new("cmd-cancel")
                        .label(tr(lang, "取消", "Cancel"))
                        .on_click(|_, window, _| window.remove_window()),
                )
                .child(
                    Button::new("cmd-ok")
                        .primary()
                        .label(tr(lang, "保存", "Save"))
                        .on_click(move |_, window, cx| match for_save.read(cx).fields(cx) {
                            Ok(fields) => {
                                dialog_window::run_in_parent(parent, cx, |window, cx| {
                                    save(fields, window, cx)
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
