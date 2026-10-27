use anyhow::{anyhow, Context, Result};
use russh::client::{self, Handle, KeyboardInteractiveAuthResponse};
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::select;
use tokio::sync::{mpsc, oneshot, Mutex};

const BUF_SIZE: usize = 32768;

/// 空闲多久发一次保活包。内网的防火墙/NAT 会掐掉长时间无包的 TCP，
/// sshd 的 ClientAliveInterval 也是从「上次收到任何包」开始计时的，30 秒足以压住两边。
pub const KEEPALIVE_SECS: u64 = 30;

/// 三条 SSH 通道（终端 / 文件面板 / 性能面板）共用的连接配置。
/// keepalive_max 沿用 russh 默认 3：连续 3 次无回应即判定链路已死，主动断开。
pub fn client_config(idle_secs: u64) -> Arc<client::Config> {
    Arc::new(client::Config {
        keepalive_interval: (idle_secs > 0).then(|| std::time::Duration::from_secs(idle_secs)),
        ..Default::default()
    })
}

#[derive(Deserialize)]
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

#[derive(Serialize, Clone)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum OutEvent {
    /// 首个事件：连接与 shell 已就绪
    Ready,
    Output { data: String },
    Closed { reason: String },
}

/// 输出往哪儿送：界面标签页走 Tauri IPC 通道，MCP 的无头会话落到自己的环形缓冲。
pub trait OutSink: Send + Sync + 'static {
    fn emit(&self, event: OutEvent);
}

impl<F> OutSink for F
where
    F: Fn(OutEvent) + Send + Sync + 'static,
{
    fn emit(&self, event: OutEvent) {
        self(event)
    }
}

pub type Sink = Arc<dyn OutSink>;

/// 把前端传来的 IPC 通道包装成 Sink
pub fn channel_sink(channel: tauri::ipc::Channel<OutEvent>) -> Sink {
    Arc::new(move |event| {
        let _ = channel.send(event);
    })
}

pub(crate) fn format_err(e: anyhow::Error) -> String {
    let s = format!("{e:#}");
    match s.find(" caused by ") {
        Some(i) => s[i + " caused by ".len()..].to_string(),
        None => s,
    }
}

pub(crate) struct ShellHandler;

impl client::Handler for ShellHandler {
    type Error = russh::Error;

    // 内网运维工具：自动接受主机密钥，不做首次指纹确认（刻意省略弹框）
    async fn check_server_key(&mut self, _key: &russh::keys::PublicKey) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

pub(crate) async fn authenticate(
    handle: &mut Handle<ShellHandler>,
    user: &str,
    password: &str,
    key: Option<PrivateKey>,
    methods: &[String],
) -> Result<()> {
    let legacy = ["PublicKey", "Password", "Keyboard Interactive"].map(str::to_string);
    let order: &[String] = if methods.is_empty() { &legacy } else { methods };
    for method in order {
        match method.as_str() {
            "PublicKey" => {
                if let Some(key) = &key {
                    let auth = handle
                        .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key.clone()), None))
                        .await
                        .context("公钥认证失败")?;
                    if auth.success() {
                        return Ok(());
                    }
                }
            }
            "Password" => {
                if !password.is_empty() {
                    let auth = handle
                        .authenticate_password(user, password)
                        .await
                        .context("密码认证失败")?;
                    if auth.success() {
                        return Ok(());
                    }
                }
            }
            "Keyboard Interactive" => {
                // 部分服务器只接受 keyboard-interactive
                if !password.is_empty() {
                    let mut resp = handle
                        .authenticate_keyboard_interactive_start(user, None)
                        .await?;
                    loop {
                        match resp {
                            KeyboardInteractiveAuthResponse::Success => return Ok(()),
                            KeyboardInteractiveAuthResponse::Failure { .. } => break,
                            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                                let answers = prompts.iter().map(|_| password.to_string()).collect();
                                resp = handle
                                    .authenticate_keyboard_interactive_respond(answers)
                                    .await?;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Err(anyhow!("认证失败，请检查用户名、密码或密钥"))
}

fn expand_key_path(p: &str) -> String {
    let home = dirs::home_dir().unwrap_or_default();
    p.replace("~\\", &format!("{}\\", home.display()))
        .replace("~/", &format!("{}/", home.display()))
}

pub(crate) fn load_key(key_path: &str) -> Option<PrivateKey> {
    let p = key_path.trim();
    if p.is_empty() {
        return None;
    }
    let expanded = expand_key_path(p);
    PrivateKey::read_openssh_file(&expanded)
        .or_else(|_| PrivateKey::read_openssh_file(p))
        .ok()
}

/// 公钥指纹，供会话属性里的 Public Key Properties 展示
pub fn fingerprint(key_path: &str) -> Result<String, String> {
    let p = key_path.trim();
    if p.is_empty() {
        return Err("未设置身份文件".into());
    }
    let expanded = expand_key_path(p);
    let key = PrivateKey::read_openssh_file(&expanded)
        .or_else(|_| PrivateKey::read_openssh_file(p))
        .map_err(|e| format!("读不了密钥文件: {e}"))?;
    Ok(format!(
        "{} {}",
        key.algorithm(),
        key.public_key().fingerprint(russh::keys::HashAlg::Sha256)
    ))
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
        let key = load_key(&args.key_path);
        let addr = format!("{}:{}", args.host, args.port);
        let idle = args.keepalive_secs.unwrap_or(KEEPALIVE_SECS);
        let mut handle = client::connect(client_config(idle), addr.as_str(), ShellHandler)
            .await
            .with_context(|| format!("无法连接到 {addr}"))?;
        authenticate(&mut handle, &args.username, &args.password, key, &args.methods).await?;

        let channel = handle.channel_open_session().await?;
        let win = Win {
            cols: args.cols.max(2),
            rows: args.rows.max(2),
        };
        let (mut r, w) = channel.split();
        w.request_pty(true, "xterm-256color", win.cols as u32, win.rows as u32, 0, 0, &[])
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
        sink.emit(OutEvent::Ready);

        let mut reader = r.make_reader();
        let mut buf = vec![0u8; BUF_SIZE];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => sink.emit(OutEvent::Output {
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
            let msg = format_err(e);
            if ready.is_some() {
                // 还没就绪就失败：原因只经 open() 的 Err 回传一次，不再往终端里灌重复文本
                let _ = ready.take().map(|tx| tx.send(Err(msg)));
                return;
            }
            msg
        }
    };
    sink.emit(OutEvent::Closed { reason });
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
        let join = tokio::spawn(session_logic(args, sink, input_rx, win_rx, mgr.clone(), ready_tx));
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
    sender.send(data).await.map_err(|_| "会话已关闭".to_string())
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
