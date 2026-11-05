//! 界面偏好：主题、语言、性能面板采样间隔与更新走的网络代理，跟会话数据落在同一个目录。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct UiPrefs {
    /// "dark" | "light"
    pub theme: String,
    /// "zh" | "en"
    pub lang: String,
    pub perf_interval: u32,
    /// 终端字号，范围与旧版缩放一致（9–32px）
    pub font_size: f32,
    /// 更新检查/下载的网络方式："direct" | "system" | "custom"。
    /// 旧配置没有这个字段，默认跟随系统。
    pub proxy_mode: String,
    /// 自定义代理地址；只在 proxy_mode 为 custom 时使用，切走也保留着方便切回来
    pub proxy: String,
}

/// 不走代理，直接连更新服务器。
pub const PROXY_DIRECT: &str = "direct";
/// 跟随系统代理（环境变量，Windows 上还有 Internet 选项）。
pub const PROXY_SYSTEM: &str = "system";
/// 走下面那条自定义代理地址。
pub const PROXY_CUSTOM: &str = "custom";

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            lang: "zh".into(),
            perf_interval: 2,
            font_size: 14.,
            proxy_mode: PROXY_SYSTEM.into(),
            proxy: String::new(),
        }
    }
}

fn path() -> Result<PathBuf, String> {
    let base = dirs::config_dir().ok_or_else(|| "无法获取配置目录".to_string())?;
    let dir = base.join("shellx");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("ui.json"))
}

pub fn load() -> UiPrefs {
    path()
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .map(|prefs: UiPrefs| prefs.normalized())
        .unwrap_or_default()
}

pub fn save(prefs: &UiPrefs) -> Result<(), String> {
    let text = serde_json::to_string_pretty(prefs).map_err(|e| e.to_string())?;
    // 后台线程可能正在读这份配置（比如更新检查取代理），别让它读到写了一半的文件
    crate::daemon::write_atomic(&path()?, &text)
}

impl UiPrefs {
    /// 采样间隔只认面板上的四档，脏数据回落到 2 秒
    fn normalized(mut self) -> Self {
        if !matches!(self.perf_interval, 1 | 2 | 5 | 10) {
            self.perf_interval = 2;
        }
        if self.theme != "light" {
            self.theme = "dark".into();
        }
        if self.lang != "en" {
            self.lang = "zh".into();
        }
        if !self.font_size.is_finite() {
            self.font_size = 14.;
        }
        self.font_size = self.font_size.clamp(9., 32.);
        self.proxy = self.proxy.trim().to_string();
        if !matches!(
            self.proxy_mode.as_str(),
            PROXY_DIRECT | PROXY_SYSTEM | PROXY_CUSTOM
        ) {
            self.proxy_mode = PROXY_SYSTEM.into();
        }
        if self.proxy_mode == PROXY_CUSTOM && self.proxy.is_empty() {
            self.proxy_mode = PROXY_SYSTEM.into();
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 字号默认十四并夹在九到三十二之间() {
        assert_eq!(UiPrefs::default().font_size, 14.);
        let cases = [
            (3., 9.),
            (9., 9.),
            (20., 20.),
            (32., 32.),
            (100., 32.),
            (f32::NAN, 14.),
        ];
        for (input, want) in cases {
            let mut prefs = UiPrefs::default();
            prefs.font_size = input;
            assert_eq!(prefs.normalized().font_size, want, "input {input}");
        }
    }

    #[test]
    fn 旧配置文件缺字号时取默认值() {
        let prefs: UiPrefs =
            serde_json::from_str(r#"{"theme":"light","lang":"en","perfInterval":5}"#).unwrap();
        assert_eq!(prefs.font_size, 14.);
        assert_eq!(prefs.proxy, "");
        assert_eq!(prefs.normalized().proxy_mode, PROXY_SYSTEM);
    }

    #[test]
    fn 旧配置没有代理方式时默认系统() {
        let with_proxy: UiPrefs =
            serde_json::from_str(r#"{"proxy":"http://127.0.0.1:7890"}"#).unwrap();
        assert_eq!(with_proxy.normalized().proxy_mode, PROXY_SYSTEM);
        let without_proxy: UiPrefs = serde_json::from_str(r#"{"proxy":""}"#).unwrap();
        assert_eq!(without_proxy.normalized().proxy_mode, PROXY_SYSTEM);
    }

    #[test]
    fn 代理方式只认三种取值() {
        let normalized = |mode: &str, proxy: &str| {
            let mut prefs = UiPrefs::default();
            prefs.proxy_mode = mode.into();
            prefs.proxy = proxy.into();
            prefs.normalized().proxy_mode
        };
        assert_eq!(normalized("bogus", ""), PROXY_SYSTEM);
        assert_eq!(normalized(PROXY_CUSTOM, ""), PROXY_SYSTEM);
        assert_eq!(normalized(PROXY_DIRECT, ""), PROXY_DIRECT);
        assert_eq!(
            normalized(PROXY_CUSTOM, "http://127.0.0.1:7890"),
            PROXY_CUSTOM
        );
    }

    #[test]
    fn 代理地址去掉首尾空白() {
        let mut prefs = UiPrefs::default();
        prefs.proxy = "  http://127.0.0.1:7890  ".into();
        assert_eq!(prefs.normalized().proxy, "http://127.0.0.1:7890");
    }
}
