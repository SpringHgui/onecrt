//! 右侧性能面板：CPU / 内存 / 网络 / 磁盘四条曲线，2 秒一采。
//!
//! 采样走 Core 里独立的 SSH 连接（按标签页 id 复用），差值在本地算，
//! 换间隔只需重启采样，不动终端那条连接。

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_channel::{unbounded, Sender};
use gpui_kit as gpui;

use alacritty_terminal::vte::ansi::Rgb as AnsiRgb;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    canvas, div, px, Context, EventEmitter, Hsla, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Window,
};
use onecrt_core::stats::{self, OutEvent, StartArgs};
use onecrt_core::Backend;

use crate::i18n::{tr, Lang};
use crate::term_view::ConnectSpec;
use crate::theme::Theme;

/// 曲线保留的采样数：2 秒间隔下约 3 分钟
const HISTORY: usize = 90;
const INTERVALS: [u32; 4] = [1, 2, 5, 10];
/// exec 通道连上却一行不回来说明脚本没跑成（非 Linux、无 /proc 或登录 shell 不配合）
const FIRST_LINE_SECS: u64 = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PerfPanelEvent {
    Close,
    Interval(u32),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Connecting,
    Live,
    Failed,
}

/// 一条采样里的绝对计数，速率与百分比要等上一条才算得出
#[derive(Clone, Default)]
struct Raw {
    cpu: Option<Vec<f64>>,
    mem_total: f64,
    mem_avail: Option<f64>,
    mem_free: Option<f64>,
    load: Option<Vec<f64>>,
    rx: f64,
    tx: f64,
    disk_used: f64,
    disk_total: f64,
    uptime: f64,
}

#[derive(Clone)]
struct Sample {
    cpu: Option<f32>,
    mem_pct: Option<f32>,
    mem_used: f64,
    mem_total: f64,
    rx: Option<f64>,
    tx: Option<f64>,
    disk_pct: Option<f32>,
    disk_used: f64,
    disk_total: f64,
    load: Option<Vec<f64>>,
    uptime: f64,
}

enum Msg {
    Ready,
    Line(String),
    Closed(String),
    StartFailed(String),
    NoData(u64),
}

pub struct PerfPanel {
    backend: Arc<Backend>,
    theme: Theme,
    lang: Lang,
    tx: Sender<Msg>,
    _pump: gpui::Task<()>,
    session: Option<(String, ConnectSpec)>,
    phase: Phase,
    error: Option<String>,
    samples: Vec<Sample>,
    prev: Option<Raw>,
    prev_at: f64,
    interval: u32,
    gen: u64,
    got_line: bool,
    _timer: Option<gpui::Task<()>>,
}

impl EventEmitter<PerfPanelEvent> for PerfPanel {}

impl PerfPanel {
    pub fn new(
        backend: Arc<Backend>,
        theme: Theme,
        lang: Lang,
        interval: u32,
        cx: &mut Context<Self>,
    ) -> Self {
        let (tx, rx) = unbounded::<Msg>();
        let pump = cx.spawn(async move |this, cx| {
            while let Ok(msg) = rx.recv().await {
                if this.update(cx, |panel, cx| panel.apply(msg, cx)).is_err() {
                    break;
                }
            }
        });
        Self {
            backend,
            theme,
            lang,
            tx,
            _pump: pump,
            session: None,
            phase: Phase::Idle,
            error: None,
            samples: Vec::new(),
            prev: None,
            prev_at: 0.,
            interval,
            gen: 0,
            got_line: false,
            _timer: None,
        }
    }

