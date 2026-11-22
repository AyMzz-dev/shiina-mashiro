use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use dashmap::DashMap;
use tokio::sync::mpsc;

use mashiro_core::agent::Msg;
use mashiro_core::bot::bind::BindTask;
use mashiro_core::bot::client::BotClient;
use mashiro_core::plugin::host::HostApiCall;
use mashiro_core::plugin::manager::PluginManager;
use mashiro_core::plugin::sdk_api::{self, ApiCtx};
use mashiro_core::router::dispatch::{self, Dispatcher};
use mashiro_core::scheduler;
use mashiro_core::storage::db::Database;

/// Agent 会话：runtime 锁定首轮选定值，msgs 给模型多轮上下文，files 是最新产出。
pub struct AgentSession {
    pub runtime: String,
    pub msgs: Vec<Msg>,
    pub files: Vec<(String, String)>,
}

/// 全局应用状态：数据库、HTTP 客户端、在线 bot、插件管理器、事件分发器、插件 API 通道。
pub struct AppState {
    pub db: Arc<Database>,
    pub http: reqwest::Client,
    pub bots: Arc<DashMap<i64, Arc<BotClient>>>,
    pub plugins: Arc<PluginManager>,
    /// OnceLock 共享单元：BotClient 拿着它，服务就绪后事件自然开始投递。
    pub dispatcher: Arc<OnceLock<Dispatcher>>,
    /// 进行中的扫码绑定会话：task_id → 会话（含本地解密密钥）
    pub bind_tasks: Mutex<HashMap<String, BindTask>>,
    /// Agent 生成会话：session_id → 多轮对话 + 当前文件集
    pub agent_sessions: Mutex<HashMap<String, AgentSession>>,
    api_rx: Mutex<Option<mpsc::UnboundedReceiver<HostApiCall>>>,
}

/// 旧版插件目录 %APPDATA%/mashiro/plugins → <运行目录>/data/plugins 一次性迁移。
/// 只拷不删；同时把 db.plugins.dir 里指旧根的绝对路径改到新根。
fn migrate_plugins_dir(db: &Database) {
    let new_root = mashiro_core::storage::db::default_dir().join("plugins");
    let Some(old_root) = dirs_next::data_dir().map(|d| d.join("mashiro").join("plugins")) else {
        return;
    };
    if !old_root.is_dir() || new_root.exists() {
        return;
    }
    if let Err(e) = copy_dir(&old_root, &new_root) {
        tracing::warn!("plugins dir migrate failed: {e}");
        return;
    }
    if let Ok(rows) = db.list_plugins() {
        let old_s = old_root.to_string_lossy().to_string();
        let new_s = new_root.to_string_lossy().to_string();
        for mut p in rows {
            if let Some(rest) = p.dir.strip_prefix(&old_s) {
                p.dir = format!("{new_s}{rest}");
                let _ = db.upsert_plugin(&p);
            }
        }
    }
    tracing::info!("plugins migrated -> {}", new_root.display());
}

fn copy_dir(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let to = dst.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &to)?;
        } else {
            std::fs::copy(e.path(), &to)?;
        }
    }
    Ok(())
}

impl AppState {
    pub fn new() -> Self {
        let (api_tx, api_rx) = mpsc::unbounded_channel();
        let db = Arc::new(Database::open_default().expect("open mashiro.db"));
        migrate_plugins_dir(&db);
        Self {
            db: Arc::clone(&db),
            http: reqwest::Client::new(),
            bots: Arc::new(DashMap::new()),
            plugins: Arc::new(PluginManager::new(api_tx, db)),
            dispatcher: Arc::new(OnceLock::new()),
            bind_tasks: Mutex::new(HashMap::new()),
            agent_sessions: Mutex::new(HashMap::new()),
            api_rx: Mutex::new(Some(api_rx)),
        }
    }

    /// 必须在 tokio 上下文里调用（lib.rs 经 tauri::async_runtime::spawn 调起）。
    pub fn start_services(&self) {
        // 事件分发器
        let d = dispatch::start(Arc::clone(&self.db), Arc::clone(&self.plugins));
        let _ = self.dispatcher.set(d);

        // 定时任务轮询
        scheduler::start(Arc::clone(&self.db), Arc::clone(&self.plugins));

        // Webhook 接入（webhook_listen 非空时启用，默认关）
        mashiro_core::webhook::start(Arc::clone(&self.db), Arc::clone(&self.dispatcher));

        // MCP 调试服务（mcp_listen 非空时启用，Bearer 鉴权）
        mashiro_core::mcp::start(mashiro_core::mcp::McpDeps {
            db: Arc::clone(&self.db),
            plugins: Arc::clone(&self.plugins),
            bots: Arc::clone(&self.bots),
            http: self.http.clone(),
        });

        // 日志保留：启动时清一次，之后每天清一次（log_keep_days 默认 30）
        let db = Arc::clone(&self.db);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(86400));
            loop {
                let days: i64 = db
                    .get_setting("log_keep_days")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(30)
                    .clamp(1, 3650);
                let cutoff = chrono::Utc::now().timestamp_millis() - days * 86400_000;
                if let Ok(n) = db.delete_logs_before(cutoff) {
                    if n > 0 {
                        tracing::info!("log retention: removed {n} rows");
                    }
                }
                tick.tick().await;
            }
        });

        // 插件 API 消费循环：host 读循环 → api_tx → sdk_api::dispatch → host.respond
        if let Some(mut rx) = self.api_rx.lock().unwrap().take() {
            let ctx = ApiCtx {
                db: Arc::clone(&self.db),
                bots: Arc::clone(&self.bots),
                http: self.http.clone(),
                plugins: Arc::clone(&self.plugins),
                plugin: String::new(),
            };
            tokio::spawn(async move {
                while let Some(call) = rx.recv().await {
                    let mut c = ctx.clone();
                    c.plugin = call.plugin.clone().unwrap_or_default();
                    let result = sdk_api::dispatch(&c, &call.method, &call.params).await;
                    if let Err(e) = call.host.respond(call.id, result).await {
                        tracing::warn!("api respond failed: {e}");
                    }
                }
            });
        }

        self.autoload();
    }

    /// 自启：加载 enabled 插件 → 启动 enabled 账号。
    fn autoload(&self) {
        if let Ok(rows) = self.db.list_plugins() {
            for p in rows.into_iter().filter(|p| p.enabled) {
                let plugins = Arc::clone(&self.plugins);
                let dir = PathBuf::from(p.dir);
                let uuid = p.uuid;
                tokio::spawn(async move {
                    if let Err(e) = plugins.load(dir).await {
                        tracing::warn!("autoload plugin {uuid}: {e}");
                    }
                });
            }
        }
        if let Ok(accs) = self.db.list_accounts() {
            for acc in accs.into_iter().filter(|a| a.enabled) {
                let client = BotClient::new(
                    acc,
                    self.http.clone(),
                    Arc::clone(&self.db),
                    Arc::clone(&self.dispatcher),
                );
                let bots = Arc::clone(&self.bots);
                tokio::spawn(async move {
                    if let Err(e) = client.start().await {
                        tracing::warn!("autostart bot {}: {e}", client.appid);
                    }
                    bots.insert(client.appid, client);
                });
            }
        }
    }
}
