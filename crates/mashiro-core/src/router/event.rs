use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

/// 会话类型：与旧框架 type 语义对齐，插件侧也只认这四个值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum EventKind {
    Group = 0,   // 群
    Guild = 1,   // 频道
    C2C = 2,     // 私聊
    GuildDm = 3, // 频道私聊
}

impl Serialize for EventKind {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_i32(*self as i32)
    }
}

impl<'de> Deserialize<'de> for EventKind {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match i32::deserialize(d)? {
            1 => Self::Guild,
            2 => Self::C2C,
            3 => Self::GuildDm,
            _ => Self::Group,
        })
    }
}

/// 标准化消息事件：框架内部字段（seq/日志id/位图等）不进事件，
/// 插件只看到这些。字段比旧版 25+ 的 MessageEvent 收敛到真正需要的。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageEvent {
    pub appid: i64,
    pub kind: EventKind,
    pub event_type: String, // 原始事件名，如 GROUP_AT_MESSAGE_CREATE
    pub group_id: String,   // 群openid / channel_id / 私聊对端 openid
    pub guild_id: String,   // 频道消息才有
    pub user: String,       // openid / member_openid
    pub nickname: String,
    pub group_name: String,
    pub msg_id: String,
    pub text: String,
    pub reply_to: String,
    pub at_me: bool,
    pub member_role: i32, // 0 群主 1 管理 2 成员
    pub user_int: i32,    // 整数短 ID（dispatch 阶段由 uid_map 填充）
    /// INTERACTION_CREATE：按钮/菜单回调数据（resolved.button_data，免挖 raw）
    pub button_data: String,
    /// INTERACTION_CREATE：应答要用的 interaction_id（取 d.id）
    pub interaction_id: String,
    /// 场景标记：INTERACTION 的 scene / 消息的 message_scene.source
    pub scene: String,
    /// 操作者 openid：成员事件的执行人（op_member_openid）、审批人等
    pub operator: String,
    /// 入群方式（GROUP_MEMBER_ADD 的 join_type）
    pub join_type: String,
    /// 附件列表（图/文件/语音…），原样透传 d.attachments
    pub attachments: Vec<Value>,
    /// 被 @ 的成员 openid/id 列表
    pub mentions: Vec<String>,
    /// 是否 @ 了全员（d.mention_everyone）
    pub mention_everyone: bool,
    /// 消息序号（群消息回复/排序定位）
    pub msg_seq: i64,
    /// 发送者是机器人账号（author.bot，插件一般据此跳过）
    pub is_bot: bool,
    /// INTERACTION 的 resolved 对象整体透传（button_id/feature_id/message_id/user_id 都在里面）
    pub resolved: Value,
    /// 事件时间戳（d.timestamp，ISO 字符串原样透传）
    pub event_ts: String,
    /// 数值子类型：MEMBER_DECREASE 的 type（1主动退/2被移）、INTERACTION 的 type（11按钮/12菜单）等
    pub sub_type: i32,
    pub raw: Value, // 原始 d 对象，兜底
}

fn base(appid: i64, kind: EventKind, event_type: &str, d: &Value) -> MessageEvent {
    MessageEvent {
        appid,
        kind,
        event_type: event_type.to_string(),
        group_id: String::new(),
        guild_id: String::new(),
        user: String::new(),
        nickname: String::new(),
        group_name: String::new(),
        msg_id: String::new(),
        text: String::new(),
        reply_to: String::new(),
        at_me: false,
        member_role: -1,
        user_int: 0,
        button_data: String::new(),
        interaction_id: String::new(),
        scene: String::new(),
        operator: String::new(),
        join_type: String::new(),
        attachments: Vec::new(),
        mentions: Vec::new(),
        mention_everyone: false,
        msg_seq: 0,
        is_bot: false,
        resolved: Value::Null,
        event_ts: String::new(),
        sub_type: 0,
        raw: d.clone(),
    }
}

/// 消息/通用事件的公共解析：附件、@提及列表、场景来源。
fn fill_common(ev: &mut MessageEvent, d: &Value) {
    if let Some(a) = d["attachments"].as_array() {
        ev.attachments = a.clone();
    }
    if let Some(a) = d["mentions"].as_array() {
        ev.mentions = a
            .iter()
            .filter_map(|m| {
                m["member_openid"]
                    .as_str()
                    .or_else(|| m["id"].as_str())
                    .or_else(|| m["user_openid"].as_str())
                    .map(str::to_string)
            })
            .collect();
    }
    if ev.scene.is_empty() {
        if let Some(src) = d["message_scene"]["source"].as_str() {
            ev.scene = src.to_string();
        }
    }
    // 回复引用（message_reference）+ 序号 + @全员 + 机器人标记
    if ev.reply_to.is_empty() {
        if let Some(id) = d["message_reference"]["message_id"].as_str() {
            ev.reply_to = id.to_string();
        }
    }
    if let Some(seq) = d["msg_seq"].as_i64() {
        ev.msg_seq = seq;
    }
    ev.mention_everyone = d["mention_everyone"].as_bool().unwrap_or(false);
    ev.is_bot = d["author"]["bot"].as_bool().unwrap_or(false);
    if ev.event_ts.is_empty() {
        if let Some(ts) = d["timestamp"].as_str() {
            ev.event_ts = ts.to_string();
        }
    }
    if ev.sub_type == 0 {
        if let Some(t) = d["type"].as_i64() {
            ev.sub_type = t as i32;
        }
    }
}

