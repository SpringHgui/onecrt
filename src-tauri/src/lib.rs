mod files;
mod import;
pub mod mcp;
mod sessions;
pub mod stats;
pub mod terminal;

use sessions::{CommandTree, Node, SessionTree};
use tauri::{Manager, State};

#[tauri::command]
fn load_sessions() -> Result<SessionTree, String> {
    sessions::load_tree()
}

#[tauri::command]
fn save_sessions(tree: SessionTree) -> Result<(), String> {
    sessions::save_tree(&tree)
}

#[tauri::command]
fn load_commands() -> Result<CommandTree, String> {
    sessions::load_commands()
}

#[tauri::command]
fn save_commands(tree: CommandTree) -> Result<(), String> {
    sessions::save_commands(&tree)
}

/// 弹出文件对话框选择 SecureCRT 导出的 XML，解析出会话节点返回给前端。
/// 用户取消选择时返回 None（前端据此不做任何改动）。
#[tauri::command]
fn import_securecrt() -> Result<Option<Vec<Node>>, String> {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("SecureCRT 配置", &["xml"])
        .pick_file()
    else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(&path).map_err(|e| format!("读取文件失败: {e}"))?;
    import::parse_securecrt(&text).map(Some)
}

/// 同上，选 WinSCP 的配置存储（WinSCP.ini）。
#[tauri::command]
fn import_winscp() -> Result<Option<Vec<Node>>, String> {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("WinSCP 配置", &["ini"])
        .pick_file()
    else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(&path).map_err(|e| format!("读取文件失败: {e}"))?;
    import::parse_winscp(&text).map(Some)
}

/// 关于页要显示的运行环境与数据目录。目录只拼路径不创建——打开弹窗不该有副作用
/// （sessions::store_path() 会 create_dir_all，这里不能复用）。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    os: &'static str,
    arch: &'static str,
    data_dir: String,
}

#[tauri::command]
fn app_info() -> AppInfo {
    let data_dir = dirs::config_dir()
        .map(|p| p.join("shellx").display().to_string())
        .unwrap_or_default();
    AppInfo { os: std::env::consts::OS, arch: std::env::consts::ARCH, data_dir }
}

#[tauri::command]
async fn terminal_open(
    state: State<'_, terminal::Shared>,
    args: terminal::OpenArgs,
    on_event: tauri::ipc::Channel<terminal::OutEvent>,
) -> Result<(), String> {
    terminal::open(&state, args, terminal::channel_sink(on_event)).await
}

#[tauri::command]
async fn terminal_send(
    state: State<'_, terminal::Shared>,
    id: String,
    data: String,
) -> Result<(), String> {
    terminal::send_input(&state, id, data).await
}

#[tauri::command]
async fn terminal_resize(
    state: State<'_, terminal::Shared>,
    id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    terminal::resize(&state, id, cols, rows).await
}

#[tauri::command]
async fn terminal_close(state: State<'_, terminal::Shared>, id: String) -> Result<(), String> {
    terminal::close(state.inner().clone(), id).await;
    Ok(())
}

#[tauri::command]
async fn sftp_open(state: State<'_, files::Shared>, args: files::OpenArgs) -> Result<String, String> {
    files::open(&state, args).await
}

#[tauri::command]
async fn sftp_list(state: State<'_, files::Shared>, id: String, path: String) -> Result<Vec<files::Entry>, String> {
    files::list(&state, id, path).await
}

#[tauri::command]
async fn sftp_mkdir(state: State<'_, files::Shared>, id: String, path: String) -> Result<(), String> {
    files::mkdir(&state, id, path).await
}

#[tauri::command]
async fn sftp_remove(
    state: State<'_, files::Shared>,
    id: String,
    path: String,
    dir: bool,
) -> Result<(), String> {
    files::remove(&state, id, path, dir).await
}

#[tauri::command]
async fn sftp_rename(state: State<'_, files::Shared>, id: String, old: String, new: String) -> Result<(), String> {
    files::rename(&state, id, old, new).await
}

#[tauri::command]
async fn sftp_close(state: State<'_, files::Shared>, id: String) -> Result<(), String> {
    files::close(&state, id).await;
    Ok(())
}

#[tauri::command]
async fn sftp_download(
    state: State<'_, files::Shared>,
    id: String,
    remote: String,
    local_dir: String,
    on_event: tauri::ipc::Channel<files::Progress>,
) -> Result<(), String> {
    files::download(&state, id, remote, local_dir, on_event).await
}

