//! MCP 服务的协议层回归：用 rmcp 自带的客户端真连一次，
//! 验证「没 token 进不来」「有 token 能列工具并跑一个不碰磁盘的工具」。
use rmcp::{
    ClientLifecycleMode, ClientServiceExt,
    model::{
        CallToolResult, ClientCapabilities, ClientConfig, ContentBlock, Implementation,
        ProtocolVersion,
    },
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use onecrt_lib::{mcp, terminal};
use std::path::PathBuf;

const TOKEN: &str = "test-token-0123456789abcdef";

fn temp_cfg_path(case: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("onecrt-mcp-{case}-{}.json", std::process::id()));
    p
}

/// 每个用例一份临时 mcp.json：port 写 0 让内核挑空闲端口，用例之间不抢 39117，
/// 也不碰 %APPDATA% 里用户自己的配置。
fn temp_cfg(case: &str) -> PathBuf {
    let p = temp_cfg_path(case);
    let _ = std::fs::remove_file(&p);
    let text = format!(r#"{{"enabled":false,"port":0,"token":"{TOKEN}"}}"#);
    std::fs::write(&p, text).expect("写临时配置");
    p
}

async fn running(case: &str) -> (mcp::Shared, String) {
    let state = mcp::new_manager_at(terminal::new_manager(), temp_cfg(case));
    let st = mcp::set_enabled(&state, true).await.expect("开启 MCP 服务");
    assert!(st.running, "开关打开后应该在监听");
    assert_eq!(st.token, TOKEN, "token 取自配置文件");
    // 监听任务一 spawn 就死过一回（应用里界面还显示"监听中"），缓一拍再确认端口真的还在
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let st = mcp::status(&state).await;
    assert!(st.running, "200ms 后仍应监听，error={:?}", st.error);
    (state, st.url)
}

fn transport(url: &str, token: &str) -> StreamableHttpClientTransport<reqwest::Client> {
    // 上游把 reqwest 编成 rustls-no-provider 档，不装 provider 连 Client::new() 都会 panic
    let _ = rustls::crypto::ring::default_provider().install_default();
    StreamableHttpClientTransport::with_client(
        reqwest::Client::new(),
        StreamableHttpClientTransportConfig::with_uri(url).auth_header(token),
    )
}

async fn handshake(url: &str, token: &str) -> Result<String, Box<dyn std::error::Error>> {
    let client = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("onecrt-test", env!("CARGO_PKG_VERSION")),
    )
    .serve_with_lifecycle(
        transport(url, token),
        ClientLifecycleMode::Auto {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            legacy_version: Some(ProtocolVersion::V_2025_11_25),
        },
    )
    .await?;
    let name = client
        .peer_info()
        .and_then(|i| i.server_info.clone())
        .map(|s| s.name)
        .unwrap_or_default();
    let _ = client.cancel().await;
    Ok(name)
}

fn text_of(res: &CallToolResult) -> String {
    res.content
        .iter()
        .filter_map(|c| match c {
            ContentBlock::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

#[tokio::test]
async fn wrong_token_cannot_complete_handshake() {
    let (state, url) = running("reject").await;
    let err = handshake(&url, "not-the-token").await.expect_err("错 token 不该连上");
    assert!(format!("{err:?}").contains("401"), "应当被 token 拦在 401: {err:?}");
    mcp::set_enabled(&state, false).await.expect("关掉");
}

/// 握手 → 列工具 → 调一个不落盘的工具
async fn tools_flow(url: &str) {
    let name = handshake(url, TOKEN).await.expect("带 token 应连上");
    assert_eq!(name, "onecrt", "握手回来的服务名");

    let client = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("onecrt-test", env!("CARGO_PKG_VERSION")),
    )
    .serve_with_lifecycle(
        transport(url, TOKEN),
        ClientLifecycleMode::Auto {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            legacy_version: Some(ProtocolVersion::V_2025_11_25),
        },
    )
    .await
    .expect("握手");

    let mut names: Vec<String> = client
        .list_tools(Default::default())
        .await
        .expect("列工具")
        .tools
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    names.sort();
    for want in ["close", "exec", "read", "send", "sessions_list", "terminals_list"] {
        assert!(names.iter().any(|n| n == want), "缺少工具 {want}，实际 {names:?}");
    }

    let res = client
        .call_tool(rmcp::model::CallToolRequestParams::new("terminals_list"))
        .await
        .expect("调用工具");
    assert!(!res.is_error.unwrap_or(false), "刚起来时不该报错");
    assert_eq!(text_of(&res), "no open shells");

    let _ = client.cancel().await;
}

#[tokio::test]
async fn right_token_lists_tools_and_runs_one() {
    let (state, url) = running("tools").await;
    tools_flow(&url).await;
    mcp::set_enabled(&state, false).await.expect("关掉");
}

/// 重新生成令牌只是换密钥，不该把服务拆掉。以前它走 stop+start 重绑同一个端口，
/// 刚关闭的套接字还占着地址，实测结果是令牌换了、界面显示「未监听」、
/// 报 os error 10048 —— 而且 rotate 不像开关那样会回滚，服务就这么瘫了。
#[tokio::test]
async fn rotate_token_keeps_the_service_up() {
    let (state, url) = running("rotate").await;
    let st = mcp::rotate_token(&state).await.expect("重新生成令牌");
    assert!(st.running, "换令牌后仍应监听，error={:?}", st.error);
    assert_eq!(st.url, url, "端口不该变");
    assert_ne!(st.token, TOKEN, "令牌确实换了");

    let text = std::fs::read_to_string(temp_cfg_path("rotate")).expect("读回配置");
    assert!(text.contains(&st.token), "新令牌要落盘，否则重启后客户端连不上");

    let err = handshake(&url, TOKEN).await.expect_err("旧令牌不该还能连");
    assert!(format!("{err:?}").contains("401"), "旧令牌应被拦在 401: {err:?}");
    assert_eq!(handshake(&url, &st.token).await.expect("新令牌应连上"), "onecrt");

    mcp::set_enabled(&state, false).await.expect("关掉");
}

/// 原端口重绑（点「应用」但没改号）走的就是 stop→start，
/// 非等到监听任务真退出再绑才不撞 10048。
#[tokio::test]
async fn reapplying_the_same_port_rebinds() {
    let (state, url) = running("sameport").await;
    let port = mcp::status(&state).await.port;
    let st = mcp::set_port(&state, port).await.expect("同端口重绑");
    assert!(st.running, "重绑后应在监听，error={:?}", st.error);
    assert_eq!(st.url, url);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let st = mcp::status(&state).await;
    assert!(st.running, "200ms 后仍应监听，error={:?}", st.error);
    assert_eq!(handshake(&url, TOKEN).await.expect("重绑后仍能握手"), "onecrt");
    mcp::set_enabled(&state, false).await.expect("关掉");
}

/// 应用侧跑在 tauri 的多线程 runtime 上，`#[tokio::test]` 默认是单线程：
/// 单线程绿、多线程红这类差别只有显式指定 flavor 才测得到。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_flow_on_multithread_runtime() {
    let (state, url) = running("mt").await;
    tools_flow(&url).await;
    mcp::set_enabled(&state, false).await.expect("关掉");
}
