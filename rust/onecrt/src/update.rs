//! 版本检查与自动更新：查 GitHub 上的最新正式版，下载对应平台的安装包并静默装上。
//!
//! 分发物只有安装包、没有签名清单，所以自动更新只做在能静默安装的平台上（Windows 的
//! per-user Inno 安装包）；其它平台仍然只给下载页。

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

/// 不能用 `/releases/latest`：它按「最近一次公开」挑，补发一个旧版本的 draft 就会顶掉最新版，
/// 界面于是看到的版本比当前还旧。这里拉列表自己取版本号最大的正式版。
const ENDPOINT: &str = "https://api.github.com/repos/SpringHgui/onecrt/releases?per_page=100";
const USER_AGENT: &str = concat!("onecrt/", env!("CARGO_PKG_VERSION"));

pub struct Latest {
    pub version: String,
    pub url: String,
    /// 当前平台安装包的直链；发布页里没有对应资产时为空
    pub asset: Option<String>,
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
pub fn download(latest: &Latest) -> Result<PathBuf, String> {
    let url = latest
        .asset
        .as_deref()
        .ok_or_else(|| "发布页上没有当前平台的安装包".to_string())?;
    let name = url.rsplit('/').next().unwrap_or("onecrt-setup").to_string();
    let dir = std::env::temp_dir().join("onecrt-update");
    std::fs::create_dir_all(&dir).map_err(|e| format!("建临时目录失败：{e}"))?;
    let dest = dir.join(name);

    let mut response = agent()
        .get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| format!("下载失败：{e}"))?;
    let mut file = std::fs::File::create(&dest).map_err(|e| format!("写临时文件失败：{e}"))?;
    // 安装包十几 MB，默认 10MB 上限不够
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(256 * 1024 * 1024)
        .reader();
    std::io::copy(&mut reader, &mut file).map_err(|e| format!("下载失败：{e}"))?;
    Ok(dest)
}

/// 拉起安装程序：先等本进程退出（安装包要覆盖正在运行的文件），再静默安装，最后启动新版本。
#[cfg(target_os = "windows")]
pub fn install(setup: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    let exe = install_exe();
    let script = format!(
        "ping -n 3 127.0.0.1 >nul & start \"\" /wait \"{}\" /SP- /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /CLOSEAPPLICATIONS & start \"\" \"{}\"",
        setup.display(),
        exe.display()
    );
    std::process::Command::new("cmd")
        .args(["/c", &script])
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
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .build()
        .into()
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
