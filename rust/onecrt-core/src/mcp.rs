//! MCP 服务：把 onecrt 已存的 SSH 会话能力暴露给外部 agent（Qoder / Claude Desktop 等）。
//!
//! 服务跑在独立的后台进程 `onecrt-mcpd` 里，默认登记开机（登录）自启，主程序开不开都在监听；
//! 主程序只写 `mcp.json` 并读它写的 `mcpd.json` 心跳，两边都不按 PID 管对方。
//! 三条底线不变：只听 127.0.0.1、每个请求都要带 `Authorization: Bearer <token>`、
//! agent 侧拿不到口令——它只能按会话名点用 onecrt 已存的凭据，工具输出里也不回传凭据内容。

use crate::daemon;
use crate::sessions::{self, Node, SshSession};
use crate::terminal::{self, OpenArgs, OutEvent};
use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::Response,
    Router,
};
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router,
    transport::{
        streamable_http_server::{session::local::LocalSessionManager, StreamableHttpService},
        StreamableHttpServerConfig,
    },
    ErrorData as McpError, ServerHandler,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio_util::sync::CancellationToken;

/// 缺省端口，避开 8xxx 这类常被 web 服务占用的号段；端口可在设置弹窗里改。
pub const DEFAULT_PORT: u16 = 39117;

/// 每个 agent 会话的输出环形缓冲上限：只留最近的字节，长命令不会把内存吃光。
const BUF_CAP: usize = 1024 * 1024;

/// 一次工具调用回传给 agent 的输出上限，超出部分给截断说明和续读偏移。
const RESULT_CAP: usize = 64 * 1024;

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_TIMEOUT_SECS: u64 = 600;

/// 无头会话的 PTY 尺寸：宽一点，命令的列对齐不容易被折行打断。
const AGENT_COLS: u16 = 200;
const AGENT_ROWS: u16 = 50;

const INSTRUCTIONS: &str = "OneCRT is a desktop SSH client holding saved sessions for internal hosts. \
Call sessions_list to discover targets, then exec to run one command with the credentials OneCRT already \
has (a password is never accepted or returned). exec reuses one live shell per saved session, so cd and \
exported variables carry across calls; use send/read for interactive programs, and close when done.";

// ------------------------------------------------------------------ 配置与运行态

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    /// 后台服务开关，同时是开机自启的登记状态：开着它才有人登录时自启、界面才会拉起它
    enabled: bool,
    port: u16,
    token: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            enabled: true,
            port: DEFAULT_PORT,
            token: random_hex(16),
        }
    }
}

