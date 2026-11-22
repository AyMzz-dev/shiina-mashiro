use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub appid: i64,
    pub secret: String,
    pub name: String,
    pub avatar: String,
    pub enabled: bool,
    /// 连接方式：0 WebSocket / 1 Webhook
    #[serde(default)]
    pub mode: i64,
    /// 腾讯沙盒环境
    #[serde(default)]
    pub sandbox: bool,
    /// 群 Markdown 开关（关闭时 send_markdown 降级纯文本）
    #[serde(default)]
    pub md_group: bool,
    /// 频道原生 Markdown
    #[serde(default)]
    pub md_guild: bool,
    /// 频道模板 Markdown
    #[serde(default)]
    pub md_guild_tpl: bool,
    /// WS intents 位掩码；0 = 默认订阅
    #[serde(default)]
    pub ws_intents: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginRow {
    pub uuid: String,
    pub dir: String,
    pub runtime: String,
    pub name: String,
    pub enabled: bool,
    pub config: String,
}

/// 插件注册的定时任务（scheduler 轮询到期 → 投递 event:timer）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimerRow {
    pub id: i64,
    pub plugin_id: String,
    pub remark: String,
    /// 下次执行时间（unix 秒）
    pub next_run: i64,
    /// >0 = 周期秒数；0 = 一次性
    pub repeat_secs: i64,
    pub enabled: bool,
}

/// 聊天室会话条目：(appid, group_id) 聚合的最新一条消息预览。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub appid: i64,
    pub group_id: String,
    /// 最新一条日志的类型（可能为 Send；会话类型以前端推断的最近接收类型为准）
    pub kind: String,
    pub last_content: String,
    pub last_ts: i64,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogRow {
    pub id: i64,
    pub ts: i64,
    pub appid: i64,
    /// 事件类型：Group / Guild / C2C / GuildDm / Send / Error ...
    pub kind: String,
    pub group_id: String,
    pub user: String,
    pub content: String,
    /// 标准化事件完整 JSON（插件收到的就是这份）；非事件来源为空串。
    #[serde(default)]
    pub raw: String,
}