#[tauri::command]
async fn sftp_upload(
    state: State<'_, files::Shared>,
    id: String,
    local: String,
    remote_dir: String,
    on_event: tauri::ipc::Channel<files::Progress>,
) -> Result<(), String> {
    files::upload(&state, id, local, remote_dir, on_event).await
}

#[tauri::command]
async fn stats_start(
    state: State<'_, stats::Shared>,
    args: stats::StartArgs,
    on_event: tauri::ipc::Channel<stats::OutEvent>,
) -> Result<(), String> {
    stats::start(&state, args, on_event).await
}

#[tauri::command]
async fn stats_stop(state: State<'_, stats::Shared>, id: String) -> Result<(), String> {
    stats::stop(&state, &id).await;
    Ok(())
}

#[tauri::command]
async fn mcp_status(state: State<'_, mcp::Shared>) -> Result<mcp::Status, String> {
    Ok(mcp::status(&state).await)
}

#[tauri::command]
async fn mcp_set_enabled(
    state: State<'_, mcp::Shared>,
    enabled: bool,
) -> Result<mcp::Status, String> {
    mcp::set_enabled(&state, enabled).await
}

#[tauri::command]
async fn mcp_set_port(state: State<'_, mcp::Shared>, port: u16) -> Result<mcp::Status, String> {
    mcp::set_port(&state, port).await
}

#[tauri::command]
async fn mcp_rotate_token(state: State<'_, mcp::Shared>) -> Result<mcp::Status, String> {
    mcp::rotate_token(&state).await
}

#[tauri::command]
fn local_home() -> String {
    files::local_home()
}

#[tauri::command]
fn local_shortcuts() -> Vec<files::Entry> {
    files::local_shortcuts()
}

/// 终端的复制/粘贴走原生剪贴板：WebView2 里 navigator.clipboard 读要弹权限框，
/// 而且窗口失焦就失败，所以前端一律调这两个命令。
#[tauri::command]
fn clipboard_read() -> Result<String, String> {
    arboard::Clipboard::new()
        .and_then(|mut c| c.get_text())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn clipboard_write(text: String) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_text(text))
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn local_list(path: String) -> Result<files::LocalListing, String> {
    files::local_list(path).await
}

#[tauri::command]
async fn local_mkdir(dir: String, name: String) -> Result<(), String> {
    files::local_mkdir(dir, name).await
}

#[tauri::command]
async fn local_remove(path: String, dir: bool) -> Result<(), String> {
    files::local_remove(path, dir).await
}

#[tauri::command]
async fn local_rename(path: String, name: String) -> Result<(), String> {
    files::local_rename(path, name).await
}

#[tauri::command]
fn key_fingerprint(path: String) -> Result<String, String> {
    terminal::fingerprint(&path)
}

#[tauri::command]
fn pick_key_file() -> Option<String> {
    rfd::FileDialog::new().pick_file().map(|p| p.to_string_lossy().into_owned())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let term = terminal::new_manager();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // 应用内更新：装完新包要 relaunch 才生效（Linux 的 AppImage 尤其靠这一步）
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(term.clone())
        .manage(files::new_manager())
        .manage(stats::new_manager())
        .manage(mcp::new_manager(term))
        .setup(|app| {
            // 上次开着 MCP 服务就按原端口/token 拉起来，agent 侧的接入配置不用改
            let state = app.state::<mcp::Shared>().inner().clone();
            tauri::async_runtime::spawn(async move { mcp::boot(&state).await });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_sessions,
            save_sessions,
            load_commands,
            save_commands,
            import_securecrt,
            import_winscp,
            app_info,
            terminal_open,
            terminal_send,
            terminal_resize,
            terminal_close,
            sftp_open,
            sftp_list,
            sftp_mkdir,
            sftp_remove,
            sftp_rename,
            sftp_close,
            sftp_download,
            sftp_upload,
            stats_start,
            stats_stop,
            mcp_status,
            mcp_set_enabled,
            mcp_set_port,
            mcp_rotate_token,
            local_home,
            local_shortcuts,
            clipboard_read,
            clipboard_write,
            local_list,
            local_mkdir,
            local_remove,
            local_rename,
            key_fingerprint,
            pick_key_file
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
