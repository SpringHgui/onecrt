//! 关于对话框：版本、运行时与数据目录，不联网。

use gpui_kit as gpui;

use gpui::{
    div, px, AnyWindowHandle, App, AppContext, Context, IntoElement, ParentElement, Render,
    SharedString, Styled, WeakEntity, Window,
};
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    notification::Notification,
    ActiveTheme, Sizable, WindowExt,
};

use crate::dialog_window;
use crate::dialog_window::ActionButton;
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
        move |_, cx| {
            cx.new(|_| AboutWindow {
                lang,
                parent,
                checking: false,
            })
        },
    );
}

struct AboutWindow {
    lang: Lang,
    parent: AnyWindowHandle,
    /// 请求在飞的时候按钮转圈并锁住，免得连点重复请求
    checking: bool,
}

impl Render for AboutWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = self.lang;
        let parent = self.parent;
        let about = cx.entity().downgrade();
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
                .child({
                    let mut check = Button::new("about-update")
                        .loading(self.checking)
                        .action_height()
                        .label(if self.checking {
                            tr(lang, "检查中…", "Checking…")
                        } else {
                            tr(lang, "检查更新", "Check for updates")
                        });
                    // Button 自带的 loading 只在有 icon 时才换成转圈，这里没有 icon，
                    // 手动挂一个 Spinner，否则按下去只剩整颗按钮淡一点
                    if self.checking {
                        check = check.child(
                            gpui::component::spinner::Spinner::new()
                                .with_size(gpui::component::Size::Small),
                        );
                    }
                    check.on_click(move |_, window, cx| {
                        let me = window.window_handle();
                        let about = about.clone();
                        let _ = about.update(cx, |this, cx| {
                            this.checking = true;
                            cx.notify();
                        });
                        check_for_update(me, parent, about, cx, lang);
                    })
                })
                .child(
                    Button::new("about-repo")
                        .label(tr(lang, "项目主页", "Project home"))
                        .action_height()
                        .on_click(|_, _, cx| cx.open_url(REPO)),
                )
                .child(
                    Button::new("about-close")
                        .primary()
                        .label(tr(lang, "关闭", "Close"))
                        .action_height()
                        .on_click(|_, window, _| window.remove_window()),
                ),
        )
    }
}

fn check_for_update(
    me: AnyWindowHandle,
    parent: AnyWindowHandle,
    about: WeakEntity<AboutWindow>,
    cx: &mut App,
    lang: Lang,
) {
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
                finish_check(&about, cx);
                notify(parent, cx, Notification::success(message));
            }
            Ok(Some(latest)) => {
                let _ = me.update(cx, |_, window, _| window.remove_window());
                dialog_window::run_after_close(parent, cx, move |window, cx| {
                    show_update(window, cx, lang, latest)
                });
            }
            Err(error) => {
                let message: SharedString = if lang == Lang::Zh {
                    format!("检查更新失败：{error}").into()
                } else {
                    format!("Update check failed: {error}").into()
                };
                finish_check(&about, cx);
                notify(parent, cx, Notification::error(message));
            }
        });
    })
    .detach();
}

/// 检查结束但没弹出新版本窗口时，把按钮从“检查中…”恢复成可点。
fn finish_check(about: &WeakEntity<AboutWindow>, cx: &mut App) {
    let _ = about.update(cx, |this, cx| {
        this.checking = false;
        cx.notify();
    });
}

/// 通知要落在主窗口上：弹窗窗口里没有通知层。
fn notify(parent: AnyWindowHandle, cx: &mut App, note: Notification) {
    let _ = parent.update(cx, |_, window, cx| window.push_notification(note, cx));
}

