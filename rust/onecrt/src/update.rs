//! 版本检查与自动更新：查 GitHub 上的最新正式版，下载对应平台的安装包并静默装上。
//!
//! 分发物只有安装包、没有签名清单，所以自动更新只做在能静默安装的平台上（Windows 的
//! per-user Inno 安装包）；其它平台仍然只给下载页。

use std::cmp::Ordering;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::time::{Duration, Instant};

use serde::Deserialize;

use onecrt_core::settings;

/// 不能用 `/releases/latest`：它按「最近一次公开」挑，补发一个旧版本的 draft 就会顶掉最新版，
/// 界面于是看到的版本比当前还旧。这里拉列表自己取版本号最大的正式版。
const ENDPOINT: &str = "https://api.github.com/repos/SpringHgui/onecrt/releases?per_page=100";
const USER_AGENT: &str = concat!("onecrt/", env!("CARGO_PKG_VERSION"));

#[derive(Clone)]
pub struct Latest {
    pub version: String,
    pub url: String,
    /// 当前平台安装包的直链；发布页里没有对应资产时为空
    pub asset: Option<String>,
}

/// 下载中的一次进度上报。
pub struct DownloadProgress {
    pub downloaded: u64,
    /// 服务端给的总字节数；没给就是 None，此时进度条只能显示已下载量
    pub total: Option<u64>,
    /// 最近一段时间的平均速度，字节/秒
    pub bytes_per_sec: f64,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// 有比 `current` 更新的正式版时返回它；出错交给调用方决定怎么提示。
pub fn check(current: &str) -> Result<Option<Latest>, String> {
    let mut response = agent()
        .get(ENDPOINT)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?;
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    let releases: Vec<Release> = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let Some((version, url, asset)) = newest(&releases) else {
        return Ok(None);
    };
    if newer(&version, current) {
        Ok(Some(Latest {
            version,
            url,
            asset,
        }))
    } else {
        Ok(None)
    }
}

/// 能不能自己装：只有 Windows 有免 UAC 的 per-user 安装包。
pub fn can_auto_install() -> bool {
    cfg!(target_os = "windows") && !asset_suffix().is_empty()
}

/// 下载当前版本的平台安装包到临时目录，返回落地路径。
///
/// 半截文件一直留在临时目录里（`.part` 后缀），中途失败、关掉窗口甚至重启应用后
/// 再来都会带 `Range` 续传；只有整份下完才改名成正式文件名。
pub fn download(
    latest: &Latest,
    cancel: &AtomicBool,
    report: impl Fn(DownloadProgress),
) -> Result<PathBuf, String> {
    let url = latest
        .asset
        .as_deref()
        .ok_or_else(|| "发布页上没有当前平台的安装包".to_string())?;
    let name = url.rsplit('/').next().unwrap_or("onecrt-setup").to_string();
    let dir = std::env::temp_dir().join("onecrt-update");
    std::fs::create_dir_all(&dir).map_err(|e| format!("建临时目录失败：{e}"))?;
    let dest = dir.join(name);
    // 同名安装包之前已经整份下完（可能只是装到一半失败），直接复用
    if std::fs::metadata(&dest)
        .map(|m| m.len() > 0)
        .unwrap_or(false)
    {
        return Ok(dest);
    }
    let partial = dest.with_extension("part");
    let mut offset = std::fs::metadata(&partial).map(|m| m.len()).unwrap_or(0);

    let agent = download_agent();
    let mut response = fetch(&agent, url, offset).map_err(|e| format!("下载失败：{e}"))?;
    let mut status = response.status().as_u16();
    if status == 416 {
        // 服务端说这个起点已经到头了，说明残留的半截文件对不上，只能整份重下
        offset = 0;
        response = fetch(&agent, url, 0).map_err(|e| format!("下载失败：{e}"))?;
        status = response.status().as_u16();
    }
    if status != 200 && status != 206 {
        return Err(format!("下载失败：HTTP {status}"));
    }
    // 200 表示服务端没按 Range 给，只能从头写
    if status == 200 {
        offset = 0;
    }
    let total = if status == 206 {
        content_range_total(&response)
    } else {
        header_u64(&response, "content-length")
    };

    if offset == 0 {
        // append 模式不会截断，重下前先把旧文件清掉
        let _ = std::fs::remove_file(&partial);
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&partial)
        .map_err(|e| format!("写临时文件失败：{e}"))?;

    // 安装包十几 MB，默认 10MB 上限不够
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(256 * 1024 * 1024)
        .reader();
    let mut buf = vec![0u8; 64 * 1024];
    let mut downloaded = offset;
    let mut speed = 0.0;
    let mut window_start = Instant::now();
    let mut window_bytes = downloaded;
    report(DownloadProgress {
        downloaded,
        total,
        bytes_per_sec: speed,
    });
    loop {
        if cancel.load(AtomicOrdering::Relaxed) {
            return Err("下载已取消".to_string());
        }
        let read = reader
            .read(&mut buf)
            .map_err(|e| format!("下载失败：{e}"))?;
        if read == 0 {
            break;
        }
        file.write_all(&buf[..read])
            .map_err(|e| format!("写临时文件失败：{e}"))?;
        downloaded += read as u64;
        let elapsed = window_start.elapsed();
        if elapsed >= Duration::from_millis(500) {
            let instant = (downloaded - window_bytes) as f64 / elapsed.as_secs_f64();
            // 半秒窗口的瞬时值抖得厉害，拿旧值压一压
            speed = if speed == 0.0 {
                instant
            } else {
                speed * 0.5 + instant * 0.5
            };
            report(DownloadProgress {
                downloaded,
                total,
                bytes_per_sec: speed,
            });
            window_start = Instant::now();
            window_bytes = downloaded;
        }
    }
    let _ = file.flush();
    drop(file);

    // 少给字节就别拿去装：续传接错位置时这是唯一能发现的判据
    if let Some(total) = total {
        if downloaded != total {
            return Err(format!("下载不完整：{downloaded} / {total} 字节"));
        }
    }

    if dest.exists() {
        let _ = std::fs::remove_file(&dest);
    }
    std::fs::rename(&partial, &dest).map_err(|e| format!("保存安装包失败：{e}"))?;
    report(DownloadProgress {
        downloaded,
        total,
        bytes_per_sec: speed,
    });
    Ok(dest)
}

/// 拿响应头而不是让 ureq 把 4xx 变成错误：416 要靠状态码判断续传起点是否失效。
fn fetch(
    agent: &ureq::Agent,
    url: &str,
    offset: u64,
) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
    let mut request = agent.get(url).header("User-Agent", USER_AGENT);
    if offset > 0 {
        request = request.header("Range", format!("bytes={offset}-"));
    }
    request.config().http_status_as_error(false).build().call()
}

