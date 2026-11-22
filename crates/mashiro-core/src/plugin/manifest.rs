use serde::{Deserialize, Serialize};
use std::path::Path;

/// 插件运行时：每个 runtime 对应一个宿主进程（node host.js / python host.py）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    Node,
    Python,
}

/// plugin.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub runtime: Runtime,
    pub entry: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    /// 能力声明：send / kv / timer / group_admin / http ...
    #[serde(default)]
    pub permissions: Vec<String>,
}

impl Manifest {
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(dir.join("plugin.json"))?;
        let m: Self = serde_json::from_str(&text)?;
        if m.id.is_empty() {
            anyhow::bail!("plugin.json 缺少 id");
        }
        if m.entry.is_empty() {
            anyhow::bail!("plugin.json 缺少 entry");
        }
        Ok(m)
    }
}