/// shellx 配置目录，`mcp.json` 与 `mcpd.json` 都落在这儿
fn config_dir() -> Result<PathBuf, String> {
    let base = dirs::config_dir().ok_or_else(|| "无法获取配置目录".to_string())?;
    let dir = base.join("shellx");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn config_path() -> Result<PathBuf, String> {
    Ok(config_dir()?.join("mcp.json"))
}

/// 后台进程的心跳文件：界面靠它判断在不在监听，不靠进程表
fn heartbeat_path() -> Result<PathBuf, String> {
    Ok(config_dir()?.join("mcpd.json"))
}

/// 读配置。文件缺失或坏掉（含 token 为空）都退回默认值并覆盖写回，
/// 免得一个残缺文件把服务永久卡在没有可用 token 的状态。
fn load_config(path: &Path) -> Result<Config, String> {
    if path.exists() {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        if let Ok(cfg) = serde_json::from_str::<Config>(&text) {
            if !cfg.token.is_empty() {
                return Ok(cfg);
            }
        }
    }
    let cfg = Config::default();
    save_config(&cfg, path)?;
    Ok(cfg)
}

fn save_config(cfg: &Config, path: &Path) -> Result<(), String> {
    let text = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    daemon::write_atomic(path, &text)
}

fn random_hex(n_bytes: usize) -> String {
    let mut buf = vec![0u8; n_bytes];
    getrandom::fill(&mut buf).expect("系统随机源不可用");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// 后台进程每秒写一次的心跳。不带令牌——它写在自己和界面都能读的明文文件里，
/// 而界面要令牌直接读 mcp.json 就有了，别在第二个文件里再复制一份秘密。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Heartbeat {
    pub pid: u32,
    /// 实际绑上的端口（端口被占去重试时也是这个）
    pub port: u16,
    pub listening: bool,
    pub error: Option<String>,
    /// 写文件的 Unix 秒
    pub at: u64,
    #[serde(default)]
    pub agents: Vec<AgentInfo>,
}

impl Heartbeat {
    fn is_live(&self, now: u64) -> bool {
        daemon::is_live(self.at, now)
    }
}

/// agent 开出来的一个无头 SSH 会话，连着它的输出缓冲。
struct Agent {
    terminal_id: String,
    /// 对应的已存会话 id：复用按它找，界面标签页从来不进这张表
    session_id: String,
    session_name: String,
    host: String,
    buf: Mutex<Buf>,
    /// read 省略 from 时从这里接着读
    cursor: AtomicU64,
}

struct Buf {
    data: VecDeque<u8>,
    /// 被挤掉的字节数：绝对偏移要能换算回缓冲里的位置
    dropped: u64,
    total: u64,
    closed: Option<String>,
    /// 最后发出去的那行，设置弹窗用它显示 agent 在敲什么
    last_command: String,
}

impl Buf {
    fn push(&mut self, s: &str) {
        self.total += s.len() as u64;
        for b in s.bytes() {
            self.data.push_back(b);
        }
        while self.data.len() > BUF_CAP {
            self.data.pop_front();
            self.dropped += 1;
        }
    }

    /// 缓冲里最早那个字节的绝对偏移：等于被挤掉的字节数。
    /// 写成 `total - dropped` 会让 read_from 每次把整个缓冲当增量回给 agent
    /// （横幅重复 N 遍，N=轮询次数），且逢报"前缀已丢弃"。
    fn base(&self) -> u64 {
        self.dropped
    }
}

impl Agent {
    fn push(&self, ev: OutEvent) {
        let mut g = self.buf.lock().unwrap();
        match ev {
            OutEvent::Output { data } => g.push(&data),
            OutEvent::Ready => {}
            OutEvent::Closed { reason } => g.closed = Some(reason),
        }
    }

    fn closed(&self) -> Option<String> {
        self.buf.lock().unwrap().closed.clone()
    }

    fn total(&self) -> u64 {
        self.buf.lock().unwrap().total
    }

    /// 从绝对偏移起读，返回（文本, 末尾偏移, 是否丢了前缀）
    fn read_from(&self, from: u64) -> (String, u64, bool) {
        let g = self.buf.lock().unwrap();
        let base = g.base();
        let start = from.max(base);
        let lost = from < base;
        let bytes: Vec<u8> = g
            .data
            .iter()
            .skip((start - base) as usize)
            .copied()
            .collect();
        (String::from_utf8_lossy(&bytes).into_owned(), g.total, lost)
    }

    fn remember_command(&self, text: &str) {
        let line = text.lines().next().unwrap_or("");
        let short: String = line.chars().take(120).collect();
        self.buf.lock().unwrap().last_command = short;
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub terminal_id: String,
    pub session_name: String,
    pub host: String,
    pub closed: bool,
    pub out_bytes: u64,
    pub last_command: String,
}

pub struct Runtime {
    cfg: Config,
    /// mcp.json 的位置；测试注入临时文件，不碰 %APPDATA%
    cfg_file: PathBuf,
    server: Option<(CancellationToken, tokio::task::JoinHandle<()>)>,
    /// 监听任务自己报的退出原因（serve 出错或任务 panic）
    serve_fail: Arc<Mutex<Option<String>>>,
    /// 鉴权中间件每次请求现读的令牌。放在共享单元里而不是拷给中间件，
    /// 这样换令牌只改这个值和 mcp.json，不用重启监听（重启会在同一端口上撞 10048）。
    token: Arc<RwLock<String>>,
    agents: Arc<Mutex<HashMap<String, Arc<Agent>>>>,
    term: terminal::Shared,
    /// 起不来的原因（多半是端口被占），设置弹窗要显示，别静默不监听
    error: Option<String>,
}

pub type Shared = Arc<Mutex<Runtime>>;

/// 只有后台进程用：界面不再持有服务运行态，它读文件。
fn new_manager_at(term: terminal::Shared, path: PathBuf) -> Shared {
    let (cfg, error) = match load_config(&path) {
        Ok(cfg) => (cfg, None),
        Err(e) => (
            Config::default(),
            Some(format!("mcp.json 读不了，本次按默认配置处理: {e}")),
        ),
    };
    let token = Arc::new(RwLock::new(cfg.token.clone()));
    Arc::new(Mutex::new(Runtime {
        cfg,
        cfg_file: path,
        server: None,
        serve_fail: Arc::new(Mutex::new(None)),
        token,
        agents: Arc::new(Mutex::new(HashMap::new())),
        term,
        error,
    }))
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// 开关状态与「有没有在监听」是两件事：开着开关但后台进程起不来时 enabled=true、running=false
    pub enabled: bool,
    pub running: bool,
    pub port: u16,
    pub token: String,
    pub url: String,
    pub config_file: String,
    pub error: Option<String>,
    pub agents: Vec<AgentInfo>,
}

// ------------------------------------------------------------------ 界面侧
//
// 界面不再监听端口：它只写 mcp.json、管开机自启登记，在不在监听只看后台进程的心跳文件。

/// 界面看到的 MCP 状态：开关/端口/令牌来自 mcp.json，是否真在监听来自后台进程心跳。
pub fn client_status() -> Status {
    let cfg_file = config_path().unwrap_or_else(|_| PathBuf::from("mcp.json"));
    status_at(&cfg_file, &cfg_file.with_file_name("mcpd.json"))
}

fn status_at(cfg_file: &Path, hb_file: &Path) -> Status {
    let (cfg, config_err) = match load_config(cfg_file) {
        Ok(cfg) => (cfg, None),
        Err(e) => (
            Config::default(),
            Some(format!("mcp.json 读不了，本次按默认配置处理: {e}")),
        ),
    };
    // 只采信还活着的心跳：残留文件会把界面钉在一个早就没人听的旧端口或旧错误上
    let beat = read_heartbeat_at(hb_file).filter(|b| b.is_live(now_secs()));
    let port = beat.as_ref().map(|b| b.port).unwrap_or(cfg.port);
    Status {
        enabled: cfg.enabled,
        running: beat.as_ref().is_some_and(|b| b.listening),
        port,
        token: cfg.token.clone(),
        url: format!("http://127.0.0.1:{port}/mcp"),
        config_file: cfg_file.display().to_string(),
        error: config_err.or_else(|| beat.as_ref().and_then(|b| b.error.clone())),
        agents: beat.map(|b| b.agents).unwrap_or_default(),
    }
}

fn update_config(f: impl FnOnce(&mut Config)) -> Result<Config, String> {
    let path = config_path()?;
    let mut cfg = load_config(&path)?;
    f(&mut cfg);
    save_config(&cfg, &path)?;
    Ok(cfg)
}

/// 有活着的心跳就算有人在跑，哪怕它正卡在端口被占的重试上——那时再拉一个是抢端口
fn daemon_alive() -> bool {
    heartbeat_path()
        .ok()
        .and_then(|p| read_heartbeat_at(&p))
        .is_some_and(|b| b.is_live(now_secs()))
}

/// 没人在跑就补一个后台进程；已经在跑则什么都不做
fn ensure_running() -> Result<(), String> {
    if daemon_alive() {
        return Ok(());
    }
    daemon::spawn()
}

/// 开：落盘 + 登记登录时自启 + 没人在跑就立刻拉起后台进程。
/// 关：落盘 + 撤掉自启；正在跑的后台进程下一秒读到开关关掉会自己收工（连带拆掉 agent
/// 的无头 shell），所以界面不需要按 PID 动它。
pub fn set_enabled(enabled: bool) -> Result<Status, String> {
    update_config(|cfg| cfg.enabled = enabled)?;
    let mut note = None;
    if enabled {
        if let Err(e) = daemon::enable_autostart() {
            note = Some(e);
        }
        if let Err(e) = ensure_running() {
            note = note.or(Some(e));
        }
    } else {
        // 自启项删不掉也不该拦住「关闭」这个动作
        let _ = daemon::disable_autostart();
    }
    let mut status = client_status();
    if note.is_some() {
        status.error = note;
    }
    Ok(status)
}

/// 改端口只落盘：在跑的后台进程下一秒重绑新端口，没在跑且开关开着就趁机拉起来
pub fn set_port(port: u16) -> Result<Status, String> {
    if port < 1024 {
        return Err("端口要大于等于 1024".into());
    }
    let cfg = update_config(|cfg| cfg.port = port)?;
    if cfg.enabled {
        ensure_running()?;
    }
    Ok(client_status())
}

/// 换令牌只落盘：后台进程每秒现读配置，鉴权又是逐请求取最新值，两边都不用重绑端口
pub fn rotate_token() -> Result<Status, String> {
    update_config(|cfg| cfg.token = random_hex(16))?;
    Ok(client_status())
}

/// 主程序启动时补齐后台服务的状态：开关是开的就得有自启登记和一个在跑的进程。
/// 自启登记只在发布构建里做，免得开发期 `cargo run` 把 target/debug 的路径写进系统启动项。
pub fn boot() -> Result<(), String> {
    if !load_config(&config_path()?)?.enabled {
        return Ok(());
    }
    if !cfg!(debug_assertions) {
        let _ = daemon::enable_autostart();
    }
    ensure_running()
}

// ------------------------------------------------------------------ 后台进程侧

/// 心跳与配置轮询的间隔：界面改完配置最多等一秒就在状态栏上见效
const TICK: Duration = Duration::from_secs(1);

/// 后台进程主循环：按 mcp.json 监听，每秒跟一次配置改动，开关关掉就自己收工。
pub async fn run(term: terminal::Shared) -> Result<(), String> {
    let state = new_manager_at(term, config_path()?);
    let hb_file = heartbeat_path()?;
    if let Some(beat) = read_heartbeat_at(&hb_file) {
        // 开机自启已经拉起过一个了，界面再点「启动」不该开出第二个
        if beat.is_live(now_secs()) && beat.pid != std::process::id() {
            return Ok(());
        }
    }
    if !state.lock().unwrap().cfg.enabled {
        // 开关关掉之后残留的自启项不该继续监听
        return Ok(());
    }

    serve(&state).await;
    write_heartbeat(&state);
    loop {
        tokio::time::sleep(TICK).await;
        if !reload(&state).await || peer_listens(&state) {
            break;
        }
        write_heartbeat(&state);
    }
    // 收工前再写一次，界面立刻显示「未监听」，不用等心跳过期
    write_heartbeat(&state);
    Ok(())
}

/// 该监听却没有活着的任务时补一个。端口被占时不退出：原因写进 error，下一秒再来，
/// 用户改端口或者占着端口的进程消失之后它自己就好。
async fn serve(state: &Shared) {
    reap(state);
    if state.lock().unwrap().cfg.enabled {
        let _ = start(state).await;
    }
}

/// 监听任务已经退出（serve 报错或 panic）时摘掉句柄、把原因留在 error 里，
/// 下一轮 start() 才会在端口空出来之后重新绑上。
fn reap(state: &Shared) {
    let mut rt = state.lock().unwrap();
    let dead = matches!(&rt.server, Some((_, join)) if join.is_finished());
    if !dead {
        return;
    }
    rt.server = None;
    let fail = rt.serve_fail.lock().unwrap().take();
    let old = rt.error.take();
    let reason = fail.or(old).unwrap_or_else(|| "监听任务已退出".to_string());
    rt.error = Some(reason);
}

/// 自己没在监听、却有另一个进程报「我在监听」时该安静退出（两个后台进程几乎同时起来
/// 抢端口时，抢输的那个别用自己的失败心跳盖掉对方的状态）。
fn peer_listens(state: &Shared) -> bool {
    if matches!(&state.lock().unwrap().server, Some((_, join)) if !join.is_finished()) {
        return false;
    }
    let path = heartbeat_path().ok();
    path.and_then(|p| read_heartbeat_at(&p))
        .is_some_and(|b| b.pid != std::process::id() && b.is_live(now_secs()) && b.listening)
}

/// 把 mcp.json 的改动落到运行态：令牌直接换、端口重绑、开关关掉就整体退出。
/// 返回 false 表示服务已被关掉，主循环该收工。
async fn reload(state: &Shared) -> bool {
    let (path, prev) = {
        let rt = state.lock().unwrap();
        (rt.cfg_file.clone(), rt.cfg.clone())
    };
    // 读不到就当这一秒什么都没变：一个写坏的配置文件不该把在跑的服务带倒
    let next = match load_config(&path) {
        Ok(cfg) => cfg,
        Err(_) => return true,
    };
    if next.token != prev.token {
        *state.lock().unwrap().token.write().unwrap() = next.token.clone();
    }
    let port_changed = next.port != prev.port;
    let enabled = next.enabled;
    state.lock().unwrap().cfg = next;
    if !enabled {
        stop(state).await;
        return false;
    }
    if port_changed {
        stop(state).await;
    }
    serve(state).await;
    true
}

fn write_heartbeat(state: &Shared) {
    let beat = heartbeat_of(&state.lock().unwrap());
    if let (Ok(path), Ok(text)) = (heartbeat_path(), serde_json::to_string(&beat)) {
        let _ = daemon::write_atomic(&path, &text);
    }
}

/// 心跳内容直接从运行态算：任务起来了又死掉（panic 或 serve 出错）时不能继续报「监听中」，
/// 那会让界面上绿字、端口上没人。
fn heartbeat_of(rt: &Runtime) -> Heartbeat {
    let task_err = match &rt.server {
        Some((_, join)) if join.is_finished() => Some(
            rt.serve_fail
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| "监听任务已退出".to_string()),
        ),
        _ => None,
    };
    Heartbeat {
        pid: std::process::id(),
        port: rt.cfg.port,
        listening: matches!(&rt.server, Some((_, join)) if !join.is_finished()),
        error: rt.error.clone().or(task_err),
        at: now_secs(),
        agents: status_of_agents(&rt.agents),
    }
}

fn read_heartbeat_at(path: &Path) -> Option<Heartbeat> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<Heartbeat>(&text).ok()
}