fn show_update(window: &mut Window, cx: &mut App, lang: Lang, latest: update::Latest) {
    // 只有拿得到本平台安装包、并且能静默安装时才自动更新；否则退回「打开下载页」
    let auto = update::can_auto_install() && latest.asset.is_some();
    let version = latest.version.clone();
    let url = std::rc::Rc::new(latest.url.clone());
    let status: SharedString = match (auto, lang) {
        (true, Lang::Zh) => format!("正在下载 OneCRT v{version}…").into(),
        (true, Lang::En) => format!("Downloading OneCRT v{version}…").into(),
        (false, Lang::Zh) => {
            format!("OneCRT v{version} 已发布，当前平台请到发布页下载安装。").into()
        }
        (false, Lang::En) => {
            format!("OneCRT v{version} is out; download it from the release page on this platform.")
                .into()
        }
    };
    dialog_window::open(
        window,
        cx,
        tr(lang, "发现新版本", "Update available"),
        420.,
        176.,
        move |_, cx| {
            let entity = cx.new(|_| UpdateWindow {
                lang,
                version: version.clone(),
                status: status.clone(),
                url,
                phase: if auto { Phase::Working } else { Phase::Manual },
                task: None,
            });
            if auto {
                entity.update(cx, |this, cx| this.start(latest, cx));
            }
            entity
        },
    );
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    /// 下载 / 安装进行中
    Working,
    /// 装完了，马上重启
    Restarting,
    /// 出错，只能手动下载
    Failed,
    /// 这个平台不做自动安装
    Manual,
}

struct UpdateWindow {
    lang: Lang,
    version: String,
    status: SharedString,
    url: std::rc::Rc<String>,
    phase: Phase,
    task: Option<gpui::Task<()>>,
}

impl UpdateWindow {
    fn start(&mut self, latest: update::Latest, cx: &mut Context<Self>) {
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(update::download(&latest));
        });
        self.task = Some(cx.spawn(async move |this, cx| {
            let Ok(downloaded) = rx.recv().await else {
                return;
            };
            let setup = match downloaded {
                Ok(path) => path,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| this.fail(error, cx));
                    return;
                }
            };
            // install 只负责把安装程序挂到本进程退出之后，返回很快
            let spawned = update::install(&setup);
            let ok = this
                .update(cx, |this, cx| match spawned {
                    Ok(()) => {
                        this.phase = Phase::Restarting;
                        this.status = if this.lang == Lang::Zh {
                            format!("正在安装 OneCRT v{}，装完会自动重启。", this.version).into()
                        } else {
                            format!(
                                "Installing OneCRT v{}, it will restart automatically…",
                                this.version
                            )
                            .into()
                        };
                        cx.notify();
                        true
                    }
                    Err(error) => {
                        this.fail(error, cx);
                        false
                    }
                })
                .unwrap_or(false);
            if ok {
                // 让状态文字画出来再退出，安装程序在本进程消失后才动手
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(900))
                    .await;
                cx.update(|cx| cx.quit());
            }
        }));
    }

    fn fail(&mut self, error: String, cx: &mut Context<Self>) {
        self.phase = Phase::Failed;
        self.status = if self.lang == Lang::Zh {
            format!("自动更新失败：{error}").into()
        } else {
            format!("Automatic update failed: {error}").into()
        };
        cx.notify();
    }
}

impl Render for UpdateWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = self.lang;
        let url = self.url.clone();
        let title: SharedString = if lang == Lang::Zh {
            format!("OneCRT v{} 已发布", self.version).into()
        } else {
            format!("OneCRT v{} is out", self.version).into()
        };
        let mut footer = dialog_window::buttons();
        if !matches!(self.phase, Phase::Working | Phase::Restarting) {
            footer = footer.child(
                Button::new("update-open")
                    .primary()
                    .label(tr(lang, "打开下载页", "Open download page"))
                    .action_height()
                    .on_click(move |_, _, cx| cx.open_url(&url)),
            );
        }
        footer = footer.child(
            Button::new("update-close")
                .label(tr(lang, "关闭", "Close"))
                .action_height()
                .on_click(|_, window, _| window.remove_window()),
        );
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
                        .child(title),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(cx.theme().muted_foreground)
                        .child(self.status.clone()),
                ),
            footer,
        )
    }
}
