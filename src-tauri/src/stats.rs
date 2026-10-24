//! 性能面板的后端：一条独立的 SSH 连接，在 exec 通道里跑一个 /proc 轮询脚本，按行回传原始采样。
//!
//! 不复用终端那条连接（理由同 files.rs：russh 的 Handle 被 split 后消费掉了），
//! 这样终端标签关掉之后面板还能继续看数，也不往终端热路径上加东西。
//!
//! 数值一律原样回传，差值（CPU%、网速）由前端算：换采样口径不用重连、也不用改这里。

use crate::terminal;
use anyhow::Context;
use russh::client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::sync::{oneshot, Mutex};

const BUF: usize = 16384;

/// 一次采样 = 一行文本，块之间以 `~` 分隔（tr 把换行换成 `~`）。
/// 只用 POSIX 工具，不依赖 awk/perl/python；外层 `sh -c` 让登录 shell 是 csh 类的目标机也能跑循环。
fn script(interval: u32) -> String {
    let body = concat!(
        "( head -n1 /proc/uptime; head -n1 /proc/stat; head -n 16 /proc/meminfo;",
        " cat /proc/loadavg; sed 1,2d /proc/net/dev; df -kP / ) 2>/dev/null",
        " | tr '\\n' '~'; echo; "
    );
    format!("sh -c \"while :; do {body} sleep {interval}; done\"")
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartArgs {
    pub id: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub key_path: String,
    #[serde(default)]
    pub methods: Vec<String>,
    /// 采样间隔秒数，直接写进脚本的 sleep
    pub interval: u32,
}

#[derive(Serialize, Clone)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum OutEvent {
    /// 连接、认证与 exec 已就绪
    Ready,
    /// 一行完整采样（不含换行）
    Line { data: String },
    Closed { reason: String },
}

type Sink = tauri::ipc::Channel<OutEvent>;

fn emit(sink: &Sink, event: OutEvent) {
    let _ = sink.send(event);
}

/// 采样间隔的可跑区间：下限防手滑把目标机打满，上限让面板至少还有动静
fn clamp_interval(sec: u32) -> u32 {
    sec.clamp(1, 60)
}

async fn monitor_logic(args: StartArgs, sink: Sink, ready: oneshot::Sender<Result<(), String>>) {
    let interval = clamp_interval(args.interval);
    let mut ready = Some(ready);
    let result: anyhow::Result<()> = async {
        let key = terminal::load_key(&args.key_path);
        let addr = format!("{}:{}", args.host, args.port);
        let mut handle = client::connect(
            Arc::new(client::Config::default()),
            addr.as_str(),
            terminal::ShellHandler,
        )
        .await
        .with_context(|| format!("无法连接到 {addr}"))?;
        terminal::authenticate(&mut handle, &args.username, &args.password, key, &args.methods).await?;

        let channel = handle.channel_open_session().await?;
        let (mut r, w) = channel.split();
        w.exec(true, script(interval)).await.context("远端执行采样脚本失败")?;

        let _ = ready.take().map(|tx| tx.send(Ok(())));
        emit(&sink, OutEvent::Ready);

        let mut reader = r.make_reader();
        let mut buf = vec![0u8; BUF];
        // exec 通道不保证按行边界投递，残行留到这里等下一块
        let mut pending = String::new();
        loop {
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    pending.push_str(&String::from_utf8_lossy(&buf[..n]));
                    while let Some(i) = pending.find('\n') {
                        let line = pending[..i].trim_end_matches('\r').to_string();
                        pending.drain(..=i);
                        if !line.is_empty() {
                            emit(&sink, OutEvent::Line { data: line });
                        }
                    }
                }
            }
        }
        // handle 得活到读循环结束，否则连接跟着任务一起没
        drop(w);
        drop(handle);
        Ok(())
    }
    .await;

    if let Some(t) = ready.take() {
        // 还没就绪就失败：原因只经 start() 的 Err 回传一次，不再重复发事件
        let _ = t.send(Err(terminal::format_err(result.unwrap_err())));
        return;
    }
    match result {
        Ok(()) => emit(&sink, OutEvent::Closed { reason: "采样已结束".to_string() }),
        Err(e) => emit(&sink, OutEvent::Closed { reason: terminal::format_err(e) }),
    }
    // 任务已经跑完，表里留下的这条不用管：同一个 id 再 start 会把它换掉
}

pub struct StatsManager {
    monitors: HashMap<String, tokio::task::JoinHandle<()>>,
}

pub type Shared = Arc<Mutex<StatsManager>>;

pub fn new_manager() -> Shared {
    Arc::new(Mutex::new(StatsManager {
        monitors: HashMap::new(),
    }))
}

pub async fn start(mgr: &Shared, args: StartArgs, sink: Sink) -> Result<(), String> {
    let id = args.id.clone();
    let (ready_tx, ready_rx) = oneshot::channel::<Result<(), String>>();

    {
        let mut g = mgr.lock().await;
        if let Some(old) = g.monitors.remove(&id) {
            old.abort();
        }
        let join = tokio::spawn(monitor_logic(args, sink, ready_tx));
        g.monitors.insert(id.clone(), join);
    }

    match ready_rx.await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => {
            stop(mgr, &id).await;
            Err(e)
        }
        Err(_) => {
            stop(mgr, &id).await;
            Err("采样启动失败".to_string())
        }
    }
}

pub async fn stop(mgr: &Shared, id: &str) {
    if let Some(h) = mgr.lock().await.monitors.remove(id) {
        h.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::script;

    #[test]
    fn sample_line_is_one_physical_line() {
        let s = script(2);
        assert!(!s.contains('\n'), "脚本必须单行，多行会被 sshd 的 shell 引号搞坏");
        assert!(s.contains("sleep 2; done"));
        assert!(s.contains("tr '\\n' '~'"));
    }
}