async fn start(state: &Shared) -> Result<(), String> {
    let (port, token, term, agents, serve_fail) = {
        let rt = state.lock().unwrap();
        if rt.server.is_some() {
            return Ok(());
        }
        (
            rt.cfg.port,
            rt.token.clone(),
            rt.term.clone(),
            rt.agents.clone(),
            rt.serve_fail.clone(),
        )
    };

    let addr = format!("127.0.0.1:{port}");
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            let msg = format!("监听 {addr} 失败: {e}");
            state.lock().unwrap().error = Some(msg.clone());
            return Err(msg);
        }
    };
    let actual_port = listener.local_addr().map_err(|e| e.to_string())?.port();

    let ct = CancellationToken::new();
    let service = StreamableHttpService::new(
        move || Ok(OnecrtMcp::new(term.clone(), agents.clone())),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default()
            .with_cancellation_token(ct.child_token())
            .with_json_response(true),
    );
    let app = Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(token, guard));

    let join = tokio::spawn(async move {
        // 不做 graceful shutdown：stop() abort 这个任务，正在跑的 SSE 随连接一起断
        if let Err(e) = axum::serve(listener, app).await {
            *serve_fail.lock().unwrap() = Some(format!("监听退出: {e}"));
        }
    });

    let mut rt = state.lock().unwrap();
    rt.server = Some((ct, join));
    *rt.serve_fail.lock().unwrap() = None;
    rt.cfg.port = actual_port;
    rt.error = None;
    Ok(())
}

