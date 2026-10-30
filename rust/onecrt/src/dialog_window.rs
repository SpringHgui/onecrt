//! 弹窗的原生窗口外壳。
//!
//! 所有弹窗都开成独立的 `WindowKind::Dialog` 窗口：系统会把打开它的窗口禁用来
//! 充当模态父级，弹窗关掉后再自动恢复。弹窗里的“确认 / 保存”要回到父窗口执行
//! （开标签页、落盘、接着弹下一个窗口），所以每个弹窗都记住父窗口句柄。

use std::rc::Rc;
use std::time::Duration;

use gpui_kit as gpui;

use gpui::{
    div, point, px, size, AnyWindowHandle, App, AppContext, Bounds, Context, Div, Entity,
    IntoElement, ParentElement, Render, SharedString, Styled, TitlebarOptions, Window,
    WindowBounds, WindowKind, WindowOptions,
};
use gpui_kit::component::ActiveTheme;

/// 打开一个居中于父窗口的模态弹窗。
pub fn open<V: Render + 'static>(
    parent: &mut Window,
    cx: &mut App,
    title: impl Into<SharedString>,
    width: f32,
    height: f32,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) {
    let bounds = parent.bounds();
    let dialog_size = size(px(width), px(height));
    let origin = point(
        bounds.origin.x + (bounds.size.width - dialog_size.width) / 2.,
        bounds.origin.y + (bounds.size.height - dialog_size.height) / 2.,
    );
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(origin, dialog_size))),
        titlebar: Some(TitlebarOptions {
            title: Some(title.into()),
            appears_transparent: false,
            traffic_light_position: None,
        }),
        kind: WindowKind::Dialog,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    if let Err(error) = cx.open_window(options, move |window, cx| build(window, cx)) {
        eprintln!("打开弹窗失败: {error}");
    }
}

/// 弹窗的统一版式：内容区 + 底部按钮条。
pub fn page(cx: &App, content: impl IntoElement, footer: impl IntoElement) -> Div {
    let theme = cx.theme();
    div()
        .size_full()
        .flex()
        .flex_col()
        .bg(theme.background)
        .text_color(theme.foreground)
        .font_family(theme.font_family.clone())
        .text_size(theme.font_size)
        .child(div().flex_1().min_h_0().p(px(16.)).child(content))
        .child(
            div()
                .flex_shrink_0()
                .h(px(52.))
                .px(px(16.))
                .flex()
                .items_center()
                .justify_end()
                .gap(px(8.))
                .border_t_1()
                .border_color(theme.border)
                .child(footer),
        )
}

/// 底部按钮条，按钮从右往左排。
pub fn buttons() -> Div {
    div().flex().items_center().gap(px(8.))
}

/// 在父窗口里执行弹窗交回的动作。
pub fn run_in_parent(
    parent: AnyWindowHandle,
    cx: &mut App,
    action: impl FnOnce(&mut Window, &mut App),
) {
    let _ = parent.update(cx, |_, window, cx| action(window, cx));
}

/// 先关掉当前弹窗，再回到父窗口接着弹下一个窗口。
///
/// 模态父级取的是系统当前的“活动窗口”，旧窗口的 HWND 还没销毁时新窗口会被挂成
/// 它的子窗口并跟着一起销毁，所以先等旧窗口真正消失。
pub fn run_after_close(
    parent: AnyWindowHandle,
    cx: &mut App,
    action: impl FnOnce(&mut Window, &mut App) + 'static,
) {
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(Duration::from_millis(120))
            .await;
        cx.update(|cx| run_in_parent(parent, cx, action));
    })
    .detach();
}

/// 只有一段说明、一个确认按钮的确认框。
pub fn open_confirm(
    parent: &mut Window,
    cx: &mut App,
    lang: crate::i18n::Lang,
    title: SharedString,
    message: SharedString,
    ok_label: SharedString,
    danger: bool,
    on_ok: impl Fn(&mut Window, &mut App) + 'static,
) {
    let parent_handle = parent.window_handle();
    let on_ok: Rc<dyn Fn(&mut Window, &mut App)> = Rc::new(on_ok);
    open(parent, cx, title, 380., 168., move |_, cx| {
        cx.new(|_| ConfirmWindow {
            lang,
            message,
            ok_label,
            danger,
            parent: parent_handle,
            on_ok,
        })
    });
}

struct ConfirmWindow {
    lang: crate::i18n::Lang,
    message: SharedString,
    ok_label: SharedString,
    danger: bool,
    parent: AnyWindowHandle,
    on_ok: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl Render for ConfirmWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::button::{Button, ButtonVariants};

        let lang = self.lang;
        let parent = self.parent;
        let confirm = self.on_ok.clone();
        let mut ok = Button::new("confirm-ok").label(self.ok_label.clone());
        ok = if self.danger {
            ok.danger()
        } else {
            ok.primary()
        };
        page(
            cx,
            div()
                .text_size(px(12.))
                .text_color(cx.theme().muted_foreground)
                .child(self.message.clone()),
            buttons()
                .child(
                    Button::new("confirm-cancel")
                        .label(crate::i18n::tr(lang, "取消", "Cancel"))
                        .on_click(|_, window, _| window.remove_window()),
                )
                .child(ok.on_click(move |_, window, cx| {
                    run_in_parent(parent, cx, |window, cx| confirm(window, cx));
                    window.remove_window();
                })),
        )
    }
}