    pub fn set_theme(&mut self, theme: Theme, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    pub fn set_lang(&mut self, lang: Lang, cx: &mut Context<Self>) {
        self.lang = lang;
        cx.notify();
    }

    /// 面板跟着活动标签页走；采样 id 就是标签页 id，同一个 id 不重连。
    pub fn set_session(&mut self, session: Option<(String, ConnectSpec)>, cx: &mut Context<Self>) {
        let new_id = session.as_ref().map(|(id, _)| id.clone());
        let old_id = self.session.as_ref().map(|(id, _)| id.clone());
        if new_id == old_id {
            return;
        }
        self.stop(old_id);
        self.session = session;
        self.reset();
        if self.session.is_some() {
            self.start(cx);
        } else {
            cx.notify();
        }
    }

    /// 关闭面板时断开采样连接。
    pub fn shutdown(&mut self, cx: &mut Context<Self>) {
        let old = self.session.take().map(|(id, _)| id);
        self.stop(old);
        self.reset();
        cx.notify();
    }

    fn reset(&mut self) {
        self.phase = Phase::Idle;
        self.error = None;
        self.samples.clear();
        self.prev = None;
        self.prev_at = 0.;
        self.got_line = false;
    }

    fn stop(&mut self, id: Option<String>) {
        let Some(id) = id else {
            return;
        };
        let mgr = self.backend.stats.clone();
        self.backend.spawn(async move {
            stats::stop(&mgr, &id).await;
        });
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let Some((id, spec)) = self.session.clone() else {
            return;
        };
        self.gen += 1;
        let gen = self.gen;
        self.phase = Phase::Connecting;
        self.error = None;
        self.samples.clear();
        self.prev = None;
        self.prev_at = 0.;
        self.got_line = false;

        let args = StartArgs {
            id,
            host: spec.host,
            port: spec.port,
            username: spec.username,
            password: spec.password,
            key_path: spec.key_path,
            methods: spec.methods,
            interval: self.interval,
        };
        let mgr = self.backend.stats.clone();
        let tx = self.tx.clone();
        self.backend.spawn(async move {
            let sink_tx = tx.clone();
            let sink: stats::Sink = Arc::new(move |event| {
                let msg = match event {
                    OutEvent::Ready => Msg::Ready,
                    OutEvent::Line { data } => Msg::Line(data),
                    OutEvent::Closed { reason } => Msg::Closed(reason),
                };
                let _ = sink_tx.try_send(msg);
            });
            if let Err(error) = stats::start(&mgr, args, sink).await {
                let _ = tx.try_send(Msg::StartFailed(error));
            }
        });

        let tx = self.tx.clone();
        self._timer = Some(cx.spawn(async move |_this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(FIRST_LINE_SECS))
                .await;
            let _ = tx.try_send(Msg::NoData(gen));
        }));
        cx.notify();
    }

    fn restart(&mut self, cx: &mut Context<Self>) {
        if self.session.is_none() {
            cx.notify();
            return;
        }
        self.start(cx);
    }

    fn pick_interval(&mut self, sec: u32, cx: &mut Context<Self>) {
        if sec == self.interval {
            return;
        }
        self.interval = sec;
        cx.emit(PerfPanelEvent::Interval(sec));
        self.restart(cx);
    }

    fn apply(&mut self, msg: Msg, cx: &mut Context<Self>) {
        match msg {
            Msg::Ready => {
                if self.phase == Phase::Connecting {
                    self.phase = Phase::Live;
                }
            }
            Msg::Line(data) => {
                let now = now_secs();
                let raw = parse_line(&data);
                let dt = if self.prev.is_some() {
                    now - self.prev_at
                } else {
                    0.
                };
                let sample = make_sample(self.prev.as_ref(), &raw, dt);
                self.prev = Some(raw);
                self.prev_at = now;
                self.got_line = true;
                self.phase = Phase::Live;
                self.error = None;
                self.samples.push(sample);
                if self.samples.len() > HISTORY {
                    self.samples.remove(0);
                }
            }
            Msg::Closed(reason) => {
                self.phase = Phase::Failed;
                self.error = Some(reason);
            }
            Msg::StartFailed(error) => {
                self.phase = Phase::Failed;
                self.error = Some(error);
            }
            Msg::NoData(gen) => {
                if gen == self.gen && !self.got_line {
                    self.phase = Phase::Failed;
                    self.error = Some(
                        tr(
                            self.lang,
                            "目标机没有回传采样：需要 Linux 的 /proc 与 sh",
                            "No samples from this host: needs Linux /proc and sh",
                        )
                        .to_string(),
                    );
                }
            }
        }
        cx.notify();
    }

    fn column(&self, pick: impl Fn(&Sample) -> Option<f32>) -> Vec<Option<f32>> {
        self.samples.iter().map(pick).collect()
    }

    fn rate_column(&self, pick: impl Fn(&Sample) -> Option<f64>) -> Vec<Option<f32>> {
        self.samples
            .iter()
            .map(|sample| pick(sample).map(|value| value as f32))
            .collect()
    }
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.)
}