fn header_u64(response: &ureq::http::Response<ureq::Body>, name: &str) -> Option<u64> {
    response
        .headers()
        .get(name)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// `Content-Range: bytes 100-999/1000` 里的总长度。
fn content_range_total(response: &ureq::http::Response<ureq::Body>) -> Option<u64> {
    let value = response.headers().get("content-range")?.to_str().ok()?;
    parse_content_range_total(value)
}

fn parse_content_range_total(value: &str) -> Option<u64> {
    value.rsplit('/').next()?.trim().parse().ok()
}

/// 拉起安装程序：先等本进程退出（安装包要覆盖正在运行的文件），再静默安装，最后启动新版本。
#[cfg(target_os = "windows")]
pub fn install(setup: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    // 整段命令不能直接塞进 `cmd /c`：Rust 会把内嵌引号转义成 `\"`，cmd 不认这种转义，
    // start 报“文件名、目录名或卷标语法不正确”。另外脚本里的中文路径要靠 chcp 65001
    // 才能按 UTF-8 解出来。
    let script = format!(
        "@echo off\r\nchcp 65001 >nul\r\nping -n 3 127.0.0.1 >nul\r\nstart \"\" /wait \"{}\" /SP- /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /CLOSEAPPLICATIONS\r\nstart \"\" \"{}\"\r\n",
        setup.display(),
        install_exe().display()
    );
    let bat = setup.with_file_name("onecrt-update.cmd");
    std::fs::write(&bat, script.as_bytes()).map_err(|e| format!("写更新脚本失败：{e}"))?;
    std::process::Command::new("cmd")
        .args(["/c", &bat.to_string_lossy()])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .spawn()
        .map_err(|e| format!("启动安装程序失败：{e}"))?;
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn install(_setup: &Path) -> Result<(), String> {
    Err("自动安装目前只做在 Windows 上".to_string())
}

fn agent() -> ureq::Agent {
    build_agent(Some(Duration::from_secs(120)))
}

/// 下载不设总时长：十几 MB 在慢代理下可能要跑几分钟，总时长会把正常下载拦腰截断。
/// 改成按“多久没收到数据”判挂，连接和单次读取各留上限。
fn download_agent() -> ureq::Agent {
    build_agent(None)
}

fn build_agent(global: Option<Duration>) -> ureq::Agent {
    let mut config = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_per_read(Some(Duration::from_secs(30)))
        .timeout_global(global);
    let prefs = settings::load();
    match proxy_choice(&prefs.proxy_mode, &prefs.proxy) {
        ProxyChoice::Direct => config = config.proxy(None),
        // 不进这个分支就沿用 ureq 默认的取法（环境变量，Windows 上还有 Internet 选项）
        ProxyChoice::System => {}
        ProxyChoice::Custom(proxy) => config = config.proxy(Some(proxy)),
    }
    config.build().into()
}

/// 设置里的三种方式落到 ureq 代理配置上。自定义地址写坏了就退回系统，跟旧版一致。
fn proxy_choice(mode: &str, url: &str) -> ProxyChoice {
    match mode {
        settings::PROXY_DIRECT => ProxyChoice::Direct,
        settings::PROXY_CUSTOM => match proxy_from(url) {
            Some(proxy) => ProxyChoice::Custom(proxy),
            None => ProxyChoice::System,
        },
        _ => ProxyChoice::System,
    }
}

enum ProxyChoice {
    Direct,
    System,
    Custom(ureq::Proxy),
}

fn proxy_from(text: &str) -> Option<ureq::Proxy> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    ureq::Proxy::new(text).ok()
}

/// 保存前先过一遍解析，别让一条写错的地址把更新请求全弄失败。空串表示跟随系统。
pub fn proxy_is_valid(text: &str) -> bool {
    let text = text.trim();
    text.is_empty() || ureq::Proxy::new(text).is_ok()
}

/// 当前平台安装包在发布页里的文件名后缀。
const fn asset_suffix() -> &'static str {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "-windows-x64-setup.exe"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "-linux-x64.deb"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "-macos-arm64.dmg"
    } else {
        ""
    }
}

