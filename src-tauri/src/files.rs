//! 文件面板的后端：本地目录浏览 + 一条独立的 SFTP 连接。
//!
//! SFTP 不复用终端的 SSH 连接：russh 的 Handle 在 terminal.rs 里被 split 后消费掉了，
//! 为共享一条连接去改造终端热路径不值当，而且终端断开时文件面板应当还能继续传文件。

use crate::terminal;
use anyhow::Context;
use russh::client;
use russh_sftp::client::SftpSession;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::ipc::Channel;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

const CHUNK: usize = 64 * 1024;

/// 本地与远端列表共用同一形状，前端一套行渲染就够
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub dir: bool,
    /// 目录与取不到大小的条目为 None，前端显示成「—」
    pub size: Option<u64>,
    pub mtime: Option<i64>,
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
}

#[derive(Serialize, Clone)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Progress {
    Tick { name: String, done: u64, total: u64 },
    Done { name: String },
}

struct Conn {
    /// 只为了保活：drop 掉它 SSH 连接就断了
    _handle: client::Handle<terminal::ShellHandler>,
    sftp: Arc<SftpSession>,
}

pub struct FileManager {
    conns: HashMap<String, Conn>,
}

pub type Shared = Arc<Mutex<FileManager>>;

pub fn new_manager() -> Shared {
    Arc::new(Mutex::new(FileManager {
        conns: HashMap::new(),
    }))
}

fn s(err: impl std::fmt::Display) -> String {
    err.to_string()
}

async fn session(mgr: &Shared, id: &str) -> Result<Arc<SftpSession>, String> {
    mgr.lock()
        .await
        .conns
        .get(id)
        .map(|c| c.sftp.clone())
        .ok_or_else(|| "SFTP 会话尚未连接".to_string())
}

/// SFTP 全程用 `/`，不能套本地分隔符
fn join(base: &str, name: &str) -> String {
    if base.ends_with('/') {
        format!("{base}{name}")
    } else {
        format!("{base}/{name}")
    }
}