fn nums(text: &str) -> Vec<f64> {
    text.split_whitespace()
        .filter_map(|part| part.parse::<f64>().ok())
        .collect()
}

/// 后端把 /proc 的原始片段用 `~` 拼成一行回传，这里按每段的形状归类。
fn parse_line(line: &str) -> Raw {
    let mut raw = Raw::default();
    let mut ifaces: Vec<(String, f64, f64)> = Vec::new();
    for seg in line.split('~') {
        let seg = seg.trim();
        if seg.is_empty() {
            continue;
        }
        if let Some(rest) = seg.strip_prefix("cpu") {
            raw.cpu = Some(nums(rest));
            continue;
        }
        if seg.starts_with("MemTotal:")
            || seg.starts_with("MemFree:")
            || seg.starts_with("MemAvailable:")
        {
            if let Some(value) = nums(seg).first().copied() {
                if seg.starts_with("MemTotal:") {
                    raw.mem_total = value;
                } else if seg.starts_with("MemFree:") {
                    raw.mem_free = Some(value);
                } else {
                    raw.mem_avail = Some(value);
                }
            }
            continue;
        }
        let tokens: Vec<&str> = seg.split_whitespace().collect();
        if tokens.len() >= 5 && tokens[3].contains('/') {
            let first: Option<Vec<f64>> =
                tokens[..3].iter().map(|t| t.parse::<f64>().ok()).collect();
            if let Some(load) = first {
                raw.load = Some(load);
                continue;
            }
        }
        if tokens.len() >= 6 && tokens[4].ends_with('%') {
            if let (Ok(total), Ok(used)) = (tokens[1].parse::<f64>(), tokens[2].parse::<f64>()) {
                raw.disk_total = total;
                raw.disk_used = used;
                continue;
            }
        }
        if let Some((name, rest)) = seg.rsplit_once(':') {
            let body = rest.trim_end();
            let numeric = !body.is_empty() && body.chars().all(|c| c.is_ascii_digit() || c == ' ');
            if !name.trim().is_empty() && numeric {
                let fields = nums(body);
                if fields.len() >= 9 {
                    ifaces.push((name.trim().to_string(), fields[0], fields[8]));
                    continue;
                }
            }
        }
        if tokens.len() == 2 {
            if let (Ok(uptime), Ok(_)) = (tokens[0].parse::<f64>(), tokens[1].parse::<f64>()) {
                raw.uptime = uptime;
            }
        }
    }
    // 只算物理口：k8s/容器节点上 cali*、veth*、tunl0、docker0 会把同一份流量重复计一遍
    let physical: Vec<&(String, f64, f64)> = ifaces
        .iter()
        .filter(|(name, _, _)| {
            ["eth", "en", "wl", "bond", "ib"]
                .iter()
                .any(|prefix| name.starts_with(prefix))
        })
        .collect();
    let picked = if physical.is_empty() {
        ifaces.iter().collect::<Vec<_>>()
    } else {
        physical
    };
    for (_, rx, tx) in picked {
        raw.rx += rx;
        raw.tx += tx;
    }
    raw
}

fn clamp_pct(value: f64) -> Option<f32> {
    value.is_finite().then(|| value.clamp(0., 100.) as f32)
}

fn cpu_pct(a: &[f64], b: &[f64]) -> Option<f32> {
    let d_total: f64 = b.iter().sum::<f64>() - a.iter().sum::<f64>();
    if d_total <= 0. {
        return None;
    }
    let idle = |x: &[f64]| x.get(3).copied().unwrap_or(0.) + x.get(4).copied().unwrap_or(0.);
    clamp_pct((1. - (idle(b) - idle(a)) / d_total) * 100.)
}