/// 装完要启动的 exe：优先用卸载项里记的安装目录（用户装到别处也认），否则用 per-user 默认目录。
#[cfg(target_os = "windows")]
fn install_exe() -> PathBuf {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    let subkey = wide(
        r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{43AE8C70-660C-4DCB-A2DF-90E72DA9580F}_is1",
    );
    let value = wide("InstallLocation");
    let mut buf = [0u16; 520];
    let mut size = (buf.len() * 2) as u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr() as *mut _,
            &mut size,
        ) == ERROR_SUCCESS
    };
    if ok {
        let len = (size as usize / 2).saturating_sub(1);
        let dir = String::from_utf16_lossy(&buf[..len]);
        let dir = dir.trim_end_matches('\\');
        if !dir.is_empty() {
            return PathBuf::from(dir).join("onecrt.exe");
        }
    }
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join("Programs")
        .join("OneCRT")
        .join("onecrt.exe")
}

/// 列表里版本号最大的正式版；tag 不是 `v<数字…>` 的（预发布、杂 tag）一律跳过。
fn newest(releases: &[Release]) -> Option<(String, String, Option<String>)> {
    let (version, url) = releases
        .iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter_map(|r| {
            let tag = r.tag_name.trim_start_matches(['v', 'V']);
            tag.starts_with(|c: char| c.is_ascii_digit())
                .then(|| (tag.to_string(), r.html_url.clone()))
        })
        .max_by(|a, b| compare(&a.0, &b.0))?;
    let wanted = format!("onecrt-{version}{}", asset_suffix());
    let asset = releases
        .iter()
        .filter(|r| r.tag_name.trim_start_matches(['v', 'V']) == version)
        .flat_map(|r| r.assets.iter())
        .find(|a| a.name == wanted)
        .map(|a| a.browser_download_url.clone());
    Some((version, url, asset))
}

/// 只比较点分数字段；预发布后缀（`-beta.1` 等）按正式版对待，GitHub 的
/// latest 端点本身也不会把 prerelease 算进来。
fn newer(latest: &str, current: &str) -> bool {
    compare(latest, current) == Ordering::Greater
}