fn base_name(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn sort_entries(v: &mut [Entry]) {
    v.sort_by(|a, b| {
        b.dir
            .cmp(&a.dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

// ---------- 远端 ----------

pub async fn open(mgr: &Shared, args: OpenArgs) -> Result<String, String> {
    let addr = format!("{}:{}", args.host, args.port);
    let key = terminal::load_key(&args.key_path);
    let mut handle = client::connect(
        Arc::new(client::Config::default()),
        addr.as_str(),
        terminal::ShellHandler,
    )
    .await
    .with_context(|| format!("无法连接到 {addr}"))
    .map_err(terminal::format_err)?;
    terminal::authenticate(&mut handle, &args.username, &args.password, key)
        .await
        .map_err(terminal::format_err)?;

    let channel = handle
        .channel_open_session()
        .await
        .map_err(|e| format!("打开通道失败: {e}"))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| format!("服务器未开启 SFTP 子系统: {e}"))?;
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|e| format!("SFTP 初始化失败: {e}"))?;
    let home = sftp.canonicalize(".").await.unwrap_or_else(|_| "/".into());

    let old = mgr.lock().await.conns.insert(
        args.id,
        Conn {
            _handle: handle,
            sftp: Arc::new(sftp),
        },
    );
    // 同 id 重开时旧连接让后台任务尽快收尾，不阻塞本次返回
    if let Some(old) = old {
        tokio::spawn(async move {
            let _ = old.sftp.close().await;
        });
    }
    Ok(home)
}

pub async fn list(mgr: &Shared, id: String, path: String) -> Result<Vec<Entry>, String> {
    let sftp = session(mgr, &id).await?;
    let mut rd = sftp.read_dir(&path).await.map_err(s)?;
    let mut out = Vec::new();
    while let Some(e) = rd.next() {
        let meta = e.metadata();
        out.push(Entry {
            name: e.file_name(),
            path: e.path(),
            dir: e.file_type().is_dir(),
            size: meta.size,
            mtime: meta.mtime.map(|m| m as i64),
        });
    }
    sort_entries(&mut out);
    Ok(out)
}

pub async fn mkdir(mgr: &Shared, id: String, path: String) -> Result<(), String> {
    session(mgr, &id).await?.create_dir(path).await.map_err(s)
}

pub async fn remove(mgr: &Shared, id: String, path: String, dir: bool) -> Result<(), String> {
    let sftp = session(mgr, &id).await?;
    if dir {
        sftp.remove_dir(path).await
    } else {
        sftp.remove_file(path).await
    }
    .map_err(s)
}

pub async fn rename(mgr: &Shared, id: String, old: String, new: String) -> Result<(), String> {
    session(mgr, &id).await?.rename(old, new).await.map_err(s)
}

pub async fn close(mgr: &Shared, id: String) {
    if let Some(c) = mgr.lock().await.conns.remove(&id) {
        let _ = c.sftp.close().await;
    }
}

pub async fn download(
    mgr: &Shared,
    id: String,
    remote: String,
    local_dir: String,
    sink: Channel<Progress>,
) -> Result<(), String> {
    let sftp = session(mgr, &id).await?;
    let name = base_name(&remote);
    let dest = localized(&local_dir, &name);
    let total = sftp.metadata(&remote).await.ok().and_then(|m| m.size).unwrap_or(0);

    let mut src = sftp.open(&remote).await.map_err(s)?;
    let mut dst = tokio::fs::File::create(&dest)
        .await
        .map_err(|e| format!("无法写入 {}: {e}", dest.display()))?;
    let mut buf = vec![0u8; CHUNK];
    let mut done = 0u64;
    loop {
        let n = src.read(&mut buf).await.map_err(s)?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n]).await.map_err(|e| format!("写入失败: {e}"))?;
        done += n as u64;
        let _ = sink.send(Progress::Tick {
            name: name.clone(),
            done,
            total,
        });
    }
    dst.flush().await.map_err(|e| format!("写入失败: {e}"))?;
    drop(dst);
    let _ = src.close().await;
    let _ = sink.send(Progress::Done { name });
    Ok(())
}

pub async fn upload(
    mgr: &Shared,
    id: String,
    local: String,
    remote_dir: String,
    sink: Channel<Progress>,
) -> Result<(), String> {
    let sftp = session(mgr, &id).await?;
    let src_path = PathBuf::from(&local);
    let name = src_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| "只能传输文件".to_string())?;
    let total = tokio::fs::metadata(&src_path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);

    let mut src = tokio::fs::File::open(&src_path)
        .await
        .map_err(|e| format!("无法读取 {local}: {e}"))?;
    let mut dst = sftp.create(join(&remote_dir, &name)).await.map_err(s)?;
    let mut buf = vec![0u8; CHUNK];
    let mut done = 0u64;
    loop {
        let n = src.read(&mut buf).await.map_err(s)?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n]).await.map_err(s)?;
        done += n as u64;
        let _ = sink.send(Progress::Tick {
            name: name.clone(),
            done,
            total,
        });
    }
    // SFTP 的 File::close 负责把缓冲落盘，不能只 drop
    dst.close().await.map_err(s)?;
    let _ = sink.send(Progress::Done { name });
    Ok(())
}

// ---------- 本地 ----------

fn localized(dir: &str, name: &str) -> PathBuf {
    let d = Path::new(dir);
    if d.as_os_str().is_empty() {
        PathBuf::from(name)
    } else {
        d.join(name)
    }
}

/// 本地侧的上级目录要按平台规则算（盘符根、UNC），所以随列表一起回给前端。
/// 空串表示已经在根列表（盘符）这一层，没有上级。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LocalListing {
    pub parent: String,
    pub entries: Vec<Entry>,
}

/// 本地侧的根：Windows 上是盘符列表，其它系统是单个 /
fn roots() -> Vec<Entry> {
    if cfg!(windows) {
        (b'A'..=b'Z')
            .map(|c| format!("{}:\\", c as char))
            .filter(|p| Path::new(p).exists())
            .map(|p| Entry {
                name: p.clone(),
                path: p,
                dir: true,
                size: None,
                mtime: None,
            })
            .collect()
    } else {
        vec![Entry {
            name: "/".into(),
            path: "/".into(),
            dir: true,
            size: None,
            mtime: None,
        }]
    }
}

