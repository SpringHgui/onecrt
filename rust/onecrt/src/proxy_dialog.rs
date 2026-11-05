//! 网络代理设置：更新检查与安装包下载可以直连、跟随系统代理，或走自定义代理。

use gpui_kit as gpui;

use gpui::{
    div, px, App, AppContext, Context, Entity, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    ActiveTheme, Icon,
};
use onecrt_core::settings;
use ureq::ProxyProtocol;

use crate::dialog_window::{self, ActionButton};
use crate::i18n::{tr, Lang};
use crate::update;

pub fn open_proxy_dialog(
    window: &mut Window,
    cx: &mut App,
    lang: Lang,
    on_saved: impl Fn(&mut App) + 'static,
) {
    dialog_window::open(
        window,
        cx,
        tr(lang, "网络代理", "Network Proxy"),
        460.,
        332.,
        move |window, cx| {
            let view = cx.new(|cx| ProxyWindow::new(lang, window, cx, Box::new(on_saved)));
            if view.read(cx).mode == Mode::Custom {
                view.read(cx)
                    .address
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
            }
            view
        },
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Direct,
    System,
    Custom,
}

impl Mode {
    fn from_setting(value: &str) -> Self {
        match value {
            settings::PROXY_DIRECT => Self::Direct,
            settings::PROXY_CUSTOM => Self::Custom,
            _ => Self::System,
        }
    }

    fn setting(self) -> &'static str {
        match self {
            Self::Direct => settings::PROXY_DIRECT,
            Self::System => settings::PROXY_SYSTEM,
            Self::Custom => settings::PROXY_CUSTOM,
        }
    }
}

/// 配置里存的是一条完整 URL，打开弹窗时拆回各个输入框。
struct SavedProxy {
    scheme: &'static str,
    host: String,
    port: String,
    user: String,
    password: String,
}

impl Default for SavedProxy {
    fn default() -> Self {
        Self {
            scheme: "http",
            host: String::new(),
            port: "7890".into(),
            user: String::new(),
            password: String::new(),
        }
    }
}

impl SavedProxy {
    fn parse(value: &str) -> Self {
        let value = value.trim();
        if value.is_empty() {
            return Self::default();
        }
        let Ok(proxy) = ureq::Proxy::new(value) else {
            // 认不出来的地址原样留在输入框里，别让用户以为配置丢了
            return Self {
                host: value.to_string(),
                ..Self::default()
            };
        };
        Self {
            scheme: scheme_name(proxy.protocol()),
            host: proxy.host().to_string(),
            port: proxy.port().to_string(),
            user: proxy.username().unwrap_or_default().to_string(),
            password: proxy.password().unwrap_or_default().to_string(),
        }
    }
}

fn scheme_name(protocol: ProxyProtocol) -> &'static str {
    match protocol {
        ProxyProtocol::Http => "http",
        ProxyProtocol::Https => "https",
        ProxyProtocol::Socks4 => "socks4",
        ProxyProtocol::Socks4A => "socks4a",
        ProxyProtocol::Socks5 => "socks5",
        ProxyProtocol::Socks5h => "socks5h",
        _ => "http",
    }
}

#[derive(Debug)]
enum ProxyError {
    Address,
    Port,
    Invalid,
}

struct ProxyWindow {
    lang: Lang,
    mode: Mode,
    /// 用户没在地址里写协议时沿用原来的（默认 http）
    scheme: &'static str,
    address: Entity<InputState>,
    port: Entity<InputState>,
    user: Entity<InputState>,
    password: Entity<InputState>,
    error: Option<SharedString>,
    /// 保存成功后通知主窗口刷新状态栏
    on_saved: Box<dyn Fn(&mut App)>,
    _subs: Vec<Subscription>,
}

