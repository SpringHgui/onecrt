//! 界面偏好：主题、语言与性能面板采样间隔，跟会话数据落在同一个目录。

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
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            lang: "zh".into(),
            perf_interval: 2,
            font_size: 14.,
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
    std::fs::write(path()?, text).map_err(|e| e.to_string())
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
    }
}