pub fn local_home() -> String {
    dirs::home_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 快速访问：常用文件夹 + 全部盘符。走 dirs 的已知文件夹解析，所以 OneDrive 重定向过的
/// 桌面/文档也能拿到真实路径；取不到或已不存在的直接跳过。
pub fn local_shortcuts() -> Vec<Entry> {
    let known: [(&str, fn() -> Option<PathBuf>); 6] = [
        ("桌面", dirs::desktop_dir),
        ("下载", dirs::download_dir),
        ("文档", dirs::document_dir),
        ("图片", dirs::picture_dir),
        ("音乐", dirs::audio_dir),
        ("视频", dirs::video_dir),
    ];
    let mut out: Vec<Entry> = known
        .into_iter()
        .filter_map(|(name, get)| {
            let p = get()?;
            p.is_dir().then(|| Entry {
                name: name.into(),
                path: p.to_string_lossy().into_owned(),
                dir: true,
                size: None,
                mtime: None,
            })
        })
        .collect();
    out.extend(roots());
    out
}

/// 空串表示「根列表」（盘符），此时没有上级
fn local_parent(path: &str) -> String {
    if path.trim().is_empty() {
        return String::new();
    }
    match Path::new(path).parent() {
        Some(pp) if !pp.as_os_str().is_empty() => pp.to_string_lossy().into_owned(),
        _ => String::new(),
    }
}

fn local_list_one(path: String) -> Result<Vec<Entry>, String> {
    if path.trim().is_empty() {
        return Ok(roots());
    }
    let mut out = Vec::new();
    // 单个条目读不到元数据（权限、失效的符号链接）不该让整份列表失败
    let rd = std::fs::read_dir(&path).map_err(|e| format!("{}: {e}", path))?;
    for e in rd.flatten() {
        let meta = match e.metadata() {
            Ok(m) => Some(m),
            Err(_) => None,
        };
        let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        out.push(Entry {
            name: e.file_name().to_string_lossy().into_owned(),
            path: e.path().to_string_lossy().into_owned(),
            dir: is_dir,
            size: meta.as_ref().filter(|_| !is_dir).map(|m| m.len()),
            mtime: meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64),
        });
    }
    sort_entries(&mut out);
    Ok(out)
}

pub async fn local_list(path: String) -> Result<LocalListing, String> {
    let for_parent = path.clone();
    let entries = tokio::task::spawn_blocking(move || local_list_one(path))
        .await
        .map_err(s)??;
    Ok(LocalListing {
        parent: local_parent(&for_parent),
        entries,
    })
}

pub async fn local_mkdir(dir: String, name: String) -> Result<(), String> {
    let p = localized(&dir, &name);
    tokio::task::spawn_blocking(move || std::fs::create_dir(p))
        .await
        .map_err(s)?
        .map_err(|e| format!("建目录失败: {e}"))
}

pub async fn local_remove(path: String, dir: bool) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        if dir {
            std::fs::remove_dir(&path)
        } else {
            std::fs::remove_file(&path)
        }
    })
    .await
    .map_err(s)?
    .map_err(|e| format!("删除失败: {e}"))
}

pub async fn local_rename(path: String, name: String) -> Result<(), String> {
    let from = PathBuf::from(&path);
    let parent = from
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let to = localized(&parent, &name);
    tokio::task::spawn_blocking(move || std::fs::rename(from, to))
        .await
        .map_err(s)?
        .map_err(|e| format!("重命名失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_paths_always_use_slash() {
        assert_eq!(join("/srv", "app.jar"), "/srv/app.jar");
        assert_eq!(join("/", "app.jar"), "/app.jar");
        assert_eq!(base_name("/srv/app.jar"), "app.jar");
    }

    #[test]
    fn local_parent_stops_at_drive_root() {
        // 盘符根的 parent 会指回自己；空串是前端置灰「上一级」的信号
        assert_eq!(local_parent(""), "");
        assert_eq!(local_parent("C:\\Users\\Sprin"), "C:\\Users");
        assert_eq!(local_parent("C:\\Users"), "C:\\");
        assert_eq!(local_parent("C:\\"), "");
    }

    #[test]
    fn roots_are_navigable() {
        let r = roots();
        assert!(!r.is_empty());
        assert!(r.iter().all(|e| e.dir && e.size.is_none()));
    }

    #[test]
    fn quick_access_entries_are_usable() {
        let s = local_shortcuts();
        // 常用文件夹按机器/账户走，断言不了具体几个；能断言的是每条都得是个能点进去的目录
        assert!(!s.is_empty());
        assert!(s.iter().all(|e| e.dir && !e.name.is_empty()));
        assert!(s.iter().all(|e| Path::new(&e.path).is_dir()));
    }

    #[tokio::test]
    async fn local_listing_puts_dirs_first() {
        let root = std::env::temp_dir().join(format!("shellx-fs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("zdir")).unwrap();
        std::fs::write(root.join("a.txt"), b"hello").unwrap();

        let listing = local_list(root.to_string_lossy().into_owned()).await.unwrap();
        let names: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["zdir", "a.txt"]);
        assert!(listing.entries[0].size.is_none(), "目录不报大小");
        let file = listing.entries.iter().find(|e| e.name == "a.txt").unwrap();
        assert_eq!((file.dir, file.size, file.mtime.is_some()), (false, Some(5), true));
        assert!(!listing.parent.is_empty());

        std::fs::remove_dir_all(&root).unwrap();
    }
}
