//! 工具自己的配置，存在 Tauri 的应用配置目录下的 config.json

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub wsl_distro: String,
    /// 清单目录；None 表示还没让用户确认过
    pub manifest_dir: Option<String>,
    /// 每份清单上次写入内容的哈希，键是 "windows" / "wsl"
    pub manifest_hashes: HashMap<String, String>,
    /// 上次在工具里推送失败的仓库（键见 commands::push_key）→ 失败信息；下次推送成功才清除。
    /// 一个远程配多个推送地址时，一处成功会让 git 认为已推送，靠这里继续标红
    pub push_failures: HashMap<String, String>,
    pub editor: String,
    pub gita_windows: String,
    pub skip_dirs: Vec<String>,
    pub discover_depth: usize,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            wsl_distro: "Ubuntu".into(),
            manifest_dir: None,
            manifest_hashes: HashMap::new(),
            push_failures: HashMap::new(),
            editor: "code".into(),
            gita_windows: "gita".into(),
            skip_dirs: ["node_modules", "build", "Debug", "Release", ".venv"]
                .map(String::from)
                .to_vec(),
            discover_depth: 6,
        }
    }
}

impl AppConfig {
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("创建配置目录失败：{e}"))?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| format!("保存配置失败：{e}"))
    }
}

/// 默认清单目录：OneDrive 下的 gita-gui 文件夹
pub fn default_manifest_dir() -> Option<PathBuf> {
    std::env::var_os("OneDrive").map(|d| PathBuf::from(d).join("gita-gui"))
}
