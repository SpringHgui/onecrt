//! 关于对话框：版本、运行时与数据目录，不联网。

use gpui_kit as gpui;

use gpui::{
    div, px, AnyWindowHandle, App, AppContext, Context, IntoElement, ParentElement, Render,
    SharedString, Styled, Window,
};
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    notification::Notification,
    ActiveTheme, WindowExt,
};

use crate::dialog_window;
use crate::i18n::{tr, Lang};
use crate::update;

const REPO: &str = "https://github.com/SpringHgui/onecrt";

pub fn open_about(window: &mut Window, cx: &mut App, lang: Lang) {
    let parent = window.window_handle();
    dialog_window::open(
        window,
        cx,
        tr(lang, "关于 OneCRT", "About OneCRT"),
        460.,
        352.,
        move |_, cx| cx.new(|_| AboutWindow { lang, parent }),
    );
}

struct AboutWindow {
    lang: Lang,
    parent: AnyWindowHandle,
}

impl Render for AboutWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = self.lang;
        let parent = self.parent;
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let data_dir = dirs::config_dir()
            .map(|p| p.join("shellx").to_string_lossy().into_owned())
            .unwrap_or_else(|| "—".into());
        let version = env!("CARGO_PKG_VERSION");
        dialog_window::page(
            cx,
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(
                            gpui::img(std::sync::Arc::new(gpui::Image::from_bytes(
                                gpui::ImageFormat::Png,
                                include_bytes!("../assets/logo.png").to_vec(),
                            )))
                            .size(px(44.)),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_size(px(20.))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(fg)
                                        .child("OneCRT"),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(muted)
                                        .child(SharedString::from(format!(
                                            "v{version} · gpui · pure Rust"
                                        ))),
                                ),
                        ),
                )
                .child(
                    div().text_size(px(12.)).text_color(fg).child(tr(
                        lang,
                        "跨平台 SSH 客户端：会话管理、快速连接、命令库、SFTP 文件面板，外加一个给智能体用的 MCP 服务。",
                        "Cross-platform SSH client: sessions, quick connect, command library, SFTP panel, plus an MCP server for agents.",
                    )),
                )
                .child(
                    div().text_size(px(11.)).text_color(muted).child(tr(
                        lang,
                        "会话与命令保存在下面的数据目录；Windows 上口令经 DPAPI 加密后落盘，Linux 与 macOS 为明文。仅面向内网运维，请勿用于对外分发的机器。",
                        "Sessions and commands live in the data directory below. Passwords are DPAPI-encrypted on Windows and plain text on Linux/macOS. Intended for internal ops only.",
                    )),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .text_size(px(11.))
                        .text_color(muted)
                        .child(SharedString::from(format!(
                            "{} {data_dir}",
                            tr(lang, "数据目录", "Data directory")
                        )))
                        .child(SharedString::from(format!("{REPO}"))),
                ),
            dialog_window::buttons()
                .child(
                    Button::new("about-update")
                        .label(tr(lang, "检查更新", "Check for updates"))
                        .on_click(move |_, window, cx| {
                            let me = window.window_handle();
                            check_for_update(me, parent, cx, lang);
                        }),
                )
                .child(
                    Button::new("about-repo")
                        .label(tr(lang, "项目主页", "Project home"))
                        .on_click(|_, _, cx| cx.open_url(REPO)),
                )
                .child(
                    Button::new("about-close")
                        .primary()
                        .label(tr(lang, "关闭", "Close"))
                        .on_click(|_, window, _| window.remove_window()),
                ),
        )
    }
}

fn check_for_update(me: AnyWindowHandle, parent: AnyWindowHandle, cx: &mut App, lang: Lang) {
    let (tx, rx) = async_channel::bounded(1);
    let current = env!("CARGO_PKG_VERSION").to_string();
    let probe = current.clone();
    std::thread::spawn(move || {
        let _ = tx.send_blocking(update::check(&probe));
    });
    cx.spawn(async move |cx| {
        let Ok(result) = rx.recv().await else {
            return;
        };
        cx.update(|cx| match result {
            Ok(None) => {
                let message: SharedString = if lang == Lang::Zh {
                    format!("已是最新版本 v{current}").into()
                } else {
                    format!("You are up to date (v{current})").into()
                };
                notify(parent, cx, Notification::success(message));
            }
            Ok(Some(latest)) => {
                let _ = me.update(cx, |_, window, _| window.remove_window());
                dialog_window::run_after_close(parent, cx, move |window, cx| {
                    show_update(window, cx, lang, &current, latest)
                });
            }
            Err(error) => {
                let message: SharedString = if lang == Lang::Zh {
                    format!("检查更新失败：{error}").into()
                } else {
                    format!("Update check failed: {error}").into()
                };
                notify(parent, cx, Notification::error(message));
            }
        });
    })
    .detach();
}

/// 通知要落在主窗口上：弹窗窗口里没有通知层。
fn notify(parent: AnyWindowHandle, cx: &mut App, note: Notification) {
    let _ = parent.update(cx, |_, window, cx| window.push_notification(note, cx));
}

fn show_update(
    window: &mut Window,
    cx: &mut App,
    lang: Lang,
    current: &str,
    latest: update::Latest,
) {
    let url = std::rc::Rc::new(latest.url);
    let version = latest.version;
    let title = tr(lang, "发现新版本", "Update available");
    let heading: SharedString = if lang == Lang::Zh {
        format!("OneCRT v{version} 已发布").into()
    } else {
        format!("OneCRT v{version} is out").into()
    };
    let detail: SharedString = if lang == Lang::Zh {
        format!("当前运行 v{current}。到发布页下载新包，替换后重启即可；本版本不提供自动安装。")
            .into()
    } else {
        format!(
            "You are running v{current}. Download the new build from the release page and restart after replacing it; this build does not self-install."
        )
        .into()
    };
    dialog_window::open(window, cx, title, 420., 200., move |_, cx| {
        cx.new(|_| UpdateWindow {
            lang,
            heading,
            detail,
            url,
        })
    });
}

struct UpdateWindow {
    lang: Lang,
    heading: SharedString,
    detail: SharedString,
    url: std::rc::Rc<String>,
}

impl Render for UpdateWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = self.lang;
        let url = self.url.clone();
        dialog_window::page(
            cx,
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(self.heading.clone()),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(cx.theme().muted_foreground)
                        .child(self.detail.clone()),
                ),
            dialog_window::buttons()
                .child(
                    Button::new("update-close")
                        .label(tr(lang, "关闭", "Close"))
                        .on_click(|_, window, _| window.remove_window()),
                )
                .child(
                    Button::new("update-open")
                        .primary()
                        .label(tr(lang, "打开下载页", "Open download page"))
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                ),
        )
    }
}
