use std::sync::Arc;

use tokio::sync::mpsc;

use crate::plugin::manager::PluginManager;
use crate::plugin::manifest::Runtime;
use crate::router::event::MessageEvent;
use crate::storage::db::Database;
use crate::storage::models::LogRow;

/// 事件入口：gateway 收包解析后调 submit()，后台消费循环做
/// 填短ID → 写日志 → cmd 前缀命中 → 事件订阅扇出，投递插件宿主。
#[derive(Clone)]
pub struct Dispatcher {
    tx: mpsc::Sender<MessageEvent>,
}

impl Dispatcher {
    pub async fn submit(&self, ev: MessageEvent) {
        let _ = self.tx.send(ev).await;
    }
}

pub fn start(db: Arc<Database>, plugins: Arc<PluginManager>) -> Dispatcher {
    let (tx, mut rx) = mpsc::channel::<MessageEvent>(1024);
    // (appid, msg_id) 去重：订阅 AT + 全量两种 intent 时，同一条 @ 消息会以
    // GROUP_AT_MESSAGE_CREATE 和 GROUP_MESSAGE_CREATE 两个事件名推两遍。
    // 第一把锁只管「日志 + cmd 命中」；第二把锁按事件名去重——订阅
    // event:GROUP_MESSAGE_CREATE 的插件仍应收到那份全量事件，不能整条丢。
    let seen = dashmap::DashMap::<String, i64>::new();
    let seen_ev = dashmap::DashMap::<String, i64>::new();
    tokio::spawn(async move {
        while let Some(mut ev) = rx.recv().await {
            let now = chrono::Utc::now().timestamp_millis();
            let fresh = ev.msg_id.is_empty()
                || seen
                    .insert(format!("{}:{}", ev.appid, ev.msg_id), now)
                    .is_none();
            if seen.len() > 5000 || seen_ev.len() > 5000 {
                seen.retain(|_, ts| now - *ts < 600_000);
                seen_ev.retain(|_, ts| now - *ts < 600_000);
            }
            ev.user_int = db.short_id(ev.appid, &ev.user).unwrap_or(0);
            handle(&db, &plugins, ev, fresh, &seen_ev, now).await;
        }
    });
    Dispatcher { tx }
}

async fn handle(
    db: &Arc<Database>,
    plugins: &Arc<PluginManager>,
    ev: MessageEvent,
    fresh: bool,
    seen_ev: &dashmap::DashMap<String, i64>,
    now: i64,
) {
    let ev_json = serde_json::json!(&ev);
    if fresh {
        // 1) 写日志（kind 区分 事件/群/私聊 等，前端筛选）；raw 存完整事件 JSON 供调试
        let _ = db.insert_log(&LogRow {
            id: 0,
            ts: chrono::Utc::now().timestamp_millis(),
            appid: ev.appid,
            kind: format!("{:?}", ev.kind),
            group_id: ev.group_id.clone(),
            user: ev.user.clone(),
            content: ev.text.clone(),
            raw: serde_json::to_string(&ev).unwrap_or_default(),
        });

        // 2) 指令前缀命中：注册了 cmd:<前缀> 的插件直接投递。
        //    按 runtime 分组成单帧批投递：一条消息命中 N 个插件时，
        //    事件 JSON 只序列化一次、每个宿主只收一帧。
        let mut batches: std::collections::HashMap<Runtime, Vec<(String, String)>> =
            std::collections::HashMap::new();
        for (plugin_id, handler) in plugins.cmd_matches(&ev.text) {
            if !db.plugin_allowed(&plugin_id, ev.appid) {
                continue;
            }
            // 每指令触发权限门（config.cmd_cfg）：停用/管理员/群主限定在此拦截
            let cmd = handler.strip_prefix("cmd:").unwrap_or(&handler);
            if let Some(c) = db.plugin_cmd_cfg(&plugin_id).get(cmd) {
                if c["enabled"].as_bool() == Some(false) {
                    continue;
                }
                // member_role: 0群主 1管理 2成员 -1未知（私聊等）
                let ok = match c["perm"].as_str().unwrap_or("all") {
                    "owner" => ev.member_role == 0,
                    "admin" => matches!(ev.member_role, 0 | 1),
                    _ => true,
                };
                if !ok {
                    continue;
                }
            }
            let _ = db.bump_plugin_stat(&plugin_id);
            if let Some(rt) = plugins.runtime_of(&plugin_id) {
                batches.entry(rt).or_default().push((plugin_id, handler));
            }
        }
        for (rt, items) in batches {
            if let Err(e) = plugins.invoke_batch(rt, &items, &ev_json).await {
                tracing::warn!("invoke_batch cmd: {e:#}");
            }
        }
    }

    // 3) 事件订阅投递：注册了 event:<原始事件名> 的插件。
    //    按 (appid,msg_id,event_type) 去重——同消息换个事件名再来仍要投递给对应订阅者
    if !ev.msg_id.is_empty()
        && seen_ev
            .insert(format!("{}:{}:{}", ev.appid, ev.msg_id, ev.event_type), now)
            .is_some()
    {
        return;
    }
    let handler = format!("event:{}", ev.event_type);
    // 事件订阅同样按 runtime 分组批投递
    let mut batches: std::collections::HashMap<Runtime, Vec<(String, String)>> =
        std::collections::HashMap::new();
    for pid in plugins.owners_of_handler(&handler) {
        if !db.plugin_allowed(&pid, ev.appid) {
            continue;
        }
        if let Some(rt) = plugins.runtime_of(&pid) {
            batches.entry(rt).or_default().push((pid, handler.clone()));
        }
    }
    for (rt, items) in batches {
        if let Err(e) = plugins.invoke_batch(rt, &items, &ev_json).await {
            tracing::warn!("invoke_batch event: {e:#}");
        }
    }
}
