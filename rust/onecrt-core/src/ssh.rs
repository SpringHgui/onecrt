//! SSH 连接的三块地基：连接配置、主机密钥策略与认证顺序。

use anyhow::{anyhow, Context, Result};
use russh::client::{self, Handle, KeyboardInteractiveAuthResponse};
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg};
use std::sync::Arc;

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
    async fn check_server_key(
        &mut self,
        _key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
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
                        .authenticate_publickey(
                            user,
                            PrivateKeyWithHashAlg::new(Arc::new(key.clone()), None),
                        )
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
                                let answers =
                                    prompts.iter().map(|_| password.to_string()).collect();
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

/// 建连 + 认证。终端、文件面板、性能面板三条通道都从这里起步。
pub(crate) async fn connect(args: &ConnectArgs, idle_secs: u64) -> Result<Handle<ShellHandler>> {
    let key = load_key(&args.key_path);
    let addr = format!("{}:{}", args.host, args.port);
    let mut handle = client::connect(client_config(idle_secs), addr.as_str(), ShellHandler)
        .await
        .with_context(|| format!("无法连接到 {addr}"))?;
    authenticate(
        &mut handle,
        &args.username,
        &args.password,
        key,
        &args.methods,
    )
    .await?;
    Ok(handle)
}

/// 三条通道共用的连接参数（不含终端尺寸之类各自的东西）
#[derive(Clone, Debug, Default)]
pub struct ConnectArgs {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub key_path: String,
    pub methods: Vec<String>,
}
