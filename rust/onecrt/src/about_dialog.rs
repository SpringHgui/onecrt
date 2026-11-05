//! 关于对话框：版本、运行时与数据目录，不联网。

use gpui_kit as gpui;

use gpui::{
    div, px, AnyWindowHandle, App, AppContext, Context, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, WeakEntity, Window,
};
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    notification::Notification,
    progress::Progress,
    ActiveTheme, Icon, Sizable, WindowExt,
};

use crate::dialog_window;
use crate::dialog_window::ActionButton;
use crate::i18n::{tr, Lang};
use crate::update;

const GITHUB: &str = "https://github.com/SpringHgui/onecrt";
const GITEE: &str = "https://gitee.com/SpringHgui/onecrt";

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
        let repo_link = move |id: &'static str, url: &'static str, logo: &'static [u8]| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(6.))
                .text_color(muted)
                .cursor_pointer()
                .hover(move |s| s.text_color(fg))
                .child(Icon::default().data(logo).size(px(12.)).text_color(muted))
                .child(url)
                .on_click(move |_, _, cx| cx.open_url(url))
        };
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
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .text_size(px(11.))
                        .text_color(muted)
                        .child({
                            let dir = SharedString::from(data_dir.clone());
                            div()
                                .id("about-data-dir")
                                .cursor_pointer()
                                .hover(move |s| s.text_color(fg))
                                .on_click(move |_, _, cx| cx.open_url(&dir))
                                .child(SharedString::from(format!(
                                    "{} {data_dir}",
                                    tr(lang, "数据目录", "Data directory")
                                )))
                        })
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(3.))
                                .child(repo_link(
                                    "about-github",
                                    GITHUB,
                                    include_bytes!("../assets/github.svg"),
                                ))
                                .child(repo_link(
                                    "about-gitee",
                                    GITEE,
                                    include_bytes!("../assets/gitee.svg"),
                                )),
                        ),
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
    let downloading: SharedString = match lang {
        Lang::Zh => format!("正在下载 OneCRT v{version}…").into(),
        Lang::En => format!("Downloading OneCRT v{version}…").into(),
    };
    let status: SharedString = if auto {
        downloading.clone()
    } else if lang == Lang::Zh {
        format!("OneCRT v{version} 已发布，当前平台请到发布页下载安装。").into()
    } else {
        format!("OneCRT v{version} is out; download it from the release page on this platform.")
            .into()
    };
    // 自动更新时多留出进度条和速度那一行
    dialog_window::open(
        window,
        cx,
        tr(lang, "发现新版本", "Update available"),
        420.,
        if auto { 200. } else { 176. },
        move |_, cx| {
            let entity = cx.new(|_| UpdateWindow {
                lang,
                version: version.clone(),
                downloading,
                status: status.clone(),
                url,
                phase: if auto { Phase::Working } else { Phase::Manual },
                task: None,
                progress: None,
                latest: auto.then(|| latest.clone()),
                cancel: Default::default(),
            });
            if auto {
                entity.update(cx, |this, cx| this.start(cx));
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
    /// 「正在下载 v…」这句要复用，重试时再设回去
    downloading: SharedString,
    status: SharedString,
    url: std::rc::Rc<String>,
    phase: Phase,
    task: Option<gpui::Task<()>>,
    progress: Option<update::DownloadProgress>,
    latest: Option<update::Latest>,
    /// 窗口关掉就置位，下载线程看到后留着半截文件退出，等下次续传
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for UpdateWindow {
    fn drop(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

enum DownloadMsg {
    Progress(update::DownloadProgress),
    Done(Result<std::path::PathBuf, String>),
}

impl UpdateWindow {
    fn start(&mut self, cx: &mut Context<Self>) {
        let Some(latest) = self.latest.clone() else {
            return;
        };
        let (tx, rx) = async_channel::unbounded();
        let cancel = self.cancel.clone();
        std::thread::spawn(move || {
            let result = update::download(&latest, &cancel, |progress| {
                let _ = tx.try_send(DownloadMsg::Progress(progress));
            });
            let _ = tx.try_send(DownloadMsg::Done(result));
        });
        self.task = Some(cx.spawn(async move |this, cx| {
            let setup = loop {
                match rx.recv().await {
                    Ok(DownloadMsg::Progress(progress)) => {
                        let _ = this.update(cx, |this, cx| {
                            this.progress = Some(progress);
                            cx.notify();
                        });
                    }
                    Ok(DownloadMsg::Done(Ok(path))) => break path,
                    Ok(DownloadMsg::Done(Err(error))) => {
                        let _ = this.update(cx, |this, cx| this.fail(error, cx));
                        return;
                    }
                    Err(_) => return,
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

    fn retry(&mut self, cx: &mut Context<Self>) {
        if self.latest.is_none() {
            return;
        }
        self.phase = Phase::Working;
        self.progress = None;
        self.status = self.downloading.clone();
        self.cancel = Default::default();
        self.start(cx);
        cx.notify();
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
        let muted = cx.theme().muted_foreground;
        let accent = cx.theme().primary;
        let title: SharedString = if lang == Lang::Zh {
            format!("OneCRT v{} 已发布", self.version).into()
        } else {
            format!("OneCRT v{} is out", self.version).into()
        };
        let mut footer = dialog_window::buttons();
        if self.phase == Phase::Failed {
            footer = footer.child(
                Button::new("update-retry")
                    .primary()
                    .label(tr(lang, "重试", "Retry"))
                    .action_height()
                    .on_click(cx.listener(|this, _, _, cx| this.retry(cx))),
            );
        }
        if !matches!(self.phase, Phase::Working | Phase::Restarting) {
            let open = Button::new("update-open")
                .label(tr(lang, "打开下载页", "Open download page"))
                .action_height()
                .on_click(move |_, _, cx| cx.open_url(&url));
            // 失败态里「重试」才是主按钮
            footer = footer.child(if self.phase == Phase::Failed {
                open
            } else {
                open.primary()
            });
        }
        footer = footer.child(
            Button::new("update-close")
                .label(tr(lang, "关闭", "Close"))
                .action_height()
                .on_click(|_, window, _| window.remove_window()),
        );

        let mut content = div()
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
                    .text_color(muted)
                    .child(self.status.clone()),
            );
        if self.phase == Phase::Working {
            let (value, stats) = match &self.progress {
                Some(progress) => (
                    progress
                        .total
                        .filter(|total| *total > 0)
                        .map(|total| {
                            (progress.downloaded as f32 / total as f32 * 100.).clamp(0., 100.)
                        })
                        .unwrap_or(0.),
                    Some(download_stats(progress)),
                ),
                None => (0., None),
            };
            content = content
                .child(
                    div().pt(px(4.)).child(
                        Progress::new("update-progress")
                            .loading(self.progress.is_none())
                            .value(value)
                            .color(accent)
                            .accessibility_label(tr(lang, "下载进度", "Download progress")),
                    ),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(muted)
                        .child(match stats {
                            Some(stats) => SharedString::from(stats),
                            None => tr(lang, "正在连接…", "Connecting…"),
                        }),
                );
        }

        dialog_window::page(cx, content, footer)
    }
}

/// 「3.2 / 13.8 MB · 23% · 1.2 MB/s」，总量未知时省掉百分比。
fn download_stats(progress: &update::DownloadProgress) -> String {
    let mut parts = Vec::new();
    match progress.total.filter(|total| *total > 0) {
        Some(total) => {
            parts.push(format!(
                "{} / {}",
                human_size(progress.downloaded as f64),
                human_size(total as f64)
            ));
            parts.push(format!(
                "{:.0}%",
                progress.downloaded as f64 / total as f64 * 100.
            ));
        }
        None => parts.push(human_size(progress.downloaded as f64)),
    }
    if progress.bytes_per_sec > 0. {
        parts.push(format!("{}/s", human_size(progress.bytes_per_sec)));
    }
    parts.join(" · ")
}

fn human_size(bytes: f64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes.max(0.);
    let mut unit = 0;
    while value >= 1024. && unit < UNITS.len() - 1 {
        value /= 1024.;
        unit += 1;
    }
    if unit == 0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
