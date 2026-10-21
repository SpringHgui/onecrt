mod files;
mod sessions;
pub mod terminal;

use sessions::{CommandTree, SessionTree};
use tauri::State;

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

#[tauri::command]
async fn terminal_open(
    state: State<'_, terminal::Shared>,
    args: terminal::OpenArgs,
    on_event: tauri::ipc::Channel<terminal::OutEvent>,
) -> Result<(), String> {
    terminal::open(&state, args, on_event).await
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(terminal::new_manager())
        .manage(files::new_manager())
        .invoke_handler(tauri::generate_handler![
            load_sessions,
            save_sessions,
            load_commands,
            save_commands,
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
            local_home,
            local_shortcuts,
            clipboard_read,
            clipboard_write,
            local_list,
            local_mkdir,
            local_remove,
            local_rename
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
