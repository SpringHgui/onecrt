//! MCP 服务：把 onecrt 已存的 SSH 会话能力暴露给外部 agent（Qoder / Claude Desktop 等）。
//!
//! 三条底线：默认关闭、只听 127.0.0.1、每个请求都要带 `Authorization: Bearer <token>`。
//! agent 侧拿不到口令——它只能按会话名点用 onecrt 已存的凭据，工具输出里也不回传凭据内容。

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
use std::time::{Duration, Instant};
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
    enabled: bool,
    port: u16,
    token: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            enabled: false,
            port: DEFAULT_PORT,
            token: random_hex(16),
        }
    }
}

fn config_path() -> Result<PathBuf, String> {
    let base = dirs::config_dir().ok_or_else(|| "无法获取配置目录".to_string())?;
    let dir = base.join("shellx");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("mcp.json"))
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
    std::fs::write(path, text).map_err(|e| e.to_string())
}

fn random_hex(n_bytes: usize) -> String {
    let mut buf = vec![0u8; n_bytes];
    getrandom::fill(&mut buf).expect("系统随机源不可用");
    buf.iter().map(|b| format!("{b:02x}")).collect()
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

#[derive(Clone, Debug, Serialize)]
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

pub fn new_manager(term: terminal::Shared) -> Shared {
    let path = config_path().unwrap_or_else(|_| PathBuf::from("mcp.json"));
    new_manager_at(term, path)
}

pub fn new_manager_at(term: terminal::Shared, path: PathBuf) -> Shared {
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
    pub enabled: bool,
    pub running: bool,
    pub port: u16,
    pub token: String,
    pub url: String,
    pub config_file: String,
    pub error: Option<String>,
    pub agents: Vec<AgentInfo>,
}

fn status_of(rt: &Runtime) -> Status {
    // 任务起来了又死掉（panic 或 serve 出错）时不能继续报"监听中"，
    // 那会让界面上绿字、端口上没人。
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
    let running = matches!(&rt.server, Some((_, join)) if !join.is_finished());
    Status {
        enabled: rt.cfg.enabled,
        running,
        port: rt.cfg.port,
        token: rt.cfg.token.clone(),
        url: format!("http://127.0.0.1:{}/mcp", rt.cfg.port),
        config_file: rt.cfg_file.display().to_string(),
        error: rt.error.clone().or(task_err),
        agents: rt
            .agents
            .lock()
            .unwrap()
            .values()
            .map(|a| {
                let g = a.buf.lock().unwrap();
                AgentInfo {
                    terminal_id: a.terminal_id.clone(),
                    session_name: a.session_name.clone(),
                    host: a.host.clone(),
                    closed: g.closed.is_some(),
                    out_bytes: g.total,
                    last_command: g.last_command.clone(),
                }
            })
            .collect(),
    }
}

pub async fn status(state: &Shared) -> Status {
    status_of(&state.lock().unwrap())
}

/// 按上次落盘的开关在启动时拉起服务。
pub async fn boot(state: &Shared) {
    let enabled = state.lock().unwrap().cfg.enabled;
    if enabled {
        let _ = start(state).await;
    }
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

/// 配置改动落盘，再按新配置重启监听（开关和端口得重开，令牌不用）。
async fn apply(state: &Shared) -> Result<Status, String> {
    let (enabled, cfg, cfg_file) = {
        let rt = state.lock().unwrap();
        (rt.cfg.enabled, rt.cfg.clone(), rt.cfg_file.clone())
    };
    save_config(&cfg, &cfg_file)?;
    stop(state).await;
    if enabled {
        start(state).await?;
    }
    Ok(status_of(&state.lock().unwrap()))
}

pub async fn set_enabled(state: &Shared, enabled: bool) -> Result<Status, String> {
    state.lock().unwrap().cfg.enabled = enabled;
    match apply(state).await {
        Ok(st) => Ok(st),
        Err(e) => {
            // 起不来就把开关落回关闭，别让界面显示"已开"却根本没在监听
            state.lock().unwrap().cfg.enabled = false;
            let _ = apply(state).await;
            Err(e)
        }
    }
}

pub async fn set_port(state: &Shared, port: u16) -> Result<Status, String> {
    if port < 1024 {
        return Err("端口要大于等于 1024".into());
    }
    state.lock().unwrap().cfg.port = port;
    apply(state).await
}

/// 换令牌只改内存里的共享单元和 mcp.json，不重启监听：鉴权是逐请求现读的，
/// 重绑同一个端口反而会被刚关闭的套接字挡住。
pub async fn rotate_token(state: &Shared) -> Result<Status, String> {
    let (cfg, cfg_file) = {
        let mut rt = state.lock().unwrap();
        rt.cfg.token = random_hex(16);
        *rt.token.write().unwrap() = rt.cfg.token.clone();
        (rt.cfg.clone(), rt.cfg_file.clone())
    };
    save_config(&cfg, &cfg_file)?;
    Ok(status_of(&state.lock().unwrap()))
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
    let raw = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    // 各家客户端都发 `Bearer <token>`，容忍裸 token 免得撞上一个不规范的
    let given = raw.strip_prefix("Bearer ").unwrap_or(raw);
    if !given.is_empty() && given == token {
        Ok(next.run(req).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
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
}