/// 计数器被重置（重启或回绕）时差值为负，这格按无数据画
fn rate(a: f64, b: f64, dt: f64) -> Option<f64> {
    let delta = b - a;
    (delta >= 0. && dt > 0.).then(|| delta / dt)
}

fn mem_pct(raw: &Raw) -> Option<f32> {
    let avail = raw.mem_avail.or(raw.mem_free)?;
    if raw.mem_total <= 0. {
        return None;
    }
    clamp_pct((raw.mem_total - avail) / raw.mem_total * 100.)
}

fn make_sample(prev: Option<&Raw>, raw: &Raw, dt: f64) -> Sample {
    Sample {
        cpu: match prev {
            Some(prev) if dt > 0. => match (&prev.cpu, &raw.cpu) {
                (Some(a), Some(b)) => cpu_pct(a, b),
                _ => None,
            },
            _ => None,
        },
        mem_pct: mem_pct(raw),
        mem_used: raw.mem_total - raw.mem_avail.or(raw.mem_free).unwrap_or(0.),
        mem_total: raw.mem_total,
        rx: prev.and_then(|prev| rate(prev.rx, raw.rx, dt)),
        tx: prev.and_then(|prev| rate(prev.tx, raw.tx, dt)),
        disk_pct: (raw.disk_total > 0.)
            .then(|| clamp_pct(raw.disk_used / raw.disk_total * 100.))
            .flatten(),
        disk_used: raw.disk_used,
        disk_total: raw.disk_total,
        load: raw.load.clone(),
        uptime: raw.uptime,
    }
}

/// 轴的上界取不小于峰值的 1/2/5×10ⁿ，刻度才不会每秒跳一个数
fn nice_max(peak: f64) -> f32 {
    if peak <= 0. {
        return 1.;
    }
    let power = 10f64.powf(peak.log10().floor());
    for factor in [1., 2., 5., 10.] {
        if factor * power >= peak {
            return (factor * power) as f32;
        }
    }
    (10. * power) as f32
}

fn fmt_size(n: f64) -> String {
    if n < 1024. {
        return format!("{n:.0} B");
    }
    let units = ["KB", "MB", "GB", "TB"];
    let mut value = n / 1024.;
    let mut index = 0;
    while value >= 1024. && index < units.len() - 1 {
        value /= 1024.;
        index += 1;
    }
    if value >= 100. {
        format!("{value:.0} {}", units[index])
    } else {
        format!("{value:.1} {}", units[index])
    }
}

fn fmt_rate(bytes: Option<f64>) -> String {
    match bytes {
        Some(bytes) if bytes >= 1. => format!("{}/s", fmt_size(bytes.round())),
        _ => "—".to_string(),
    }
}

