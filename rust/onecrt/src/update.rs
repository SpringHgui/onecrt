//! 版本检查：只查 GitHub 上最新的 Release，不下载、不替换正在运行的二进制。

use std::time::Duration;

use serde::Deserialize;

const ENDPOINT: &str = "https://api.github.com/repos/SpringHgui/onecrt/releases/latest";

pub struct Latest {
    pub version: String,
    pub url: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
}

/// 有比 `current` 更新的正式版时返回它；出错交给调用方决定怎么提示。
pub fn check(current: &str) -> Result<Option<Latest>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(8)))
        .build()
        .into();
    let mut response = agent
        .get(ENDPOINT)
        .header("User-Agent", concat!("onecrt/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?;
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    let release: Release = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let version = release.tag_name.trim_start_matches(['v', 'V']).to_string();
    if newer(&version, current) {
        Ok(Some(Latest {
            version,
            url: release.html_url,
        }))
    } else {
        Ok(None)
    }
}

/// 只比较点分数字段；预发布后缀（`-beta.1` 等）按正式版对待，GitHub 的
/// latest 端点本身也不会把 prerelease 算进来。
fn newer(latest: &str, current: &str) -> bool {
    fn parts(v: &str) -> Vec<u64> {
        let core = v.split(['-', '+']).next().unwrap_or(v);
        core.split('.').map(|p| p.parse().unwrap_or(0)).collect()
    }
    let (a, b) = (parts(latest), parts(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::newer;

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
}
