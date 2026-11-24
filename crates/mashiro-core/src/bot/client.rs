use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tokio::sync::{mpsc, Mutex, Notify};
use tokio::task::JoinHandle;

use crate::bot::{gateway::Gateway, token::TokenStore};
use crate::router::dispatch::Dispatcher;
use crate::router::event::MessageEvent;
use crate::storage::db::Database;
use crate::storage::models::{Account, LogRow};

/// 单个机器人账号的运行时实例：持有网关任务句柄、token 缓存、在线标记。
/// dispatcher 用 OnceLock 共享单元 —— bot 可能先于后台服务启动，单元就绪后自动投事件。
pub struct BotClient {
    pub appid: i64,
    secret: String,
    /// READY 后用网关返回的 username/avatar 实时更新
    profile: std::sync::Mutex<(String, String)>,
    /// 0 WebSocket / 1 Webhook（webhook 账号不起网关，事件由 webhook 服务投递）
    mode: i64,
    sandbox: bool,
    intents: i64,
    /// 自定义 WS 网关（webhook→websocket 中转/自建网关）；非空时 mode=0 也走它
    ws_url: Option<String>,
    /// 群/频道 Markdown 开关（关闭时对应域 send_markdown 降级纯文本）
    pub md_group: bool,
    pub md_guild: bool,
    online: AtomicBool,
    http: reqwest::Client,
    db: Arc<Database>,
    tokens: TokenStore,
    dispatcher: Arc<OnceLock<Dispatcher>>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl BotClient {
    pub fn new(
        acc: Account,
        http: reqwest::Client,
        db: Arc<Database>,
        dispatcher: Arc<OnceLock<Dispatcher>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            appid: acc.appid,
            secret: acc.secret,
            profile: std::sync::Mutex::new((acc.name, acc.avatar)),
            mode: acc.mode,
            sandbox: acc.sandbox,
            intents: if acc.ws_intents == 0 {
                crate::bot::gateway::DEFAULT_INTENTS
            } else {
                acc.ws_intents
            },
            ws_url: if acc.ws_url.trim().is_empty() {
                None
            } else {
                Some(acc.ws_url)
            },
            md_group: acc.md_group,
            md_guild: acc.md_guild,
            online: AtomicBool::new(false),
            http,
            db,
            tokens: TokenStore::new(),
            dispatcher,
            task: Mutex::new(None),
        })
    }

    /// HTTP API 域名（沙盒账号走腾讯沙盒环境）。
    pub fn api_base(&self) -> &'static str {
        crate::bot::api::api_base(self.sandbox)
    }

    pub fn is_webhook(&self) -> bool {
        self.mode == 1
    }

    pub async fn start(self: &Arc<Self>) -> anyhow::Result<()> {
        // Webhook 账号不建 WS 连接：注册即在线，事件由 webhook 服务投递。
        if self.is_webhook() {
            self.mark_online();
            self.log("System", "Webhook 模式已就绪");
            return Ok(());
        }
        let mut guard = self.task.lock().await;
        if guard.is_some() {
            return Ok(());
        }
        self.log("System", "开始连接…");
        let this = Arc::clone(self);
        *guard = Some(tokio::spawn(async move {
            if let Err(e) = this.run_gateway().await {
                tracing::error!(appid = this.appid, "gateway exited: {e:#}");
                this.log("Error", format!("连接已退出: {e:#}"));
            }
            this.online.store(false, Ordering::Relaxed);
        }));
        Ok(())
    }

    /// 网关监督循环：断线指数退避重连；session_id/seq 跨重连保留以走 op6 Resume。
    async fn run_gateway(self: &Arc<Self>) -> anyhow::Result<()> {
        let session_id = Arc::new(Mutex::new(String::new()));
        let seq = Arc::new(AtomicI64::new(0));
        let mut backoff = 1u64;
        loop {
            let token = match self.access_token().await {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(appid = self.appid, "get token: {e:#}");
                    self.log("Error", format!("获取 access_token 失败: {e:#}"));
                    tokio::time::sleep(Duration::from_secs(backoff)).await;
                    backoff = (backoff * 2).min(300);
                    continue;
                }
            };
            let (tx, mut rx) = mpsc::unbounded_channel();
            let gw = Gateway {
                appid: self.appid,
                intents: self.intents,
                sandbox: self.sandbox,
                ws_url: self.ws_url.clone(),
                stop: Arc::new(Notify::new()),
                event_tx: tx,
                seq: Arc::clone(&seq),
                session_id: Arc::clone(&session_id),
            };
            // 事件泵：网关把 op0 帧丢进 rx，这里解析 + 投递全局分发器
            let this = Arc::clone(self);
            let pump = tokio::spawn(async move {
                while let Some(v) = rx.recv().await {
                    // 任意 op0 帧到达 = 会话已活：官方链路 op6 Resume 后只回 RESUMED 不再发
                    // READY，中转网关也可能永不发 READY——统一以首帧判连通
                    if !this.is_online() {
                        this.mark_online();
                        if v["t"].as_str() == Some("RESUMED") {
                            this.log("System", "会话已恢复");
                        } else if this.ws_url.is_some() {
                            this.log("System", "中转网关已连通");
                        }
                    }
                    if v["t"].as_str() == Some("READY") {
                        this.mark_online();
                        this.set_profile(
                            v["d"]["user"]["username"].as_str().unwrap_or(""),
                            v["d"]["user"]["avatar"].as_str().unwrap_or(""),
                        );
                        // READY 的 avatar 常为空 → 异步用 /users/@me 兜底拉取
                        let this2 = Arc::clone(&this);
                        tokio::spawn(async move {
                            if let Ok(t) = this2.access_token().await {
                                match crate::bot::api::bot_info(&this2.http, &t, this2.api_base())
                                    .await
                                {
                                    Ok(info) => this2.set_profile(
                                        info["username"].as_str().unwrap_or(""),
                                        info["avatar"].as_str().unwrap_or(""),
                                    ),
                                    Err(e) => {
                                        tracing::warn!(appid = this2.appid, "/users/@me: {e:#}")
                                    }
                                }
                            }
                        });
                        this.log("System", "已上线");
                    }
                    let t = v["t"].as_str().unwrap_or("");
                    let ev = crate::router::event::parse(this.appid, t, &v["d"])
                        .or_else(|| crate::router::event::parse_misc(this.appid, t, &v["d"]));
                    if let Some(ev) = ev {
                        this.dispatch(ev).await;
                    }
                }
            });
            let started = std::time::Instant::now();
            let r = gw.run(&self.http, &token).await;
            pump.abort();
            self.mark_offline();
            match r {
                Ok(()) => return Ok(()),
                Err(e) => {
                    // 连上超过 60s 才断 = 这次是正常会话 → 退避归零，别拿旧惩罚接着用
                    if started.elapsed() > Duration::from_secs(60) {
                        backoff = 1;
                    }
                    tracing::warn!(appid = self.appid, "gateway: {e:#}, retry in {backoff}s");
                    self.log("Error", format!("连接断开: {e:#}，{backoff}s 后重连"));
                    tokio::time::sleep(Duration::from_secs(backoff)).await;
                    backoff = (backoff * 2).min(300);
                }
            }
        }
    }

    pub async fn stop(&self) {
        if let Some(h) = self.task.lock().await.take() {
            h.abort();
        }
        self.online.store(false, Ordering::Relaxed);
        self.log("System", "已停止");
    }

    pub fn is_online(&self) -> bool {
        self.online.load(Ordering::Relaxed)
    }

    pub fn mark_online(&self) {
        self.online.store(true, Ordering::Relaxed);
    }

    pub fn mark_offline(&self) {
        self.online.store(false, Ordering::Relaxed);
    }

    pub fn name(&self) -> String {
        self.profile.lock().unwrap().0.clone()
    }

    pub fn avatar(&self) -> String {
        self.profile.lock().unwrap().1.clone()
    }

    /// READY 帧携带 d.user → 更新内存并落库，前端下次拉取即是真名/头像。
    fn set_profile(&self, name: &str, avatar: &str) {
        {
            let mut p = self.profile.lock().unwrap();
            if !name.is_empty() {
                p.0 = name.to_string();
            }
            if !avatar.is_empty() {
                p.1 = avatar.to_string();
            }
            let (n, a) = (p.0.clone(), p.1.clone());
            drop(p);
            if !n.is_empty() || !a.is_empty() {
                let _ = self.db.update_account_profile(self.appid, &n, &a);
            }
        }
    }

    pub fn secret(&self) -> &str {
        &self.secret
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    pub fn db(&self) -> &Arc<Database> {
        &self.db
    }

    /// 事件交给全局分发器（未就绪时丢弃）。
    pub async fn dispatch(&self, ev: MessageEvent) {
        if let Some(d) = self.dispatcher.get() {
            d.submit(ev).await;
        }
    }

    /// 取一个可用的 access_token（缓存命中直接返回，否则去刷新）。
    pub async fn access_token(&self) -> anyhow::Result<String> {
        self.tokens.get(&self.http, self.appid, &self.secret).await
    }

    /// 写一条运行日志（连接生命周期/发送/错误，前端日志页可见）。
    pub fn log(&self, kind: &str, content: impl Into<String>) {
        let _ = self.db.insert_log(&LogRow {
            id: 0,
            ts: chrono::Utc::now().timestamp_millis(),
            appid: self.appid,
            kind: kind.to_string(),
            group_id: String::new(),
            user: String::new(),
            content: content.into(),
            raw: String::new(),
        });
    }

    /// 发一条纯文本消息（插件 API / 关键词回复都走这里）。
    pub async fn send_text(
        &self,
        kind: i32,
        openid: &str,
        text: &str,
        msg_id: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.access_token().await?;
        let r = crate::bot::api::send_text(
            &self.http,
            &token,
            self.api_base(),
            kind,
            openid,
            text,
            crate::bot::api::SendOpts {
                msg_id: msg_id.map(str::to_string),
                ..Default::default()
            },
        )
        .await;
        let _ = self.db.insert_log(&LogRow {
            id: 0,
            ts: chrono::Utc::now().timestamp_millis(),
            appid: self.appid,
            kind: if r.is_ok() { "Send" } else { "Error" }.to_string(),
            group_id: openid.to_string(),
            user: String::new(),
            content: match &r {
                Ok(_) => text.chars().take(200).collect(),
                Err(e) => format!("发送失败: {e:#}"),
            },
            raw: serde_json::to_string(&serde_json::json!({
                "request": { "kind": kind, "openid": openid, "text": text, "msg_id": msg_id },
                "response": match &r {
                    Ok(v) => v.clone(),
                    Err(e) => serde_json::Value::String(format!("{e:#}")),
                },
            }))
            .unwrap_or_default(),
        });
        r
    }
}