fn compare(a: &str, b: &str) -> Ordering {
    fn parts(v: &str) -> Vec<u64> {
        let core = v.split(['-', '+']).next().unwrap_or(v);
        core.split('.').map(|p| p.parse().unwrap_or(0)).collect()
    }
    let (a, b) = (parts(a), parts(b));
    for i in 0..a.len().max(b.len()) {
        match a
            .get(i)
            .copied()
            .unwrap_or(0)
            .cmp(&b.get(i).copied().unwrap_or(0))
        {
            Ordering::Equal => {}
            other => return other,
        }
    }
    Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::{newer, newest, Asset, Release};

    fn release(tag: &str, draft: bool, prerelease: bool, assets: &[&str]) -> Release {
        Release {
            tag_name: tag.to_string(),
            html_url: format!("https://example.invalid/{tag}"),
            draft,
            prerelease,
            assets: assets
                .iter()
                .map(|name| Asset {
                    name: name.to_string(),
                    browser_download_url: format!("https://example.invalid/{name}"),
                })
                .collect(),
        }
    }

    #[test]
    fn compares_numeric_fields() {
        assert!(newer("0.3.1", "0.3.0"));
        assert!(newer("0.4.0", "0.3.9"));
        assert!(newer("1.0.0", "0.9.9"));
        assert!(!newer("0.3.0", "0.3.0"));
        assert!(!newer("0.2.1", "0.3.0"));
        assert!(!newer("0.3.0-beta.1", "0.3.0"));
        assert!(newer("0.3", "0.2.7"));
    }

    #[test]
    fn picks_highest_version_not_most_recently_published() {
        // 补发旧版本的 draft 会排在列表最前，但它不是最高版本
        let list = [
            release("v0.3.0", false, false, &[]),
            release("v0.4.0", false, false, &[]),
            release("v0.3.2", false, false, &[]),
        ];
        assert_eq!(newest(&list).unwrap().0, "0.4.0");
    }

    #[test]
    fn skips_drafts_prereleases_and_junk_tags() {
        let list = [
            release("v0.5.0", true, false, &[]),
            release("v0.6.0-beta.1", false, true, &[]),
            release("nightly", false, false, &[]),
            release("v0.4.0", false, false, &[]),
        ];
        assert_eq!(newest(&list).unwrap().0, "0.4.0");
        assert!(newest(&[]).is_none());
    }

    #[test]
    fn reads_total_length_from_content_range() {
        assert_eq!(
            super::parse_content_range_total("bytes 1000-1999/2000"),
            Some(2000)
        );
        assert_eq!(super::parse_content_range_total("bytes 0-0/123"), Some(123));
        assert_eq!(super::parse_content_range_total("bytes 0-9/*"), None);
        assert_eq!(super::parse_content_range_total("garbage"), None);
    }

    #[test]
    fn validates_proxy_urls() {
        assert!(super::proxy_is_valid(""));
        assert!(super::proxy_is_valid("   "));
        assert!(super::proxy_is_valid("http://127.0.0.1:7890"));
        // 不带协议的 host:port 也认（ureq 默认按 http 处理）
        assert!(super::proxy_is_valid("127.0.0.1:7890"));
        assert!(super::proxy_is_valid("socks5://127.0.0.1:1080"));
        assert!(!super::proxy_is_valid("http://"));
        assert!(super::proxy_from("  ").is_none());
    }

    #[test]
    fn proxy_choice_follows_mode() {
        use super::{proxy_choice, ProxyChoice};
        use onecrt_core::settings::{PROXY_CUSTOM, PROXY_DIRECT, PROXY_SYSTEM};

        let url = "http://127.0.0.1:7890";
        assert!(matches!(
            proxy_choice(PROXY_DIRECT, url),
            ProxyChoice::Direct
        ));
        assert!(matches!(
            proxy_choice(PROXY_SYSTEM, url),
            ProxyChoice::System
        ));
        assert!(matches!(
            proxy_choice(PROXY_CUSTOM, url),
            ProxyChoice::Custom(_)
        ));
        // 自定义方式却没有可用地址时退回系统，别直接断网
        assert!(matches!(
            proxy_choice(PROXY_CUSTOM, "  "),
            ProxyChoice::System
        ));
        assert!(matches!(proxy_choice("bogus", url), ProxyChoice::System));
    }

    #[test]
    fn picks_this_platforms_installer_from_the_release() {
        let list = [release(
            "v0.4.1",
            false,
            false,
            &[
                "onecrt-0.4.1-linux-x64.deb",
                "onecrt-0.4.1-windows-x64-setup.exe",
                "onecrt-0.4.1-macos-arm64.dmg",
                "SHA256SUMS.txt",
            ],
        )];
        let (_, _, asset) = newest(&list).unwrap();
        let asset = asset.expect("该版本应带当前平台的安装包");
        assert!(asset.ends_with(super::asset_suffix()), "{asset}");
    }
}