/// 停服务顺手把 agent 的 shell 全拆掉：关掉之后不该还有别的进程能借道连到内网。
async fn stop(state: &Shared) {
    let old = state.lock().unwrap().server.take();
    if let Some((ct, join)) = old {
        ct.cancel();
        join.abort();
        // 非等到任务真退出不可：listener 就在那个任务手里，没释放就重绑同一个端口，
        // 实测会撞上"通常每个套接字地址(协议/网络地址/端口)只允许使用一次 (os error 10048)"。
        let _ = join.await;
    }
    let (agents, term) = {
        let rt = state.lock().unwrap();
        (rt.agents.clone(), rt.term.clone())
    };
    let ids: Vec<String> = agents.lock().unwrap().keys().cloned().collect();
    for id in ids {
        terminal::close(term.clone(), id.clone()).await;
        agents.lock().unwrap().remove(&id);
    }
}

/// 只认 `Authorization: Bearer <token>`。虽然绑在回环上，本机其它进程照样连得上，
/// 这一层不能省。
async fn guard(
    State(cell): State<Arc<RwLock<String>>>,
    headers: HeaderMap,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // 现读：重新生成令牌之后不用重启监听，下一个请求就用新值判
    let token = cell.read().unwrap().clone();
    if bearer_ok(headers.get(axum::http::header::AUTHORIZATION), &token) {
        Ok(next.run(req).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

/// 各家客户端都发 `Bearer <token>`，容忍裸 token 免得撞上一个不规范的；
/// 空值一律拒，别让「没带鉴权头」和一个恰好为空的令牌等价。
fn bearer_ok(header: Option<&axum::http::HeaderValue>, token: &str) -> bool {
    let raw = header.and_then(|v| v.to_str().ok()).unwrap_or("");
    let given = raw.strip_prefix("Bearer ").unwrap_or(raw);
    !given.is_empty() && given == token
}

// ------------------------------------------------------------------ MCP 工具

#[derive(Clone)]
pub struct OnecrtMcp {
    term: terminal::Shared,
    agents: Arc<Mutex<HashMap<String, Arc<Agent>>>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ExecArgs {
    /// Saved session id or name, as sessions_list reports them
    session: String,
    /// One command line to run on the remote host
    command: String,
    /// Reuse the shell already held for this session (default true); false forces a fresh login
    #[serde(default)]
    reuse: Option<bool>,
    /// Stop waiting after this many seconds; default 30, max 600
    #[serde(default)]
    timeout_seconds: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SendArgs {
    /// terminalId as returned by exec
    terminal_id: String,
    /// Bytes to type into the PTY; a newline is appended when the text has none
    data: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ReadArgs {
    /// terminalId as returned by exec
    terminal_id: String,
    /// Absolute output offset to read from; omit to continue after the previous read
    #[serde(default)]
    from: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct CloseArgs {
    /// terminalId as returned by exec
    terminal_id: String,
}

/// 命令后面追的结束标记。PTY 会回显整行 echo，但回显里是字面量 `$?`，
/// 只有真执行完才打印出数字，所以按「前缀+数字」匹配不会把回显当成结果。
fn marker_prefix() -> String {
    format!("ONECRT-MCP-{}:", random_hex(4))
}

/// 见到标记后的数字就给出退出码；解析不出数字时算结束但退出码未知
fn find_exit(text: &str, prefix: &str) -> Option<Option<u8>> {
    let at = text.rfind(prefix)? + prefix.len();
    let digits: Vec<u8> = text[at..]
        .bytes()
        .take_while(|b| b.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&digits).parse::<u8>().ok())
}

/// 结束标记那两行（echo 命令的回显和它的输出）不进正文
fn strip_marker(text: &str, prefix: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        if line.contains(prefix) {
            continue;
        }
        out.push_str(line.trim_end_matches('\r'));
        out.push('\n');
    }
    out
}

fn clip(text: &str) -> String {
    if text.len() <= RESULT_CAP {
        return text.to_string();
    }
    // 按字符边界切，别把中文切成半个字节序列
    let mut cut = RESULT_CAP;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!(
        "{}\n… 已截断：只回了前 {cut} 字节，其余用 read 接着取\n",
        &text[..cut]
    )
}

fn credentials_of(s: &SshSession) -> &'static str {
    match (!s.password.is_empty(), !s.key_path.trim().is_empty()) {
        (true, true) => "password+key",
        (true, false) => "password",
        (false, true) => "key",
        (false, false) => "none",
    }
}

/// 展平会话树，每项带上 `文件夹/子文件夹/会话名` 路径
fn all_sessions() -> Result<Vec<(String, SshSession)>, String> {
    let tree = sessions::load_tree()?;
    let mut out = Vec::new();
    walk(&tree.sessions, "", &mut out);
    Ok(out)
}

fn walk(nodes: &[Node], parent: &str, out: &mut Vec<(String, SshSession)>) {
    for n in nodes {
        match n {
            Node::Folder { name, children, .. } => {
                let path = if parent.is_empty() {
                    name.clone()
                } else {
                    format!("{parent}/{name}")
                };
                walk(children, &path, out);
            }
            Node::Session(s) => {
                let path = if parent.is_empty() {
                    s.name.clone()
                } else {
                    format!("{parent}/{}", s.name)
                };
                out.push((path, s.clone()));
            }
        }
    }
}

/// 先按 id 精确找，再按整条路径、最后按叶子名比对（后两者不区分大小写）
fn find_session(want: &str) -> Option<SshSession> {
    let all = all_sessions().ok()?;
    let wl = want.to_lowercase();
    all.iter()
        .find(|(_, s)| s.id == want)
        .or_else(|| all.iter().find(|(path, _)| path.to_lowercase() == wl))
        .or_else(|| {
            all.iter()
                .find(|(path, _)| path.rsplit('/').next().unwrap_or(path).to_lowercase() == wl)
        })
        .map(|(_, s)| s.clone())
}

#[tool_router]
impl OnecrtMcp {
    fn new(term: terminal::Shared, agents: Arc<Mutex<HashMap<String, Arc<Agent>>>>) -> Self {
        OnecrtMcp { term, agents }
    }

    #[tool(
        name = "sessions_list",
        description = "List saved SSH sessions as id / path / host:port / user / credentials, without any secret. Call this first to discover what you may run against."
    )]
    async fn sessions_list(&self) -> Result<CallToolResult, McpError> {
        let sessions = match all_sessions() {
            Ok(v) => v,
            Err(e) => return tool_error(e),
        };
        if sessions.is_empty() {
            return tool_ok("no saved sessions".into());
        }
        let out = sessions
            .iter()
            .map(|(path, s)| {
                format!(
                    "id={}\tpath={}\thost={}:{}\tuser={}\tcredentials={}\n",
                    s.id,
                    path,
                    s.host,
                    s.port,
                    s.username,
                    credentials_of(s)
                )
            })
            .collect::<String>();
        tool_ok(out)
    }

    #[tool(
        name = "terminals_list",
        description = "List the remote shells this service currently holds open, with terminalId, target, state and the last line typed."
    )]
    async fn terminals_list(&self) -> Result<CallToolResult, McpError> {
        let rows: Vec<String> = status_of_agents(&self.agents)
            .iter()
            .map(|a| {
                format!(
                    "terminalId={}\tsession={}\thost={}\tstate={}\toutBytes={}\tlast={}\n",
                    a.terminal_id,
                    a.session_name,
                    a.host,
                    if a.closed { "closed" } else { "connected" },
                    a.out_bytes,
                    a.last_command
                )
            })
            .collect();
        if rows.is_empty() {
            return tool_ok("no open shells".into());
        }
        tool_ok(rows.concat())
    }

    #[tool(
        name = "exec",
        description = "Run one command line on a saved SSH session using the credentials OneCRT stores, wait for it to finish, and return its output with the exit code. The shell stays open and is reused per session, so cd and exported variables carry across calls."
    )]
    async fn exec(
        &self,
        Parameters(args): Parameters<ExecArgs>,
    ) -> Result<CallToolResult, McpError> {
        let session = match find_session(&args.session) {
            Some(s) => s,
            None => {
                return tool_error(format!(
                    "no saved session matching {:?} — call sessions_list",
                    args.session
                ));
            }
        };
        if credentials_of(&session) == "none" {
            return tool_error(format!(
                "session {:?} has no stored password or key; connect it once in OneCRT and save the credentials",
                args.session
            ));
        }
        let timeout = args
            .timeout_seconds
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .clamp(1, MAX_TIMEOUT_SECS);

        let agent = match self.agent_for(&session, args.reuse.unwrap_or(true)).await {
            Ok(agent) => agent,
            Err(e) => return tool_error(e),
        };

        // 从此刻已有的输出往后收：前面的是登录横幅和上一条命令的结果
        let start = agent.total();
        let prefix = marker_prefix();
        agent.remember_command(&args.command);
        if let Err(e) = self.send_line(&agent, &args.command).await {
            return tool_error(e);
        }
        if let Err(e) = self
            .send_line_raw(&agent, &format!("echo \"{prefix}$?\"\n"))
            .await
        {
            return tool_error(e);
        }

        let done = wait_done(
            &agent,
            start,
            &prefix,
            Instant::now() + Duration::from_secs(timeout),
        )
        .await;
        agent.cursor.store(done.next, Ordering::Relaxed);

        let mut head = format!(
            "terminalId={}\tsession={}\thost={}\texit={}\n",
            agent.terminal_id, agent.session_name, agent.host, done.exit
        );
        if done.lost_prefix {
            head.push_str("(earlier output was dropped from the buffer)\n");
        }
        if let Some(note) = done.note {
            head.push_str(&note);
        }
        tool_ok(format!(
            "{head}--- output ---\n{}--- end ---\nreadFrom={}",
            clip(&done.text),
            done.next
        ))
    }

    #[tool(
        name = "send",
        description = "Type into a shell opened by exec, for interactive programs that ask questions or need control keys. Nothing is returned: call read afterwards."
    )]
    async fn send(
        &self,
        Parameters(args): Parameters<SendArgs>,
    ) -> Result<CallToolResult, McpError> {
        let agent = match self.agent(&args.terminal_id) {
            Some(a) => a,
            None => return tool_error(format!("unknown terminalId {:?}", args.terminal_id)),
        };
        agent.remember_command(&args.data);
        match self.send_line(&agent, &args.data).await {
            Ok(()) => tool_ok(format!(
                "sent {} bytes to {}; call read to collect",
                args.data.chars().count(),
                args.terminal_id
            )),
            Err(e) => tool_error(e),
        }
    }

    #[tool(
        name = "read",
        description = "Read output a shell produced since the last read (or since an explicit offset). Returns the text plus readFrom, the offset to continue from."
    )]
    async fn read(
        &self,
        Parameters(args): Parameters<ReadArgs>,
    ) -> Result<CallToolResult, McpError> {
        let agent = match self.agent(&args.terminal_id) {
            Some(a) => a,
            None => return tool_error(format!("unknown terminalId {:?}", args.terminal_id)),
        };
        let from = args
            .from
            .unwrap_or_else(|| agent.cursor.load(Ordering::Relaxed));
        let (text, next, lost) = agent.read_from(from);
        agent.cursor.store(next, Ordering::Relaxed);
        let mut note = String::new();
        if lost {
            note.push_str("(earlier output was dropped from the buffer)\n");
        }
        if let Some(reason) = agent.closed() {
            note.push_str(&format!("(shell closed: {reason})\n"));
        }
        tool_ok(format!(
            "{note}--- output ---\n{}--- end ---\nreadFrom={next}",
            clip(&text)
        ))
    }

    #[tool(
        name = "close",
        description = "Close a shell opened by exec and forget it. The saved session and the user's own tabs are untouched."
    )]
    async fn close(
        &self,
        Parameters(args): Parameters<CloseArgs>,
    ) -> Result<CallToolResult, McpError> {
        if self.agent(&args.terminal_id).is_none() {
            return tool_error(format!("unknown terminalId {:?}", args.terminal_id));
        }
        terminal::close(self.term.clone(), args.terminal_id.clone()).await;
        self.agents.lock().unwrap().remove(&args.terminal_id);
        tool_ok(format!("closed {}", args.terminal_id))
    }
}

