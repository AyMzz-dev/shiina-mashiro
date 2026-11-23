use std::path::PathBuf;
use std::sync::Arc;

use dashmap::DashMap;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::storage::db::Database;

use super::host::{HostApiCall, PluginHost};
use super::manifest::{Manifest, Runtime};

pub struct LoadedPlugin {
    pub manifest: Manifest,
    pub dir: PathBuf,
    /// 插件加载后注册的 handler 列表（"cmd:签到" / "event:xxx"），来自 describe。
    pub handlers: Vec<String>,
    /// 框架下发的身份凭据：load 时经宿主注入，API 帧必须带回才算本人
    pub token: String,
}

/// 插件管理器：每种 runtime 一个宿主进程，插件按 id 归口到宿主。
/// api_tx：插件反向调框架 API 的统一出口（交给 sdk_api 消费）。
pub struct PluginManager {
    hosts: DashMap<Runtime, Arc<PluginHost>>,
    plugins: DashMap<String, LoadedPlugin>,
    api_tx: mpsc::UnboundedSender<HostApiCall>,
    /// 宿主 spawn 串行锁：host_for 的 check-then-act 竞态用它消除
    host_lock: tokio::sync::Mutex<()>,
    db: Arc<Database>,
}

impl PluginManager {
    pub fn new(api_tx: mpsc::UnboundedSender<HostApiCall>, db: Arc<Database>) -> Self {
        Self {
            hosts: DashMap::new(),
            plugins: DashMap::new(),
            api_tx,
            host_lock: tokio::sync::Mutex::new(()),
            db,
        }
    }

    /// 从目录安装/加载插件：读 manifest → 拉起对应宿主 → load_plugin → describe。
    /// 返回 (manifest, 注册的 handler 列表)。
    pub async fn load(&self, dir: PathBuf) -> anyhow::Result<(Manifest, Vec<String>)> {
        let m = Manifest::load(&dir)?;
        let host = self.host_for(m.runtime).await?;
        // 每个插件发一个一次性身份凭据：宿主转发 API 调用时盖章，
        // 插件裸写 stdout 伪造的帧没有 token → 框架按匿名处理
        let token = hex::encode(rand::random::<[u8; 16]>());
        host.call(
            "load_plugin",
            json!({
                "id": m.id,
                "dir": dir,
                "entry": m.entry,
                "token": token,
            }),
        )
        .await?;
        let handlers = host
            .call("describe", json!({ "plugin": m.id }))
            .await
            .ok()
            .and_then(|v| serde_json::from_value::<Vec<String>>(v["handlers"].clone()).ok())
            .unwrap_or_default();
        self.plugins.insert(
            m.id.clone(),
            LoadedPlugin {
                manifest: m.clone(),
                dir,
                handlers: handlers.clone(),
                token,
            },
        );
        Ok((m, handlers))
    }

    /// 加载 + 入库：upsert plugins 表、保留既有 accounts 授权，回写 config 快照。
    /// UI 安装入口与 MCP 工具共用此路径。
    pub async fn install_and_register(&self, dir: PathBuf) -> anyhow::Result<Value> {
        let (m, handlers) = self.load(dir.clone()).await?;
        let old_accounts = self
            .db
            .list_plugins()
            .ok()
            .and_then(|ps| ps.into_iter().find(|p| p.uuid == m.id))
            .and_then(|p| serde_json::from_str::<Value>(&p.config).ok())
            .and_then(|c| c["accounts"].as_array().cloned())
            .unwrap_or_default();
        self.db.upsert_plugin(&crate::storage::models::PluginRow {
            uuid: m.id.clone(),
            dir: dir.to_string_lossy().to_string(),
            runtime: format!("{:?}", m.runtime).to_lowercase(),
            name: m.name.clone(),
            enabled: true,
            config: json!({
                "commands": handlers,
                "version": m.version,
                "author": m.author,
                "description": m.description,
                "permissions": m.permissions,
                "accounts": old_accounts,
            })
            .to_string(),
        })?;
        Ok(json!({ "id": m.id, "name": m.name, "handlers": handlers }))
    }

    pub async fn unload(&self, plugin_id: &str) -> anyhow::Result<()> {
        let runtime = match self.plugins.remove(plugin_id) {
            Some((_, p)) => p.manifest.runtime,
            None => return Ok(()),
        };
        // 卸载即清掉它注册的定时任务：留着会在空插件上空转，
        // 且 Python SDK 每次 load 重复 add_timer 会滚雪球
        let _ = self.db.delete_timers_of(plugin_id);
        if let Some(host) = self.hosts.get(&runtime) {
            let _ = host.call("unload_plugin", json!({ "id": plugin_id })).await;
        }
        Ok(())
    }

    /// 投递任意事件（timer 等非消息触发）：invoke {plugin, handler, event}。
    pub async fn invoke_raw(
        &self,
        plugin_id: &str,
        handler: &str,
        event: Value,
    ) -> anyhow::Result<()> {
        let runtime = self
            .plugins
            .get(plugin_id)
            .ok_or_else(|| anyhow::anyhow!("plugin {plugin_id} not loaded"))?
            .manifest
            .runtime;
        let host = self.host_for(runtime).await?;
        host.call(
            "invoke",
            json!({ "plugin": plugin_id, "handler": handler, "event": event }),
        )
        .await?;
        Ok(())
    }

