use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tokio::sync::{mpsc, Mutex};
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
    /// 最近一次点亮在线的时刻（毫秒）；离线归 0 —— 前端展示「在线时长」用
    online_since: AtomicI64,
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
            online_since: AtomicI64::new(0),
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
        if let Some(h) = guard.as_ref() {
            if self.is_online() && !h.is_finished() {
                // 正常在线：登录是幂等空操作
                return Ok(());
            }
            if !h.is_finished() {
                // 任务活着但离线（多半在退避重试）：用户点登录 = 立刻重连，掐掉重试直接起新会话
                h.abort();
            }
            // 句柄在但任务已死（panic/退出后没人清）：必须清掉，否则永远 start 不了
            *guard = None;
        }
        self.log("System", "开始连接…");
        let this = Arc::clone(self);
        *guard = Some(tokio::spawn(async move {
            if let Err(e) = this.run_gateway().await {
                tracing::error!(appid = this.appid, "gateway exited: {e:#}");
                this.log("Error", format!("连接已退出: {e:#}"));
            }
            this.mark_offline();
        }));
        Ok(())
    }

    /// 网关监督循环：断线指数退避重连；session_id/seq 跨重连保留以走 op6 Resume。
    ///
    /// 在线状态带 30s 宽限：掉线不立刻 mark_offline——官方网关每小时例行 op7
    /// 踢人，Resume 几秒内即可恢复，若每次都标离线，「在线时长」永远超不过 1h
    /// 且状态指示会在每小时闪断一下。连续断线超 30s 才判定真离线。
    async fn run_gateway(self: &Arc<Self>) -> anyhow::Result<()> {
        let session_id = Arc::new(Mutex::new(String::new()));
        let seq = Arc::new(AtomicI64::new(0));
        // 官方网关支持 Resume 补发（服务端短期缓冲 seq 之后的事件）：把游标持久化到
        // settings，进程重启/升级后仍可 op6 恢复，补回停机窗口内的遗漏事件。
        let sess_key = format!("gw_session_{}", self.appid);
        if self.ws_url.is_none() {
            if let Ok(saved) = self.db.get_setting(&sess_key) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&saved) {
                    *session_id.lock().await =
                        v["session_id"].as_str().unwrap_or_default().to_string();
                    seq.store(v["seq"].as_i64().unwrap_or(0), Ordering::Relaxed);
                }
            }
        }
        let mut backoff = 1u64;
        // 连续断线起点（宽限期计时）；一次稳定会话（>60s）后归零
        let mut down_since: Option<std::time::Instant> = None;
        // 本次断开的时刻：Resume 成功后换算成「中断时长」写进恢复日志
        let resume_gap = Arc::new(Mutex::new(Option::<std::time::Instant>::None));
        // 上次发起连接的时刻：session_start_limit.max_concurrency=1（每 5s 一个会话），
        // 快速「秒挂→重连」循环若不做最小间隔会撞限频被服务端踢
        let mut last_attempt: Option<std::time::Instant> = None;
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

                event_tx: tx,
                seq: Arc::clone(&seq),
                session_id: Arc::clone(&session_id),
                last_ack: Arc::new(AtomicI64::new(0)),
            };
            // 事件泵：网关把 op0 帧丢进 rx，这里解析 + 投递全局分发器
            let this = Arc::clone(self);
            let resume_gap2 = Arc::clone(&resume_gap);
            let session_id2 = Arc::clone(&session_id);
            let sess_key2 = sess_key.clone();
            let persist = self.ws_url.is_none();
            let mut pump = tokio::spawn(async move {
                // 事件去重：平台可能重复推送同一 msg_id；Resume 补发/崩溃重放也会重复。
                // 键优先用 事件类型:消息id，无 id 时退回 s 序号；环形窗口 4096。
                let mut dedup_q: std::collections::VecDeque<String> =
                    std::collections::VecDeque::new();
                let mut dedup_s: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                while let Some(v) = rx.recv().await {
                    let t = v["t"].as_str().unwrap_or("");
                    // RESUMED/READY 到达 → 清掉断点计时
                    let gap = if t == "RESUMED" || t == "READY" {
                        resume_gap2.lock().await.take()
                    } else {
                        None
                    };
                    // 任意 op0 帧到达 = 会话已活：官方链路 op6 Resume 后只回 RESUMED 不再发
                    // READY，中转网关也可能永不发 READY——统一以首帧判连通
                    if !this.is_online() {
                        this.mark_online();
                        if this.ws_url.is_some() {
                            this.log("System", "中转网关已连通");
                        }
                    }
                    if t == "RESUMED" {
                        // 宽限内恢复 online_since 不动；宽限外 mark_online 已刷新
                        this.mark_online();
                        match gap {
                            Some(g) => this.log(
                                "System",
                                format!("会话已恢复（中断 {:.1}s）", g.elapsed().as_secs_f64()),
                            ),
                            None => this.log("System", "会话已恢复"),
                        }
                    }
                    if v["t"].as_str() == Some("READY") {
                        // 新会话序号空间重置，去重窗口一并清空
                        dedup_q.clear();
                        dedup_s.clear();
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
                    // 去重键：webhook 帧事件 id 在顶层、WS 帧消息 id 在 d 里、
                    // 都没有时退回 s；三者皆无（控制帧）不做去重
                    let dedup_key = v["id"]
                        .as_str()
                        .filter(|i| !i.is_empty())
                        .map(|i| format!("ev:{i}"))
                        .or_else(|| {
                            v["d"]["id"]
                                .as_str()
                                .filter(|i| !i.is_empty())
                                .map(|i| format!("{t}:{i}"))
                        })
                        .or_else(|| v["s"].as_i64().map(|s| format!("seq:{s}")));
                    let is_dup = match &dedup_key {
                        Some(k) => {
                            let dup = !dedup_s.insert(k.clone());
                            if !dup {
                                dedup_q.push_back(k.clone());
                                if dedup_q.len() > 4096 {
                                    if let Some(k) = dedup_q.pop_front() {
                                        dedup_s.remove(&k);
                                    }
                                }
                            }
                            dup
                        }
                        None => false,
                    };
                    let ev = if is_dup {
                        None
                    } else {
                        crate::router::event::parse(this.appid, t, &v["d"])
                            .or_else(|| crate::router::event::parse_misc(this.appid, t, &v["d"]))
                    };
                    if let Some(ev) = ev {
                        this.dispatch(ev).await;
                    }
                    // 事件处理完才持久化水位线：崩溃时已收未处理的事件 seq 未提交，
                    // 重启 Resume 会从更早的 seq 补发——宁可重启后重复，不可静默丢失。
                    if persist {
                        if let Some(s) = v["s"].as_i64() {
                            let sid = session_id2.lock().await.clone();
                            if !sid.is_empty() {
                                let _ = this.db.set_setting(
                                    &sess_key2,
                                    &serde_json::json!({"session_id": sid, "seq": s}).to_string(),
                                );
                            }
                        }
                    }
                }
            });
            // 距上次连接不足 5s 时补齐——限频只防「秒挂循环」，
            // 正常会话/op7 间隔以小时计，此分支永远不会真的等待
            if let Some(t) = last_attempt {
                let el = t.elapsed();
                if el < Duration::from_secs(5) {
                    tokio::time::sleep(Duration::from_secs(5) - el).await;
                }
            }
            let started = std::time::Instant::now();
            last_attempt = Some(started);
            let r = gw.run(&self.http, &token).await;
            // gw 已返回、event_tx 随 drop 关闭：给泵最多 3s 排空管道中已收未分发的事件，
            // 然后才 abort——否则这批事件 seq 已提交、Resume 不补发，直接丢失。
            if tokio::time::timeout(Duration::from_secs(3), &mut pump)
                .await
                .is_err()
            {
                pump.abort();
            }
            match r {
                Ok(()) => {
                    self.mark_offline();
                    return Ok(());
                }
                Err(e) => {
                    use crate::bot::gateway::{GatewayAuthFail, GatewayFatal, GatewayReconnect};
                    // 已下架/已封禁等致命错误：文档明确不可重连，熔断退出
                    if e.downcast_ref::<GatewayFatal>().is_some() {
                        self.mark_offline();
                        self.log("Error", format!("连接终止: {e:#}"));
                        return Ok(());
                    }
                    // token 被网关拒绝：作废缓存，下轮 access_token() 会强制刷新
                    if e.downcast_ref::<GatewayAuthFail>().is_some() {
                        self.tokens.invalidate();
                    }
                    // 连上超过 60s 才断 = 这次是正常会话 → 退避和断线计时归零
                    if started.elapsed() > Duration::from_secs(60) {
                        backoff = 1;
                        down_since = None;
                    }
                    // 记下断开时刻；若下条会话 Resume 成功，日志给出实际中断时长
                    *resume_gap.lock().await = Some(std::time::Instant::now());
                    // OpCode 7 例行重连：官方网关约每小时下发一次。立即重连（不睡眠），
                    // Resume 期间不标离线——在线时长跨例行重连连续累计才有意义。
                    if e.downcast_ref::<GatewayReconnect>().is_some() {
                        self.log("Info", "服务端要求重连（例行调度），正在恢复会话");
                        continue;
                    }
                    // 宽限期：连续断线超 30s 才对外表现为离线；30s 内恢复视为无缝
                    let down = down_since.get_or_insert_with(std::time::Instant::now);
                    if down.elapsed() > Duration::from_secs(30) {
                        self.mark_offline();
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
        self.mark_offline();
        self.log("System", "已停止");
    }

    pub fn is_online(&self) -> bool {
        self.online.load(Ordering::Relaxed)
    }

    pub fn mark_online(&self) {
        // 仅离线→在线的跳变才刷新起点：resume 后连续事件帧不会把时间越刷越新
        if self
            .online
            .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            self.online_since
                .store(chrono::Utc::now().timestamp_millis(), Ordering::Relaxed);
        }
    }

    pub fn mark_offline(&self) {
        self.online.store(false, Ordering::Relaxed);
        self.online_since.store(0, Ordering::Relaxed);
    }

    /// 在线起始毫秒时间戳；离线返回 0
    pub fn online_since(&self) -> i64 {
        self.online_since.load(Ordering::Relaxed)
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