impl ProxyWindow {
    fn new(
        lang: Lang,
        window: &mut Window,
        cx: &mut Context<Self>,
        on_saved: Box<dyn Fn(&mut App)>,
    ) -> Self {
        let prefs = settings::load();
        let mode = Mode::from_setting(&prefs.proxy_mode);
        let saved = SavedProxy::parse(&prefs.proxy);
        let address = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("127.0.0.1")
                .default_value(saved.host.clone())
        });
        let port = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("7890")
                .default_value(saved.port.clone())
        });
        let user = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(tr(lang, "可选", "Optional"))
                .default_value(saved.user.clone())
        });
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(tr(lang, "可选", "Optional"))
                .default_value(saved.password.clone())
        });
        password.update(cx, |state, cx| state.set_masked(true, window, cx));

        let mut subs = Vec::with_capacity(4);
        for input in [&address, &port, &user, &password] {
            subs.push(cx.subscribe_in(
                input,
                window,
                |this, _input, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { .. } => this.save(window, cx),
                    InputEvent::Change => {
                        // 改过内容就把上次的报错撤掉，别让旧错误一直挂着
                        if this.error.take().is_some() {
                            cx.notify();
                        }
                    }
                    _ => {}
                },
            ));
        }

        Self {
            lang,
            mode,
            scheme: saved.scheme,
            address,
            port,
            user,
            password,
            error: None,
            on_saved,
            _subs: subs,
        }
    }

    fn set_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        self.error = None;
        if mode == Mode::Custom {
            self.address.read(cx).focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    fn message(&self, error: ProxyError) -> SharedString {
        match error {
            ProxyError::Address => tr(self.lang, "请填写服务器地址", "Server address is required"),
            ProxyError::Port => tr(
                self.lang,
                "端口需在 1–65535 之间",
                "Port must be between 1 and 65535",
            ),
            ProxyError::Invalid => tr(
                self.lang,
                "代理地址无法解析，请检查地址与端口",
                "Invalid proxy address; check the address and port",
            ),
        }
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut prefs = settings::load();
        if self.mode == Mode::Custom {
            let address = self.address.read(cx).value().to_string();
            let port = self.port.read(cx).value().to_string();
            let user = self.user.read(cx).value().to_string();
            let password = self.password.read(cx).value().to_string();
            match compose_proxy(self.scheme, &address, &port, &user, &password) {
                Ok(proxy) => prefs.proxy = proxy,
                Err(error) => {
                    let message = self.message(error);
                    self.error = Some(message);
                    cx.notify();
                    return;
                }
            }
        }
        // 直连/系统也保留原来的地址，切回自定义时不用重填
        prefs.proxy_mode = self.mode.setting().to_string();
        match settings::save(&prefs) {
            Ok(()) => {
                (self.on_saved)(cx);
                window.remove_window();
            }
            Err(error) => {
                self.error = Some(
                    format!(
                        "{}{}",
                        tr(self.lang, "保存失败：", "Failed to save: "),
                        error
                    )
                    .into(),
                );
                cx.notify();
            }
        }
    }
}

