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
    /// 可视化配置声明：插件页按它渲染表单，值存插件 KV（cfg: 前缀）
    #[serde(default)]
    pub config_schema: Vec<ConfigField>,
}

/// 一个可视化配置项。
/// ty: string | textarea | password | number | slider | bool | select |
///     radio | multi_select | tags | path | file | color | date | time
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigField {
    /// 配置键：插件侧 bot.config(key) 读取
    pub key: String,
    /// 表单显示名
    pub label: String,
    /// 组件类型（缺省 string，见上方清单）
    #[serde(default = "cfg_ty_default")]
    pub ty: String,
    /// 默认值（类型随 ty：bool→bool，number/slider→数字，multi_select/tags→数组，其余→字符串）
    #[serde(default)]
    pub default: serde_json::Value,
    /// select / radio / multi_select 的候选值
    #[serde(default)]
    pub options: Vec<String>,
    /// 说明文字（表单 hint）
    #[serde(default)]
    pub desc: String,
    /// 敏感字段：UI 掩码输入（API key 之类；ty=password 同效）
    #[serde(default)]
    pub password: bool,
    /// number/slider 的最小值
    #[serde(default)]
    pub min: Option<f64>,
    /// number/slider 的最大值
    #[serde(default)]
    pub max: Option<f64>,
    /// number/slider 的步长
    #[serde(default)]
    pub step: Option<f64>,
    /// 输入占位提示
    #[serde(default)]
    pub placeholder: String,
    /// 必填校验（保存时拦空值）
    #[serde(default)]
    pub required: bool,
}

fn cfg_ty_default() -> String {
    "string".into()
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