#[tool_handler]
impl ServerHandler for OnecrtMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("onecrt", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}

fn status_of_agents(agents: &Arc<Mutex<HashMap<String, Arc<Agent>>>>) -> Vec<AgentInfo> {
    let g = agents.lock().unwrap();
    let mut rows: Vec<AgentInfo> = g
        .values()
        .map(|a| {
            let b = a.buf.lock().unwrap();
            AgentInfo {
                terminal_id: a.terminal_id.clone(),
                session_name: a.session_name.clone(),
                host: a.host.clone(),
                closed: b.closed.is_some(),
                out_bytes: b.total,
                last_command: b.last_command.clone(),
            }
        })
        .collect();
    rows.sort_by(|x, y| x.terminal_id.cmp(&y.terminal_id));
    rows
}

impl OnecrtMcp {
    fn agent(&self, terminal_id: &str) -> Option<Arc<Agent>> {
        self.agents.lock().unwrap().get(terminal_id).cloned()
    }

    /// 取（或新开）这个已存会话对应的 shell。已断连的旧 shell 摘掉重开。
    async fn agent_for(&self, session: &SshSession, reuse: bool) -> Result<Arc<Agent>, String> {
        if reuse {
            let hit = self
                .agents
                .lock()
                .unwrap()
                .values()
                .find(|a| a.session_id == session.id)
                .cloned();
            if let Some(agent) = hit {
                if agent.closed().is_none() {
                    return Ok(agent);
                }
                self.agents.lock().unwrap().remove(&agent.terminal_id);
            }
        }

        let id = format!("mcp-{}", random_hex(4));
        let agent = Arc::new(Agent {
            terminal_id: id.clone(),
            session_id: session.id.clone(),
            session_name: session.name.clone(),
            host: format!("{}:{}", session.host, session.port),
            buf: Mutex::new(Buf {
                data: VecDeque::new(),
                dropped: 0,
                total: 0,
                closed: None,
                last_command: String::new(),
            }),
            cursor: AtomicU64::new(0),
        });
        let held = agent.clone();
        let sink: terminal::Sink = Arc::new(move |ev| held.push(ev));
        let args = OpenArgs {
            id: id.clone(),
            host: session.host.clone(),
            port: session.port,
            username: session.username.clone(),
            password: session.password.clone(),
            key_path: session.key_path.clone(),
            methods: session
                .auth
                .iter()
                .filter(|m| m.enabled)
                .map(|m| m.method.clone())
                .collect(),
            cols: AGENT_COLS,
            rows: AGENT_ROWS,
            keepalive_secs: None,
        };
        // open() 到 ready 才返回；失败的 Err 已经是认证/网络的具体原因
        if let Err(e) = terminal::open(&self.term, args, sink).await {
            return Err(format!("连接 {} 失败: {e}", agent.host));
        }
        self.agents
            .lock()
            .unwrap()
            .insert(id.clone(), agent.clone());
        Ok(agent)
    }