fn fmt_uptime(sec: f64) -> String {
    let total = sec.max(0.) as u64;
    let days = total / 86400;
    let hours = (total % 86400) / 3600;
    let minutes = (total % 3600) / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

fn ansi_color(rgb: AnsiRgb) -> Hsla {
    let value = ((rgb.r as u32) << 16) | ((rgb.g as u32) << 8) | rgb.b as u32;
    gpui::Rgba::from(gpui::rgb(value)).into()
}

/// 迷你折线：viewBox 横向拉满，线条宽度固定 1.3px；多条曲线共用一条 y 轴
fn spark(series: Vec<(Vec<Option<f32>>, Hsla, bool)>, max: f32) -> impl IntoElement {
    canvas(
        move |_bounds, _, _| series,
        move |bounds, series, window, _| {
            let count = series
                .iter()
                .map(|(values, _, _)| values.len())
                .max()
                .unwrap_or(0);
            if count < 2 {
                return;
            }
            let pad = 3.;
            let width = bounds.size.width;
            let height = bounds.size.height;
            let x = |index: usize| bounds.origin.x + width * (index as f32 / (count - 1) as f32);
            let y = |value: f32| {
                bounds.origin.y + height
                    - px(pad)
                    - (height - px(pad * 2.)) * (value.clamp(0., max) / max)
            };
            let bottom = bounds.origin.y + height;

            for (values, color, fill) in series {
                if fill {
                    let mut points = vec![gpui::point(x(0), bottom)];
                    for (index, value) in values.iter().enumerate() {
                        if let Some(value) = value {
                            points.push(gpui::point(x(index), y(*value)));
                        }
                    }
                    points.push(gpui::point(x(count - 1), bottom));
                    let mut builder = gpui::PathBuilder::fill();
                    builder.add_polygon(&points, true);
                    if let Ok(path) = builder.build() {
                        window.paint_path(path, color.opacity(0.13));
                    }
                }

                let mut builder = gpui::PathBuilder::stroke(px(1.3));
                let mut started = false;
                let mut drew = false;
                for (index, value) in values.iter().enumerate() {
                    let Some(value) = value else {
                        started = false;
                        continue;
                    };
                    let point = gpui::point(x(index), y(*value));
                    if started {
                        builder.line_to(point);
                        drew = true;
                    } else {
                        builder.move_to(point);
                        started = true;
                    }
                }
                if drew {
                    if let Ok(path) = builder.build() {
                        window.paint_path(path, color);
                    }
                }
            }
        },
    )
    .h(px(40.))
    .w_full()
}

fn chip(
    id: &'static str,
    label: SharedString,
    active: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let t = theme.clone();
    let hover = t.bg_hover;
    div()
        .id(id)
        .h(px(20.))
        .px(px(6.))
        .flex()
        .items_center()
        .rounded_md()
        .text_size(px(10.))
        .cursor_pointer()
        .when(active, |s| s.bg(t.bg_selected).text_color(t.text))
        .when(!active, |s| s.text_color(t.text_3))
        .hover(move |s| s.bg(hover))
        .child(label)
}

impl Render for PerfPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme.clone();
        let lang = self.lang;
        let host: SharedString = self
            .session
            .as_ref()
            .map(|(_, spec)| format!("{}@{}", spec.username, spec.host).into())
            .unwrap_or_else(|| "—".into());

        let mut header = div()
            .h(px(30.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(6.))
            .border_b_1()
            .border_color(t.line)
            .child(div().text_size(px(12.)).text_color(t.text).child(tr(
                lang,
                "性能",
                "Performance",
            )))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .text_size(px(10.))
                    .text_color(t.text_3)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(host),
            );
        for sec in INTERVALS {
            let label: SharedString = if lang == Lang::Zh {
                format!("{sec}秒").into()
            } else {
                format!("{sec}s").into()
            };
            let id = match sec {
                1 => "perf-int-1",
                2 => "perf-int-2",
                5 => "perf-int-5",
                _ => "perf-int-10",
            };
            header = header.child(
                chip(id, label, self.interval == sec, &t)
                    .on_click(cx.listener(move |this, _, _, cx| this.pick_interval(sec, cx))),
            );
        }
        header = header.child(
            chip("perf-close".into(), "×".into(), false, &t)
                .on_click(cx.listener(|_, _, _, cx| cx.emit(PerfPanelEvent::Close))),
        );

        let last = self.samples.last().cloned();
        let mut body = div()
            .id("perf-body")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(10.))
            .px(px(8.))
            .py(px(8.))
            .overflow_y_scroll();

        match self.phase {
            Phase::Idle => {
                body = body.child(hint(
                    tr(
                        lang,
                        "没有活动标签页：先连上一台服务器",
                        "No active tab: connect to a server first",
                    ),
                    t.text_3,
                ));
            }
            Phase::Connecting => {
                body = body.child(hint(tr(lang, "连接中…", "Connecting…"), t.text_3));
            }
            Phase::Failed => {
                let reason = self
                    .error
                    .clone()
                    .unwrap_or_else(|| tr(lang, "采样失败", "Sampling failed").to_string());
                body = body.child(hint(reason.into(), t.danger));
            }
            Phase::Live => {
                if last.is_none() {
                    body = body.child(hint(
                        tr(
                            lang,
                            "采集中，等第二个采样点",
                            "Sampling, waiting for the second point",
                        ),
                        t.text_3,
                    ));
                }
            }
        }

        if let Some(last) = last.clone() {
            let cpu_value = last
                .cpu
                .map(|v| format!("{v:.1}%"))
                .unwrap_or_else(|| "—".into());
            let cpu_sub = last
                .load
                .as_ref()
                .map(|load| {
                    let parts: Vec<String> = load.iter().map(|v| format!("{v:.2}")).collect();
                    format!("{} {}", tr(lang, "负载", "Load"), parts.join(" "))
                })
                .unwrap_or_else(|| "—".into());
            body = body.child(metric(
                tr(lang, "CPU", "CPU"),
                cpu_value,
                cpu_sub,
                spark(vec![(self.column(|s| s.cpu), t.accent, true)], 100.),
                &t,
            ));

            let mem_value = last
                .mem_pct
                .map(|v| format!("{v:.0}%"))
                .unwrap_or_else(|| "—".into());
            let mem_sub = if last.mem_total > 0. {
                format!(
                    "{} · {} {}",
                    fmt_size(last.mem_total * 1024.),
                    tr(lang, "已用", "used"),
                    fmt_size(last.mem_used * 1024.)
                )
            } else {
                "—".into()
            };
            body = body.child(metric(
                tr(lang, "内存", "Memory"),
                mem_value,
                mem_sub,
                spark(
                    vec![(self.column(|s| s.mem_pct), ansi_color(t.ansi[3]), true)],
                    100.,
                ),
                &t,
            ));

            let peak = nice_max(
                self.samples
                    .iter()
                    .flat_map(|s| [s.rx.unwrap_or(0.), s.tx.unwrap_or(0.)])
                    .fold(1., f64::max),
            );
            let net_sub = format!(
                "{} · {} {}",
                tr(lang, "下行 / 上行", "rx / tx"),
                tr(lang, "峰值", "peak"),
                fmt_rate(Some(peak as f64))
            );
            body = body.child(metric(
                tr(lang, "网络", "Network"),
                format!("{} / {}", fmt_rate(last.rx), fmt_rate(last.tx)).into(),
                net_sub,
                spark(
                    vec![
                        (self.rate_column(|s| s.rx), t.accent, false),
                        (self.rate_column(|s| s.tx), t.text_2, false),
                    ],
                    peak,
                ),
                &t,
            ));

            let disk_value = last
                .disk_pct
                .map(|v| format!("{v:.0}%"))
                .unwrap_or_else(|| "—".into());
            let disk_sub = if last.disk_total > 0. {
                format!(
                    "{} {}",
                    tr(lang, "可用", "free"),
                    fmt_size((last.disk_total - last.disk_used) * 1024.)
                )
            } else {
                "—".into()
            };
            body = body.child(metric(
                tr(lang, "磁盘 /", "Disk /"),
                disk_value,
                disk_sub,
                spark(vec![(self.column(|s| s.disk_pct), t.text_2, true)], 100.),
                &t,
            ));
        }

        let mut footer = div()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(px(8.))
            .py(px(5.))
            .border_t_1()
            .border_color(t.line)
            .text_size(px(10.))
            .text_color(t.text_3)
            .child(
                div()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(tr(
                        lang,
                        "数据取自目标机的 /proc 与 df",
                        "Sampled from the host's /proc and df",
                    )),
            );
        if let Some(last) = last {
            footer = footer.child(
                div()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(SharedString::from(format!(
                        "{} {} · {} {}",
                        tr(lang, "运行", "up"),
                        fmt_uptime(last.uptime),
                        self.samples.len(),
                        tr(lang, "个采样", "samples")
                    ))),
            );
        }

        div()
            .w_full()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(t.bg)
            .child(header)
            .child(body)
            .child(footer)
    }
}

fn hint(text: SharedString, color: Hsla) -> gpui::Div {
    div().text_size(px(11.)).text_color(color).child(text)
}

fn metric(
    name: SharedString,
    value: String,
    sub: String,
    chart: impl IntoElement,
    theme: &Theme,
) -> gpui::Div {
    let t = theme.clone();
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(6.))
                .child(div().text_size(px(11.)).text_color(t.text_2).child(name))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(t.text)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(value),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(px(10.))
                        .text_color(t.text_3)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(sub),
                ),
        )
        .child(chart)
}