impl Render for ProxyWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = self.lang;
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let border = theme.border;
        let background = theme.background;
        let primary = theme.primary;
        let accent = theme.accent;
        let danger = theme.danger;
        let radius = theme.radius;
        let label = |text: SharedString| {
            div()
                .w(px(76.))
                .flex_shrink_0()
                .text_size(px(12.))
                .child(text)
        };

        let modes = [
            (Mode::Direct, IconName::Cable, tr(lang, "直连", "Direct")),
            (Mode::System, IconName::Monitor, tr(lang, "系统", "System")),
            (
                Mode::Custom,
                IconName::ServerCog,
                tr(lang, "自定义代理", "Custom"),
            ),
        ];
        let current = self.mode;
        let cards =
            div()
                .flex()
                .gap(px(8.))
                .children(modes.into_iter().map(|(mode, icon, text)| {
                    let on = mode == current;
                    div()
                        .id(mode.setting())
                        .flex_1()
                        .h(px(58.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(4.))
                        .rounded(radius)
                        .border_1()
                        .border_color(if on { primary } else { border })
                        .bg(if on {
                            primary.opacity(0.12)
                        } else {
                            background
                        })
                        .text_color(if on { primary } else { muted })
                        .cursor_pointer()
                        .hover(move |s| {
                            if on {
                                s
                            } else {
                                s.bg(accent).text_color(primary)
                            }
                        })
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.set_mode(mode, window, cx)),
                        )
                        .child(Icon::new(icon).size(px(18.)))
                        .child(div().text_size(px(11.)).child(text))
                }));

        let body = match self.mode {
            Mode::Custom => div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(label(tr(lang, "服务器地址", "Server address")))
                        .child(div().flex_1().child(Input::new(&self.address)))
                        .child(div().text_size(px(12.)).child(tr(lang, "端口", "Port")))
                        .child(div().w(px(84.)).child(Input::new(&self.port))),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(label(tr(lang, "用户名", "Username")))
                        .child(div().flex_1().child(Input::new(&self.user))),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(label(tr(lang, "密码", "Password")))
                        .child(
                            div()
                                .flex_1()
                                .child(Input::new(&self.password).mask_toggle()),
                        ),
                )
                // 报错和格式提示占同一行：窗口高度是按这几行定的，多插一行会被裁掉
                .child(match self.error.clone() {
                    Some(error) => div().text_size(px(11.)).text_color(danger).child(error),
                    None => div()
                        .text_size(px(11.))
                        .text_color(muted)
                        .child(tr(
                            lang,
                            "支持 HTTP / SOCKS5；地址里可带 socks5:// 前缀。",
                            "HTTP or SOCKS5; prefix the address with socks5:// to switch.",
                        )),
                }),
            Mode::Direct | Mode::System => div()
                .flex_1()
                .flex()
                .items_center()
                .p(px(12.))
                .rounded(radius)
                .border_1()
                .border_color(border)
                .bg(background)
                .text_size(px(11.))
                .text_color(muted)
                .child(if self.mode == Mode::Direct {
                    tr(
                        lang,
                        "不使用代理，直接连接更新服务器。",
                        "Connect to the update server directly, without a proxy.",
                    )
                } else {
                    tr(
                        lang,
                        "跟随系统代理设置（环境变量；Windows 上还有 Internet 选项）。",
                        "Follow the system proxy settings (environment variables, plus Internet Options on Windows).",
                    )
                }),
        };

        dialog_window::page(
            cx,
            div()
                .size_full()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(div().text_size(px(11.)).text_color(muted).child(tr(
                    lang,
                    "更新检查与安装包下载使用的网络。",
                    "Network used for update checks and installer downloads.",
                )))
                .child(cards)
                .child(body),
            dialog_window::buttons()
                .child(
                    Button::new("proxy-close")
                        .label(tr(lang, "关闭", "Close"))
                        .action_height()
                        .on_click(|_, window, _| window.remove_window()),
                )
                .child(
                    Button::new("proxy-save")
                        .primary()
                        .label(tr(lang, "保存", "Save"))
                        .action_height()
                        .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                ),
        )
    }
}

/// 把几个输入框拼回 ureq 认的代理 URL。
///
/// 地址栏允许直接写 `socks5://host` 或带端口的完整 URL；端口框只在地址没写端口时生效。
fn compose_proxy(
    scheme: &'static str,
    address: &str,
    port: &str,
    user: &str,
    password: &str,
) -> Result<String, ProxyError> {
    let raw = address.trim();
    if raw.is_empty() {
        return Err(ProxyError::Address);
    }
    let (scheme, rest) = match raw.split_once("://") {
        Some((scheme, rest)) => {
            let scheme = scheme.trim().to_ascii_lowercase();
            if !matches!(
                scheme.as_str(),
                "http" | "https" | "socks4" | "socks4a" | "socks5" | "socks5h"
            ) {
                return Err(ProxyError::Address);
            }
            (scheme, rest.trim())
        }
        None => (scheme.to_string(), raw),
    };
    let rest = rest.trim_end_matches('/');
    let (host, inline_port) = split_host_port(rest);
    if host.is_empty() {
        return Err(ProxyError::Address);
    }
    let port = match inline_port {
        Some(port) => port,
        None => port.trim(),
    };
    let port: u16 = port.parse().map_err(|_| ProxyError::Port)?;
    if port == 0 {
        return Err(ProxyError::Port);
    }

    let mut url = format!("{scheme}://");
    let user = user.trim();
    if !user.is_empty() {
        url.push_str(&encode_userinfo(user));
        if !password.is_empty() {
            url.push(':');
            url.push_str(&encode_userinfo(password));
        }
        url.push('@');
    }
    url.push_str(host);
    url.push(':');
    url.push_str(&port.to_string());
    if update::proxy_is_valid(&url) {
        Ok(url)
    } else {
        Err(ProxyError::Invalid)
    }
}

/// 地址里自带的端口优先；`[::1]:7890` 也得能拆。
fn split_host_port(rest: &str) -> (&str, Option<&str>) {
    if let Some(inner) = rest.strip_prefix('[') {
        if let Some(end) = inner.find(']') {
            let host = &rest[..end + 2];
            return match rest[end + 2..].strip_prefix(':') {
                Some(port) => (host, Some(port)),
                None => (host, None),
            };
        }
        return (rest, None);
    }
    match rest.rsplit_once(':') {
        Some((host, port))
            if !host.is_empty()
                && !host.contains(':')
                && !port.is_empty()
                && port.bytes().all(|b| b.is_ascii_digit()) =>
        {
            (host, Some(port))
        }
        _ => (rest, None),
    }
}