    pub fn loaded_count(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_loaded(&self, plugin_id: &str) -> bool {
        self.plugins.contains_key(plugin_id)
    }

    /// 插件 manifest 声明的权限清单（sdk_api 权限门用；未加载 → None）。
    pub fn permissions_of(&self, plugin_id: &str) -> Option<Vec<String>> {
        self.plugins
            .get(plugin_id)
            .map(|p| p.manifest.permissions.clone())
    }

    /// 校验 API 帧的 (plugin_id, token) 配对：宿主盖章的帧才算本人，
    /// 插件裸写 stdout 伪造的帧过不了这一关。
    pub fn token_matches(&self, plugin_id: &str, token: &str) -> bool {
        self.plugins
            .get(plugin_id)
            .map(|p| p.token == token)
            .unwrap_or(false)
    }

    /// 插件所在 runtime（按宿主分组批投递用）。
    pub fn runtime_of(&self, plugin_id: &str) -> Option<Runtime> {
        self.plugins.get(plugin_id).map(|p| p.manifest.runtime)
    }

    /// 同一 runtime 的一批 (plugin, handler) 合并成一帧批投递：
    /// 事件 JSON 只序列化一次、宿主只收一帧（对齐原版 postEventBatch）。
    pub async fn invoke_batch(
        &self,
        rt: Runtime,
        items: &[(String, String)],
        ev: &serde_json::Value,
    ) -> anyhow::Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        let host = self.host_for(rt).await?;
        host.call(
            "invoke_batch",
            json!({
                "items": items.iter().map(|(p, h)| json!({ "plugin": p, "handler": h })).collect::<Vec<_>>(),
                "event": ev,
            }),
        )
        .await?;
        Ok(())
    }

    /// 已加载插件的 handler 清单（插件详情/调试展示用）。
    pub fn handlers_of(&self, plugin_id: &str) -> Vec<String> {
        self.plugins
            .get(plugin_id)
            .map(|p| p.handlers.clone())
            .unwrap_or_default()
    }

    /// 注册了某个 handler 的插件 id 列表（事件订阅扇出用）。
    pub fn owners_of_handler(&self, handler: &str) -> Vec<String> {
        self.plugins
            .iter()
            .filter(|p| p.handlers.iter().any(|h| h == handler))
            .map(|p| p.key().clone())
            .collect()
    }

    /// 消息文本前缀命中的 cmd handler：返回 (plugin_id, handler)。
    /// "签到 xxx" 会命中注册过 cmd:签到 的所有插件。
    pub fn cmd_matches(&self, text: &str) -> Vec<(String, String)> {
        let text = text.trim();
        if text.is_empty() {
            return Vec::new();
        }
        self.plugins
            .iter()
            .flat_map(|p| {
                p.handlers
                    .iter()
                    .filter_map(|h| {
                        h.strip_prefix("cmd:")
                            .filter(|cmd| !cmd.is_empty() && text.starts_with(cmd))
                            .map(|_| (p.key().clone(), h.clone()))
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    async fn host_for(&self, rt: Runtime) -> anyhow::Result<Arc<PluginHost>> {
        if let Some(h) = self.hosts.get(&rt) {
            if h.is_alive() {
                return Ok(h.clone());
            }
        }
        // 串行化 spawn：并发 load 多个同 runtime 插件时不能各起一个宿主
        let _guard = self.host_lock.lock().await;
        if let Some(h) = self.hosts.get(&rt) {
            if h.is_alive() {
                return Ok(h.clone());
            }
            // 尸体移除，下面重建
            self.hosts.remove(&rt);
        }
        // 解释器路径：设置页可配（node_path/python_path），默认按 PATH 找
        let key = match rt {
            Runtime::Node => "node_path",
            Runtime::Python => "python_path",
        };
        let configured = self.db.get_setting(key).unwrap_or_default();
        let program = if configured.is_empty() {
            match rt {
                Runtime::Node => "node",
                Runtime::Python => "python",
            }
            .to_string()
        } else {
            configured
        };
        let h = PluginHost::spawn(rt, hosts_dir(), self.api_tx.clone(), &program).await?;
        self.hosts.insert(rt, h.clone());

        // 宿主重建后老 Worker 全没了：把该 runtime 已登记的插件逐个装回去
        // （沿用原 token，插件身份凭据不变；失败只记日志不拖垮本次 load）
        let reloaded: Vec<(String, PathBuf, String, String)> = self
            .plugins
            .iter()
            .filter(|p| p.manifest.runtime == rt)
            .map(|p| {
                (
                    p.manifest.id.clone(),
                    p.dir.clone(),
                    p.manifest.entry.clone(),
                    p.token.clone(),
                )
            })
            .collect();
        for (id, dir, entry, token) in reloaded {
            match h
                .call(
                    "load_plugin",
                    json!({ "id": id, "dir": dir, "entry": entry, "token": token }),
                )
                .await
            {
                Ok(_) => tracing::info!(plugin = id, "plugin restored into respawned host"),
                Err(e) => tracing::warn!(plugin = id, "plugin restore failed: {e:#}"),
            }
        }
        Ok(h)
    }
}

/// hosts/ 目录定位：优先 MASHIRO_HOSTS_DIR，其次 exe 旁 hosts/，开发期退回 ./hosts。
fn hosts_dir() -> PathBuf {
    if let Ok(p) = std::env::var("MASHIRO_HOSTS_DIR") {
        return PathBuf::from(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("hosts");
            if p.is_dir() {
                return p;
            }
            // macOS bundle：exe 在 Contents/MacOS/，resources 落在 Contents/Resources/hosts
            let p = dir.join("../Resources/hosts");
            if p.is_dir() {
                return p;
            }
            // cargo dev 时 exe 在 target/debug，回退到 workspace 根
            if let Some(root) = dir.ancestors().nth(2) {
                let p = root.join("hosts");
                if p.is_dir() {
                    return p;
                }
            }
        }
    }
    PathBuf::from("hosts")
}
