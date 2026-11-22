use std::sync::Arc;

use tokio::sync::mpsc;

use crate::plugin::manager::PluginManager;
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
    let seen = dashmap::DashMap::<String, i64>::new();
    tokio::spawn(async move {
        while let Some(mut ev) = rx.recv().await {
            if !ev.msg_id.is_empty() {
                let now = chrono::Utc::now().timestamp_millis();
                if seen
                    .insert(format!("{}:{}", ev.appid, ev.msg_id), now)
                    .is_some()
                {
                    continue;
                }
                if seen.len() > 5000 {
                    seen.retain(|_, ts| now - *ts < 600_000);
                }
            }
            ev.user_int = db.short_id(ev.appid, &ev.user).unwrap_or(0);
            handle(&db, &plugins, ev).await;
        }
    });
    Dispatcher { tx }
}

async fn handle(db: &Arc<Database>, plugins: &Arc<PluginManager>, ev: MessageEvent) {
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

    // 2) 指令前缀命中：注册了 cmd:<前缀> 的插件直接投递（插件声明即生效）
    for (plugin_id, handler) in plugins.cmd_matches(&ev.text) {
        if !db.plugin_allowed(&plugin_id, ev.appid) {
            continue;
        }
        let _ = db.bump_plugin_stat(&plugin_id);
        if let Err(e) = plugins.invoke_raw(&plugin_id, &handler, serde_json::json!(ev)).await {
            tracing::warn!("plugin {plugin_id} invoke {handler}: {e}");
        }
    }

    // 3) 事件订阅投递：注册了 event:<原始事件名> 的插件
    let handler = format!("event:{}", ev.event_type);
    for pid in plugins.owners_of_handler(&handler) {
        if !db.plugin_allowed(&pid, ev.appid) {
            continue;
        }
        if let Err(e) = plugins.invoke_raw(&pid, &handler, serde_json::json!(ev)).await {
            tracing::warn!("plugin {pid} invoke {handler}: {e}");
        }
    }
}
