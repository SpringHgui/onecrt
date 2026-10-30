use crate::ssh;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::select;
use tokio::sync::{mpsc, oneshot, Mutex};

const BUF_SIZE: usize = 32768;

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OpenArgs {
    pub id: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub key_path: String,
    /// 勾选了的认证方法，按列表顺序尝试；为空回退旧版固定顺序
    #[serde(default)]
    pub methods: Vec<String>,
    pub cols: u16,
    pub rows: u16,
    /// 空闲保活秒数，0 表示关闭；省略时用 KEEPALIVE_SECS
    #[serde(default)]
    pub keepalive_secs: Option<u64>,
}

#[derive(Clone, Copy)]
struct Win {
    cols: u16,
    rows: u16,
}

#[derive(Serialize, Clone, Debug)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum OutEvent {
    /// 首个事件：连接与 shell 已就绪
    Ready,
    Output {
        data: String,
    },
    Closed {
        reason: String,
    },
}

/// 输出往哪儿送：界面标签页走 UI 事件队列，MCP 的无头会话落到自己的环形缓冲。
pub type Sink = Arc<dyn Fn(OutEvent) + Send + Sync + 'static>;

fn connect_args(args: &OpenArgs) -> ssh::ConnectArgs {
    ssh::ConnectArgs {
        host: args.host.clone(),
        port: args.port,
        username: args.username.clone(),
        password: args.password.clone(),
        key_path: args.key_path.clone(),
        methods: args.methods.clone(),
    }
}

async fn session_logic(
    args: OpenArgs,
    sink: Sink,
    mut input_rx: mpsc::Receiver<String>,
    mut win_rx: tokio::sync::watch::Receiver<Win>,
    mgr: Shared,
    ready: oneshot::Sender<Result<(), String>>,
) {
    let id = args.id.clone();
    let mut ready = Some(ready);
    let result: Result<()> = async {
        let idle = args.keepalive_secs.unwrap_or(ssh::KEEPALIVE_SECS);
        let handle = ssh::connect(&connect_args(&args), idle).await?;

        let channel = handle.channel_open_session().await?;
        let win = Win {
            cols: args.cols.max(2),
            rows: args.rows.max(2),
        };
        let (mut r, w) = channel.split();
        w.request_pty(
            true,
            "xterm-256color",
            win.cols as u32,
            win.rows as u32,
            0,
            0,
            &[],
        )
        .await?;
        w.request_shell(true).await?;

        let writer = tokio::spawn(async move {
            loop {
                select! {
                    msg = input_rx.recv() => match msg {
                        Some(s) => {
                            if w.data_bytes(s.into_bytes()).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    },
                    changed = win_rx.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        let w0 = *win_rx.borrow_and_update();
                        let _ = w.window_change(w0.cols as u32, w0.rows as u32, 0, 0).await;
                    }
                }
            }
            let _ = w.close().await;
        });

        let _ = ready.take().map(|tx| tx.send(Ok(())));
        sink(OutEvent::Ready);

        let mut reader = r.make_reader();
        let mut buf = vec![0u8; BUF_SIZE];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => sink(OutEvent::Output {
                    data: String::from_utf8_lossy(&buf[..n]).into_owned(),
                }),
            }
        }
        writer.abort();
        Ok(())
    }
    .await;

    if let Some(t) = mgr.lock().await.terminals.remove(&id) {
        t.join.abort();
    }
    let reason = match result {
        Ok(()) => "连接已断开".to_string(),
        Err(e) => {
            let msg = ssh::format_err(e);
            if ready.is_some() {
                // 还没就绪就失败：原因只经 open() 的 Err 回传一次，不再往终端里灌重复文本
                let _ = ready.take().map(|tx| tx.send(Err(msg)));
                return;
            }
            msg
        }
    };
    sink(OutEvent::Closed { reason });
}

struct Terminal {
    input: mpsc::Sender<String>,
    win: tokio::sync::watch::Sender<Win>,
    join: tokio::task::JoinHandle<()>,
}

pub struct TerminalManager {
    terminals: HashMap<String, Terminal>,
}

pub type Shared = Arc<Mutex<TerminalManager>>;

pub fn new_manager() -> Shared {
    Arc::new(Mutex::new(TerminalManager {
        terminals: HashMap::new(),
    }))
}

pub async fn open(mgr: &Shared, args: OpenArgs, sink: Sink) -> Result<(), String> {
    let id = args.id.clone();
    let (input_tx, input_rx) = mpsc::channel::<String>(256);
    let (win_tx, win_rx) = tokio::sync::watch::channel(Win {
        cols: args.cols,
        rows: args.rows,
    });
    let (ready_tx, ready_rx) = oneshot::channel::<Result<(), String>>();

    {
        let mut g = mgr.lock().await;
        if g.terminals.contains_key(&id) {
            return Err("该标签页的连接尚未关闭".into());
        }
        let join = tokio::spawn(session_logic(
            args,
            sink,
            input_rx,
            win_rx,
            mgr.clone(),
            ready_tx,
        ));
        g.terminals.insert(
            id.clone(),
            Terminal {
                input: input_tx,
                win: win_tx,
                join,
            },
        );
    }

    match ready_rx.await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => {
            remove(mgr, &id).await;
            Err(e)
        }
        Err(_) => {
            remove(mgr, &id).await;
            Err("会话启动失败".into())
        }
    }
}

async fn remove(mgr: &Shared, id: &str) {
    if let Some(t) = mgr.lock().await.terminals.remove(id) {
        t.join.abort();
    }
}

pub async fn send_input(mgr: &Shared, id: String, data: String) -> Result<(), String> {
    let sender = mgr
        .lock()
        .await
        .terminals
        .get(&id)
        .map(|t| t.input.clone())
        .ok_or("会话不存在")?;
    sender
        .send(data)
        .await
        .map_err(|_| "会话已关闭".to_string())
}

pub async fn resize(mgr: &Shared, id: String, cols: u16, rows: u16) -> Result<(), String> {
    let tx = mgr
        .lock()
        .await
        .terminals
        .get(&id)
        .map(|t| t.win.clone())
        .ok_or("会话不存在")?;
    let _ = tx.send(Win {
        cols: cols.max(2),
        rows: rows.max(2),
    });
    Ok(())
}

pub async fn close(mgr: Shared, id: String) {
    if let Some(t) = mgr.lock().await.terminals.remove(&id) {
        drop(t.input);
        let _ = tokio::time::timeout(std::time::Duration::from_millis(300), t.join).await;
    }
}
