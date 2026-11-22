use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use crate::plugin::manager::PluginManager;
use crate::storage::db::Database;

/// 定时任务轮询：插件经 add_timer 注册的任务落 timers 表，
/// 每 15s 扫一遍到期的 → 投递 event:timer 给插件宿主。
/// 周期任务 next_run += repeat_secs；一次性任务到期即删。
pub fn start(db: Arc<Database>, plugins: Arc<PluginManager>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(15));
        loop {
            tick.tick().await;
            let now = chrono::Utc::now().timestamp();
            let timers = match db.due_timers(now) {
                Ok(t) => t,
                Err(e) => {
                    tracing::error!("due_timers: {e}");
                    continue;
                }
            };
            for t in timers {
                if t.repeat_secs > 0 {
                    let _ = db.bump_timer(t.id, now + t.repeat_secs);
                } else {
                    let _ = db.delete_timer(t.id);
                }
                let event = json!({
                    "timer_id": t.id,
                    "remark": t.remark,
                    "fired_at": now,
                });
                if let Err(e) = plugins.invoke_raw(&t.plugin_id, "event:timer", event).await {
                    tracing::warn!("timer {} -> {}: {e}", t.id, t.plugin_id);
                }
            }
        }
    });
}