/// ureq 会把仓库里的百分号编码解一次；账号密码里的特殊字符得先编进去。
fn encode_userinfo(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{compose_proxy, encode_userinfo, split_host_port, Mode, ProxyError, SavedProxy};
    use onecrt_core::settings;

    #[test]
    fn splits_host_and_optional_port() {
        assert_eq!(split_host_port("127.0.0.1"), ("127.0.0.1", None));
        assert_eq!(
            split_host_port("127.0.0.1:7890"),
            ("127.0.0.1", Some("7890"))
        );
        assert_eq!(split_host_port("[::1]:7890"), ("[::1]", Some("7890")));
        assert_eq!(split_host_port("[::1]"), ("[::1]", None));
    }

    #[test]
    fn composes_plain_and_full_addresses() {
        assert_eq!(
            compose_proxy("http", "127.0.0.1", "7890", "", "").unwrap(),
            "http://127.0.0.1:7890"
        );
        // 地址里写了端口就不看端口框
        assert_eq!(
            compose_proxy("http", "http://127.0.0.1:8080", "7890", "", "").unwrap(),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            compose_proxy("http", "socks5://127.0.0.1", "1080", "", "").unwrap(),
            "socks5://127.0.0.1:1080"
        );
        assert_eq!(
            compose_proxy("http", "[::1]", "7890", "", "").unwrap(),
            "http://[::1]:7890"
        );
    }

    #[test]
    fn rejects_bad_fields() {
        assert!(matches!(
            compose_proxy("http", "", "7890", "", ""),
            Err(ProxyError::Address)
        ));
        assert!(matches!(
            compose_proxy("http", "127.0.0.1", "", "", ""),
            Err(ProxyError::Port)
        ));
        assert!(matches!(
            compose_proxy("http", "127.0.0.1", "70000", "", ""),
            Err(ProxyError::Port)
        ));
        assert!(matches!(
            compose_proxy("http", "ftp://127.0.0.1", "7890", "", ""),
            Err(ProxyError::Address)
        ));
    }

    #[test]
    fn encodes_credentials_and_roundtrips_through_ureq() {
        let url = compose_proxy("http", "127.0.0.1", "7890", "u", "p@ss word").unwrap();
        assert_eq!(url, "http://u:p%40ss%20word@127.0.0.1:7890");
        let proxy = ureq::Proxy::new(&url).unwrap();
        assert_eq!(proxy.username(), Some("u"));
        assert_eq!(proxy.password(), Some("p@ss word"));
        assert_eq!(encode_userinfo("a:b"), "a%3Ab");
    }

    #[test]
    fn saved_url_splits_back_into_fields() {
        let saved = SavedProxy::parse("socks5://u:p%40ss@127.0.0.1:1080");
        assert_eq!(
            (saved.scheme, saved.host.as_str(), saved.port.as_str()),
            ("socks5", "127.0.0.1", "1080")
        );
        assert_eq!(
            (saved.user.as_str(), saved.password.as_str()),
            ("u", "p@ss")
        );
        // 再拼回去应当还是原来那个地址
        assert_eq!(
            compose_proxy(
                saved.scheme,
                &saved.host,
                &saved.port,
                &saved.user,
                &saved.password
            )
            .unwrap(),
            "socks5://u:p%40ss@127.0.0.1:1080"
        );
    }

    #[test]
    fn mode_maps_to_and_from_settings() {
        assert_eq!(Mode::from_setting(settings::PROXY_DIRECT), Mode::Direct);
        assert_eq!(Mode::from_setting(settings::PROXY_CUSTOM), Mode::Custom);
        assert_eq!(Mode::from_setting(settings::PROXY_SYSTEM), Mode::System);
        assert_eq!(Mode::from_setting("bogus"), Mode::System);
        assert_eq!(Mode::Direct.setting(), settings::PROXY_DIRECT);
        assert_eq!(Mode::System.setting(), settings::PROXY_SYSTEM);
        assert_eq!(Mode::Custom.setting(), settings::PROXY_CUSTOM);
    }
}
