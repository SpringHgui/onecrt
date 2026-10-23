//! 端到端验证 SSH 链路：进程内起一个 russh 服务端，驱动真实的客户端代码。
use russh::keys::PrivateKey;
use russh::server::{Msg, Session as ServerSession};
use russh::{ChannelId, Pty};
use shellx_lib::terminal::{self, OpenArgs, OutEvent, Shared};
use std::sync::{Arc, Mutex};
use tauri::ipc::{Channel, InvokeResponseBody};
use tokio::net::TcpListener;

#[derive(Default)]
struct Obs {
    pty: Option<(u32, u32)>,
    shell: bool,
    winsizes: Vec<(u32, u32)>,
}

#[derive(Clone)]
struct Srv {
    obs: Arc<Mutex<Obs>>,
    chan: Arc<Mutex<Option<ChannelId>>>,
}

impl Srv {
    fn channel(&self) -> Option<ChannelId> {
        *self.chan.lock().unwrap()
    }

    async fn send(&self, session: &mut ServerSession, text: &str) {
        if let Some(ch) = self.channel() {
            let _ = session.handle().data(ch, text.as_bytes().to_vec()).await;
        }
    }
}

impl russh::server::Server for Srv {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        self.clone()
    }
}

impl russh::server::Handler for Srv {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<russh::server::Auth, Self::Error> {
        if user == "root" && password == "secret" {
            Ok(russh::server::Auth::Accept)
        } else {
            Ok(russh::server::Auth::reject())
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: russh::Channel<Msg>,
        _session: &mut ServerSession,
    ) -> Result<bool, Self::Error> {
        *self.chan.lock().unwrap() = Some(channel.id());
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        self.obs.lock().unwrap().pty = Some((col_width, row_height));
        let _ = session.channel_success(channel);
        Ok(())
    }

    async fn shell_request(&mut self, channel: ChannelId, session: &mut ServerSession) -> Result<(), Self::Error> {
        self.obs.lock().unwrap().shell = true;
        let _ = session.channel_success(channel);
        self.send(session, "READY-BANNER\r\n").await;
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        self.obs.lock().unwrap().winsizes.push((col_width, row_height));
        let _ = session.channel_success(channel);
        self.send(session, &format!("WINSIZE:{}x{}\r\n", col_width, row_height))
            .await;
        Ok(())
    }

    async fn data(&mut self, _channel: ChannelId, data: &[u8], session: &mut ServerSession) -> Result<(), Self::Error> {
        let text = String::from_utf8_lossy(data).to_string();
        self.send(session, &format!("ECHO:{}\r\n", text)).await;
        Ok(())
    }
}

/// 仅测试用的临时主机密钥（tests/fixtures/test_host_key），不对应任何真实凭据
const TEST_HOST_KEY_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test_host_key");

async fn start_server() -> (u16, Arc<Mutex<Obs>>, tokio::task::JoinHandle<()>) {
    let obs = Arc::new(Mutex::new(Obs::default()));
    let srv = Srv {
        obs: obs.clone(),
        chan: Arc::new(Mutex::new(None)),
    };
    let config = Arc::new(russh::server::Config {
        auth_rejection_time: std::time::Duration::ZERO,
        keys: vec![PrivateKey::read_openssh_file(TEST_HOST_KEY_PATH).unwrap()],
        ..Default::default()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else { break };
            let config = config.clone();
            let handler = srv.clone();
            tokio::spawn(async move {
                let _ = russh::server::run_stream(config, stream, handler).await;
            });
        }
    });
    (port, obs, task)
}

fn args_for(port: u16, password: &str) -> OpenArgs {
    OpenArgs {
        id: "t1".into(),
        host: "127.0.0.1".into(),
        port,
        username: "root".into(),
        password: password.into(),
        key_path: String::new(),
        methods: vec![],
        cols: 100,
        rows: 30,
    }
}

fn recording_channel(log: Arc<Mutex<Vec<String>>>) -> Channel<OutEvent> {
    Channel::new(move |body| {
        if let InvokeResponseBody::Json(json) = body {
            log.lock().unwrap().push(json);
        }
        Ok(())
    })
}

async fn wait_for(log: &Arc<Mutex<Vec<String>>>, needle: &str, secs: u64) -> bool {
    for _ in 0..secs * 20 {
        if log.lock().unwrap().join("").contains(needle) {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    false
}

async fn wait_until<F: FnMut(&mut Obs) -> bool>(obs: &Arc<Mutex<Obs>>, mut pred: F, secs: u64) {
    for _ in 0..secs * 20 {
        if pred(&mut obs.lock().unwrap()) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn connects_authenticates_and_streams_shell_io() {
    let (port, obs, _server) = start_server().await;
    let m = terminal::new_manager();
    let log = Arc::new(Mutex::new(Vec::new()));

    terminal::open(&m, args_for(port, "secret"), recording_channel(log.clone()))
        .await
        .expect("open 应当成功");

    wait_until(&obs, |o| o.pty.is_some() && o.shell, 5).await;
    assert_eq!(obs.lock().unwrap().pty, Some((100, 30)), "pty 尺寸应来自客户端");
    assert!(obs.lock().unwrap().shell, "服务端应收到 shell 请求");
    assert!(wait_for(&log, "READY-BANNER", 5).await, "应收到服务端欢迎串");

    terminal::send_input(&m, "t1".into(), "ls\r".into())
        .await
        .expect("send_input");
    assert!(wait_for(&log, "ECHO:ls", 5).await, "输入应送达服务端并回显");

    terminal::resize(&m, "t1".into(), 132, 44).await.expect("resize");
    assert!(
        wait_for(&log, "WINSIZE:132x44", 5).await,
        "窗口尺寸变更应送达服务端"
    );

    terminal::close(m.clone(), "t1".into()).await;
    assert!(terminal::send_input(&m, "t1".into(), "x".into())
        .await
        .is_err());
}

#[tokio::test]
async fn wrong_password_fails_with_clear_error() {
    let (port, _obs, _server) = start_server().await;
    let m: Shared = terminal::new_manager();
    let log = Arc::new(Mutex::new(Vec::new()));
    let err = terminal::open(&m, args_for(port, "wrong"), recording_channel(log.clone()))
        .await
        .expect_err("错误密码不应连上");
    assert!(err.contains("认证"), "错误信息应说明认证失败，实际: {err}");
    // 启动期失败只经返回值回报一次，不应再往终端灌重复的 Output/Closed
    assert!(log.lock().unwrap().is_empty(), "失败路径不应产生事件: {:?}", log.lock().unwrap());
}

#[tokio::test]
async fn unreachable_host_reports_error() {
    // 端口 1 基本不可能有 ssh 服务
    let m = terminal::new_manager();
    let log = Arc::new(Mutex::new(Vec::new()));
    let err = terminal::open(&m, args_for(1, "x"), recording_channel(log.clone()))
        .await
        .expect_err("连不上时应报错");
    assert!(err.contains("无法连接"), "错误信息应说明连接失败，实际: {err}");
    assert!(log.lock().unwrap().is_empty(), "失败路径不应产生事件: {:?}", log.lock().unwrap());
}