    async fn send_line(&self, agent: &Agent, text: &str) -> Result<(), String> {
        let data = if text.ends_with('\n') {
            text.to_string()
        } else {
            format!("{text}\n")
        };
        self.send_line_raw(agent, &data).await
    }

    async fn send_line_raw(&self, agent: &Agent, data: &str) -> Result<(), String> {
        terminal::send_input(&self.term, agent.terminal_id.clone(), data.to_string()).await
    }
}

struct Done {
    text: String,
    next: u64,
    lost_prefix: bool,
    exit: String,
    note: Option<String>,
}

/// 等到结束标记打出来 / shell 断了 / 超时。只增量取新字节，避免每次轮询重拷整个缓冲。
async fn wait_done(agent: &Agent, start: u64, prefix: &str, deadline: Instant) -> Done {
    let mut seen = start;
    let mut acc = String::new();
    let mut lost_prefix = false;
    loop {
        let (delta, next, lost) = agent.read_from(seen);
        seen = next;
        lost_prefix |= lost;
        acc.push_str(&delta);
        if let Some(code) = find_exit(&acc, prefix) {
            return Done {
                text: strip_marker(&acc, prefix),
                next,
                lost_prefix,
                exit: match code {
                    Some(c) => c.to_string(),
                    None => "?".into(),
                },
                note: None,
            };
        }
        if let Some(reason) = agent.closed() {
            return Done {
                text: acc,
                next,
                lost_prefix,
                exit: "n/a".into(),
                note: Some(format!("(shell closed while waiting: {reason})\n")),
            };
        }
        if Instant::now() >= deadline {
            return Done {
                text: acc,
                next,
                lost_prefix,
                exit: "n/a".into(),
                note: Some(format!(
                    "(still running after {}s — send Ctrl-C to interrupt, then read again)\n",
                    deadline.elapsed().as_secs()
                )),
            };
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn tool_ok(text: String) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

fn tool_error(text: String) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::error(vec![ContentBlock::text(text)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf() -> Buf {
        Buf {
            data: VecDeque::new(),
            dropped: 0,
            total: 0,
            closed: None,
            last_command: String::new(),
        }
    }

    fn agent_with(g: Buf) -> Agent {
        Agent {
            terminal_id: "mcp-test".into(),
            session_id: "s-test".into(),
            session_name: "s-test".into(),
            host: "127.0.0.1:22".into(),
            buf: Mutex::new(g),
            cursor: AtomicU64::new(0),
        }
    }

    /// read_from 只能给 from 之后的增量：偏移算反过一次，让 wait_done 每轮都把整个
    /// 缓冲当新输出累加，真跑 exec 时登录横幅被回给 agent 六遍。
    #[test]
    fn read_from_returns_only_the_delta() {
        let mut g = buf();
        g.push("Welcome to Example !\n");
        let after_banner = g.total;
        g.push("[root@host ~]# uname -a\nLinux host\n");
        let agent = agent_with(g);

        let (text, next, lost) = agent.read_from(after_banner);
        assert_eq!(
            text, "[root@host ~]# uname -a\nLinux host\n",
            "只该拿到横幅之后的字节"
        );
        assert!(!lost, "偏移没落在被挤掉的范围里不该报丢弃");
        assert_eq!(next, agent.total(), "续读偏移就是当前末尾");

        // 接着读没有新字节，不该再吐出任何旧内容
        let (again, _, _) = agent.read_from(next);
        assert_eq!(again, "", "读完之后再读应为空");
    }

    #[test]
    fn read_from_from_zero_gives_whole_buffer() {
        let mut g = buf();
        g.push("abc");
        g.push("def");
        let agent = agent_with(g);
        let (text, next, lost) = agent.read_from(0);
        assert_eq!(text, "abcdef");
        assert!(!lost);
        assert_eq!(next, 6);
    }

    /// 缓冲挤掉前缀之后：base 是被挤掉的字节数，比它小的 from 才算丢
    #[test]
    fn wrap_drops_the_head_and_marks_it_lost() {
        let mut g = buf();
        let filler = "x".repeat(BUF_CAP + 1000);
        g.push(&filler);
        let total = g.total;
        let agent = agent_with(g);

        assert_eq!(
            agent.buf.lock().unwrap().base(),
            1000,
            "最早存活字节的绝对偏移"
        );
        assert_eq!(agent.buf.lock().unwrap().data.len(), BUF_CAP);
        assert_eq!(total, BUF_CAP as u64 + 1000);

        let (text, _, lost) = agent.read_from(0);
        assert!(lost, "from 早于 base 才该报丢弃");
        assert_eq!(text.len(), BUF_CAP);

        let (_, _, lost2) = agent.read_from(1000);
        assert!(!lost2, "正好落在缓冲起点不该报丢弃");

        let (tail, next, _) = agent.read_from(total - 3);
        assert_eq!(tail, "xxx");
        assert_eq!(next, total);
    }

    #[test]
    fn bearer_check_takes_prefixed_or_bare_tokens_only() {
        fn h(v: &str) -> Option<axum::http::HeaderValue> {
            axum::http::HeaderValue::from_str(v).ok()
        }
        assert!(bearer_ok(h("Bearer abc").as_ref(), "abc"));
        assert!(bearer_ok(h("abc").as_ref(), "abc"), "容忍裸令牌");
        assert!(!bearer_ok(h("Bearer wrong").as_ref(), "abc"));
        assert!(!bearer_ok(None, "abc"), "没带鉴权头要拒");
        assert!(
            !bearer_ok(h("").as_ref(), "abc"),
            "空值不能和一个恰好为空的令牌等价"
        );
        assert!(
            !bearer_ok(h("Bearer ").as_ref(), ""),
            "令牌为空时不该有能过鉴权的写法"
        );
    }

    /// 心跳里写的是实际绑上的端口：界面只信 mcp.json 会把 agent 指到一个没人听的端口
    #[test]
    fn status_follows_a_live_heartbeat() {
        let dir = case_dir("live");
        let cfg = dir.join("mcp.json");
        let hb = dir.join("mcpd.json");
        save_config(
            &Config {
                enabled: true,
                port: DEFAULT_PORT,
                token: "abc".into(),
            },
            &cfg,
        )
        .unwrap();
        let beat = Heartbeat {
            pid: 4321,
            port: 40001,
            listening: true,
            error: None,
            at: now_secs(),
            agents: vec![AgentInfo {
                terminal_id: "mcp-1".into(),
                session_name: "web-1".into(),
                host: "10.0.0.1:22".into(),
                closed: false,
                out_bytes: 12,
                last_command: "uname -a".into(),
            }],
        };
        daemon::write_atomic(&hb, &serde_json::to_string(&beat).unwrap()).unwrap();

        let st = status_at(&cfg, &hb);
        assert!(st.running, "心跳新鲜又在监听就该报在监听");
        assert_eq!(st.port, 40001, "端口跟着心跳走，不能停在配置里的默认值");
        assert_eq!(st.url, "http://127.0.0.1:40001/mcp");
        assert_eq!(st.agents.len(), 1, "agent 会话列表由心跳带回来");
        assert_eq!(
            st.token, "abc",
            "令牌只从 mcp.json 读，心跳文件里不该有一份"
        );
        assert_eq!(st.error, None);
        cleanup(&dir);
    }

    /// 后台进程被杀之后心跳文件还留在原地：越过 STALE_SECS 就不能再信
    #[test]
    fn stale_heartbeat_falls_back_to_the_configured_port() {
        let dir = case_dir("stale");
        let cfg = dir.join("mcp.json");
        let hb = dir.join("mcpd.json");
        save_config(
            &Config {
                enabled: true,
                port: DEFAULT_PORT,
                token: "abc".into(),
            },
            &cfg,
        )
        .unwrap();
        let beat = Heartbeat {
            pid: 4321,
            port: 40001,
            listening: true,
            error: None,
            at: now_secs() - daemon::STALE_SECS - 1,
            agents: vec![AgentInfo {
                terminal_id: "mcp-1".into(),
                session_name: "web-1".into(),
                host: "10.0.0.1:22".into(),
                closed: false,
                out_bytes: 12,
                last_command: String::new(),
            }],
        };
        daemon::write_atomic(&hb, &serde_json::to_string(&beat).unwrap()).unwrap();

        let st = status_at(&cfg, &hb);
        assert!(
            !st.running,
            "心跳过期就报没在监听，哪怕文件里写着 listening"
        );
        assert!(st.enabled, "开关状态来自配置，与进程死活无关");
        assert_eq!(st.port, DEFAULT_PORT, "没有活的进程时回落到配置里的端口");
        assert!(
            st.agents.is_empty(),
            "别把上一个进程的 agent 会话算到当前状态上"
        );
        cleanup(&dir);
    }

    /// 端口被别的进程占了：进程活着但没在监听，原因必须上界面，别静默显示绿字
    #[test]
    fn live_but_not_listening_shows_the_reason() {
        let dir = case_dir("busy");
        let cfg = dir.join("mcp.json");
        let hb = dir.join("mcpd.json");
        save_config(
            &Config {
                enabled: true,
                port: DEFAULT_PORT,
                token: "abc".into(),
            },
            &cfg,
        )
        .unwrap();
        let beat = Heartbeat {
            pid: 4321,
            port: DEFAULT_PORT,
            listening: false,
            error: Some("监听 127.0.0.1:39117 失败: os error 10048".into()),
            at: now_secs(),
            agents: vec![],
        };
        daemon::write_atomic(&hb, &serde_json::to_string(&beat).unwrap()).unwrap();

        let st = status_at(&cfg, &hb);
        assert!(!st.running);
        let err = st.error.expect("端口冲突的原因要带给界面");
        assert!(err.contains("10048"), "{err}");
        cleanup(&dir);
    }

    /// 换令牌不重绑端口：旧令牌立刻失效、新令牌能过鉴权。
    /// 重绑反而会被刚关闭的套接字挡住（os error 10048），这条是「免重启」设计的落点。
    #[tokio::test]
    async fn rotated_token_takes_effect_without_rebinding_the_port() {
        let dir = case_dir("rotate");
        let cfg = dir.join("mcp.json");
        let port = free_port();
        save_config(
            &Config {
                enabled: true,
                port,
                token: "old".into(),
            },
            &cfg,
        )
        .unwrap();
        let state = new_manager_at(crate::terminal::new_manager(), cfg.clone());
        start(&state).await.expect("起监听");

        let url = format!("http://127.0.0.1:{port}/mcp");
        let http = reqwest::Client::new();
        let denied = http
            .get(&url)
            .bearer_auth("wrong")
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(
            denied,
            StatusCode::UNAUTHORIZED,
            "错误令牌要挡在鉴权中间件外"
        );

        // 走真路径：界面只改文件，后台进程那一轮 reload 把它读进来
        save_config(
            &Config {
                enabled: true,
                port,
                token: "new".into(),
            },
            &cfg,
        )
        .unwrap();
        assert!(reload(&state).await, "只换令牌不该让主循环收工");
        assert!(
            matches!(&state.lock().unwrap().server, Some((_, join)) if !join.is_finished()),
            "端口没改就不该停掉监听任务"
        );

        let still_old = http
            .get(&url)
            .bearer_auth("old")
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(
            still_old,
            StatusCode::UNAUTHORIZED,
            "换完之后旧令牌立刻失效"
        );
        let accepted = http
            .get(&url)
            .bearer_auth("new")
            .send()
            .await
            .unwrap()
            .status();
        assert_ne!(
            accepted,
            StatusCode::UNAUTHORIZED,
            "新令牌该穿过鉴权中间件落到 MCP 路由"
        );

        stop(&state).await;
        cleanup(&dir);
    }

    /// 改端口要停掉旧监听再绑新的：旧端口必须真还回来，否则重绑撞 10048
    #[tokio::test]
    async fn port_change_moves_the_listener() {
        let dir = case_dir("port");
        let cfg = dir.join("mcp.json");
        let first = free_port();
        let second = free_port();
        save_config(
            &Config {
                enabled: true,
                port: first,
                token: "abc".into(),
            },
            &cfg,
        )
        .unwrap();
        let state = new_manager_at(crate::terminal::new_manager(), cfg.clone());
        start(&state).await.expect("起监听");
        assert!(
            std::net::TcpListener::bind(("127.0.0.1", first)).is_err(),
            "测试前提：旧端口该被监听占着"
        );

        save_config(
            &Config {
                enabled: true,
                port: second,
                token: "abc".into(),
            },
            &cfg,
        )
        .unwrap();
        assert!(reload(&state).await);
        assert!(
            std::net::TcpListener::bind(("127.0.0.1", first)).is_ok(),
            "旧端口该还回来"
        );
        let beat = heartbeat_of(&state.lock().unwrap());
        assert!(beat.listening, "新端口该已经绑上");
        assert_eq!(beat.port, second, "心跳要报实际在听的端口");

        stop(&state).await;
        cleanup(&dir);
    }

    /// 关掉开关：主循环收到「收工」，监听停、端口还回来
    #[tokio::test]
    async fn turning_the_switch_off_stops_serving() {
        let dir = case_dir("off");
        let cfg = dir.join("mcp.json");
        let port = free_port();
        save_config(
            &Config {
                enabled: true,
                port,
                token: "abc".into(),
            },
            &cfg,
        )
        .unwrap();
        let state = new_manager_at(crate::terminal::new_manager(), cfg.clone());
        start(&state).await.expect("起监听");

        save_config(
            &Config {
                enabled: false,
                port,
                token: "abc".into(),
            },
            &cfg,
        )
        .unwrap();
        assert!(!reload(&state).await, "开关关掉要告诉主循环可以退出");
        assert!(state.lock().unwrap().server.is_none(), "监听任务该摘干净");
        assert!(
            std::net::TcpListener::bind(("127.0.0.1", port)).is_ok(),
            "端口该还给用户"
        );
        cleanup(&dir);
    }

    /// 找一个暂时空闲的端口给测试用
    fn free_port() -> u16 {
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("找不到空闲端口");
        let port = l.local_addr().expect("读不到端口").port();
        drop(l);
        port
    }

    fn case_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "onecrt-mcp-{}-{tag}-{}",
            std::process::id(),
            random_hex(4)
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录失败");
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }
}