/// 把 `<@!openid>` 这类 mention 从正文剥掉。
fn strip_mentions(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(i) = rest.find("<@!") {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        match rest.find('>') {
            Some(j) => rest = &rest[j + 1..],
            None => break,
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// WS op0 dispatch → MessageEvent。t = 事件名，d = 事件体。
/// 未覆盖的事件类型（进群/回调/表情表态…）后续按需通过 event:* 订阅扩展。
pub fn parse(appid: i64, t: &str, d: &Value) -> Option<MessageEvent> {
    let s = |k: &str| d[k].as_str().unwrap_or("").to_string();
    let author = &d["author"];

    match t {
        // GROUP_MESSAGE_CREATE = 全量模式（群主开「获取群内全部消息」后才有），at_me 标记是否 @ 机器人
        "GROUP_AT_MESSAGE_CREATE" | "GROUP_MESSAGE_CREATE" => {
            let mut ev = base(appid, EventKind::Group, t, d);
            ev.group_id = if s("group_openid").is_empty() {
                s("group_id")
            } else {
                s("group_openid")
            };
            ev.user = author["member_openid"].as_str().unwrap_or("").to_string();
            ev.msg_id = s("id");
            ev.text = strip_mentions(&s("content"));
            ev.at_me = t == "GROUP_AT_MESSAGE_CREATE";
            ev.member_role = 2;
            fill_common(&mut ev, d);
            Some(ev)
        }
        // 注意：C2C_MSG_RECEIVE / C2C_MSG_REJECT 是回执通知（无 author/content），
        // 不属于消息事件，走 parse_misc 透传；官方事件名没有 C2C_MESSAGE_REJECT。
        "C2C_MESSAGE_CREATE" => {
            let mut ev = base(appid, EventKind::C2C, t, d);
            ev.group_id = author["user_openid"].as_str().unwrap_or("").to_string();
            ev.user = ev.group_id.clone();
            ev.msg_id = s("id");
            ev.text = s("content").trim().to_string();
            fill_common(&mut ev, d);
            Some(ev)
        }
        "AT_MESSAGE_CREATE" | "MESSAGE_CREATE" => {
            let mut ev = base(appid, EventKind::Guild, t, d);
            ev.group_id = s("channel_id");
            ev.guild_id = s("guild_id");
            ev.user = author["id"].as_str().unwrap_or("").to_string();
            ev.nickname = author["username"].as_str().unwrap_or("").to_string();
            ev.msg_id = s("id");
            ev.text = strip_mentions(&s("content"));
            ev.at_me = t == "AT_MESSAGE_CREATE";
            if let Some(roles) = d["member"]["roles"].as_array() {
                ev.member_role = roles
                    .iter()
                    .filter_map(|r| r.as_str())
                    .find_map(|r| r.parse::<i32>().ok())
                    .unwrap_or(2);
            }
            fill_common(&mut ev, d);
            Some(ev)
        }
        "DIRECT_MESSAGE_CREATE" => {
            let mut ev = base(appid, EventKind::GuildDm, t, d);
            ev.group_id = s("channel_id");
            ev.guild_id = s("guild_id");
            ev.user = author["id"].as_str().unwrap_or("").to_string();
            ev.nickname = author["username"].as_str().unwrap_or("").to_string();
            ev.msg_id = s("id");
            ev.text = s("content").trim().to_string();
            fill_common(&mut ev, d);
            Some(ev)
        }
        _ => None,
    }
}

/// 非消息类事件兜底：进群/加好友/机器人入群退群/表情表态…
/// 产出「空文本」标准化事件，只走 event:<原始事件名> 订阅投递，
/// 不触发 cmd 指令（text 为空天然不匹配任何指令前缀）。
pub fn parse_misc(appid: i64, t: &str, d: &Value) -> Option<MessageEvent> {
    // READY/RESUMED 之类握手帧不进事件流
    if t.is_empty() || t == "READY" || t == "RESUMED" {
        return None;
    }
    let s = |k: &str| d[k].as_str().unwrap_or("").to_string();
    let kind = if !s("group_openid").is_empty() {
        EventKind::Group
    } else if !s("guild_id").is_empty() || !s("channel_id").is_empty() {
        EventKind::Guild
    } else {
        EventKind::C2C
    };
    let mut ev = base(appid, kind, t, d);
    ev.group_id = if !s("group_openid").is_empty() {
        s("group_openid")
    } else if !s("channel_id").is_empty() {
        s("channel_id")
    } else {
        s("openid")
    };
    ev.guild_id = s("guild_id");
    ev.user = if !s("openid").is_empty() {
        s("openid")
    } else if !s("user_openid").is_empty() {
        s("user_openid")
    } else if !s("member_openid").is_empty() {
        s("member_openid")
    } else {
        d["author"]["id"].as_str().unwrap_or("").to_string()
    };

    // INTERACTION_CREATE：按钮/菜单回调——框架直接解出 button_data/interaction_id，
    // 插件不用手挖 raw.data.resolved（纯白同款全量解析思路）
    if t == "INTERACTION_CREATE" {
        let data = &d["data"];
        ev.button_data = data["resolved"]["button_data"]
            .as_str()
            .or_else(|| data["button_data"].as_str())
            .unwrap_or("")
            .to_string();
        ev.interaction_id = s("id");
        ev.scene = s("scene");
        ev.resolved = data["resolved"].clone();
        if ev.user.is_empty() {
            ev.user = data["resolved"]["user_id"]
                .as_str()
                .unwrap_or("")
                .to_string();
        }
    }
    // 成员变动/审批类事件：操作者与入群方式顺手解出
    if ev.operator.is_empty() {
        ev.operator = s("op_member_openid");
    }
    ev.join_type = s("join_type");
    // 回执类事件（GROUP_MSG_RECEIVE/REJECT 等）带 msg_id——插件拿它对应主动消息
    if ev.msg_id.is_empty() {
        ev.msg_id = s("msg_id");
    }
    fill_common(&mut ev, d);
    Some(ev)
}
