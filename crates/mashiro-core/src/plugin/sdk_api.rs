use std::sync::Arc;

use dashmap::DashMap;
use serde_json::{json, Value};

use crate::bot::{api, client::BotClient};
use crate::plugin::manager::PluginManager;
use crate::storage::db::Database;
use crate::storage::models::{LogRow, TimerRow};

/// 插件 API 调用上下文：db / 在线 bot / http / 插件管理器 / 调用方插件 id。
#[derive(Clone)]
pub struct ApiCtx {
    pub db: Arc<Database>,
    pub bots: Arc<DashMap<i64, Arc<BotClient>>>,
    pub http: reqwest::Client,
    pub plugins: Arc<PluginManager>,
    pub plugin: String,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn req_str<'a>(p: &'a Value, k: &str) -> Result<&'a str, String> {
    p[k].as_str().ok_or_else(|| format!("param {k} required"))
}

fn req_i64(p: &Value, k: &str) -> Result<i64, String> {
    p[k].as_i64()
        .or_else(|| p[k].as_str().and_then(|s| s.parse().ok()))
        .ok_or_else(|| format!("param {k} required"))
}

/// method → 需要的 manifest 权限；None = 不校验（ping/log/本地只读查询）。
fn required_perm(method: &str) -> Option<&'static str> {
    Some(match method {
        "kv_get" | "kv_set" | "kv_del" | "kv_incr" => "kv",
        "send_message" | "send_markdown" => "send",
        "send_media" | "material_list" | "material_upload" | "material_delete" => "media",
        "recall" => "recall",
        "add_timer" | "cancel_timer" => "timer",
        "guild_info" | "get_member_list" | "get_member_info" | "kick_member" => "group_admin",
        "http_request" => "http",
        _ => return None,
    })
}

/// 权限门：manifest.permissions 是白名单——声明了才放行对应能力；
/// 空表/未加载插件对所有受限方法一律拒绝（报错信息自带修复指引）。
fn check_perm(ctx: &ApiCtx, method: &str) -> Result<(), String> {
    let Some(perm) = required_perm(method) else {
        return Ok(());
    };
    let granted = ctx
        .plugins
        .permissions_of(&ctx.plugin)
        .is_some_and(|ps| ps.iter().any(|x| x == perm));
    if granted {
        Ok(())
    } else {
        Err(format!(
            "permission denied: 插件未声明 \"{perm}\" 权限（在 plugin.json 的 permissions 中添加后重载）"
        ))
    }
}

/// 插件 → 框架 的 API 分派。返回值经 proto::respond 写回插件。
pub async fn dispatch(ctx: &ApiCtx, method: &str, p: &Value) -> Result<Value, String> {
    check_perm(ctx, method)?;
    let db = &*ctx.db;
    match method {
        "ping" => Ok(json!("pong")),

        "log" => {
            let _ = db.insert_log(&LogRow {
                id: 0,
                ts: chrono::Utc::now().timestamp_millis(),
                appid: req_i64(p, "appid").unwrap_or(0),
                kind: "Plugin".into(),
                group_id: String::new(),
                user: ctx.plugin.clone(),
                content: p["msg"].as_str().unwrap_or_default().to_string(),
                raw: String::new(),
            });
            Ok(Value::Null)
        }

        // ---------- 插件私有 KV ----------
        "kv_get" => db
            .kv_get(&ctx.plugin, req_str(p, "key")?)
            .map(|v| json!(v))
            .map_err(err),
        "kv_set" => db
            .kv_set(
                &ctx.plugin,
                req_str(p, "key")?,
                p["value"].as_str().unwrap_or_default(),
            )
            .map(|_| Value::Null)
            .map_err(err),
        "kv_del" => db
            .kv_del(&ctx.plugin, req_str(p, "key")?)
            .map(|_| Value::Null)
            .map_err(err),
        "kv_incr" => db
            .kv_incr(&ctx.plugin, req_str(p, "key")?)
            .map(|n| json!(n))
            .map_err(err),

        // ---------- 发消息 ----------
        "send_message" => {
            let appid = req_i64(p, "appid")?;
            let bot = bot_of(ctx, appid)?;
            let token = bot.access_token().await.map_err(err)?;
            let openid = req_str(p, "openid")?;
            let text = req_str(p, "text")?;
            let r = api::send_text(
                &ctx.http,
                &token,
                bot.api_base(),
                p["kind"].as_i64().unwrap_or(0) as i32,
                openid,
                text,
                send_opts(p),
            )
            .await;
            log_send(db, appid, openid, p, text, &r);
            r.map_err(err)
        }
        "send_markdown" => {
            let appid = req_i64(p, "appid")?;
            let bot = bot_of(ctx, appid)?;
            let token = bot.access_token().await.map_err(err)?;
            let kind = p["kind"].as_i64().unwrap_or(0) as i32;
            let openid = req_str(p, "openid")?;
            let content = req_str(p, "content")?;
            // 账号的 Markdown 开关关掉时降级纯文本（群看 md_group，频道看 md_guild）
            let md_on = if kind == 1 { bot.md_guild } else { bot.md_group };
            let r = if md_on {
                api::send_markdown(
                    &ctx.http,
                    &token,
                    bot.api_base(),
                    kind,
                    openid,
                    content,
                    send_opts(p),
                )
                .await
            } else {
                api::send_text(
                    &ctx.http,
                    &token,
                    bot.api_base(),
                    kind,
                    openid,
                    content,
                    send_opts(p),
                )
                .await
            };
            log_send(db, appid, openid, p, content, &r);
            r.map_err(err)
        }
        "send_media" => {
            let appid = req_i64(p, "appid")?;
            let bot = bot_of(ctx, appid)?;
            let token = bot.access_token().await.map_err(err)?;
            let openid = req_str(p, "openid")?;
            let url = req_str(p, "url")?;
            let r = api::send_media(
                &ctx.http,
                &token,
                bot.api_base(),
                p["kind"].as_i64().unwrap_or(0) as i32,
                openid,
                p["file_type"].as_i64().unwrap_or(1) as i32,
                url,
                send_opts(p),
            )
            .await;
            log_send(db, appid, openid, p, url, &r);
            r.map_err(err)
        }
        "recall" => {
            let bot = bot_of(ctx, req_i64(p, "appid")?)?;
            let token = bot.access_token().await.map_err(err)?;
            api::recall(
                &ctx.http,
                &token,
                bot.api_base(),
                p["kind"].as_i64().unwrap_or(0) as i32,
                req_str(p, "openid")?,
                req_str(p, "msg_id")?,
            )
            .await
            .map(|_| Value::Null)
            .map_err(err)
        }

        // ---------- 定时任务 ----------
        "add_timer" => {
            let delay = p["delay_secs"].as_i64().unwrap_or(60).max(1);
            let repeat = p["repeat_secs"].as_i64().unwrap_or(0).max(0);
            let t = TimerRow {
                id: 0,
                plugin_id: ctx.plugin.clone(),
                remark: p["remark"].as_str().unwrap_or_default().to_string(),
                next_run: chrono::Utc::now().timestamp() + delay,
                repeat_secs: repeat,
                enabled: true,
            };
            db.insert_timer(&t).map(|id| json!(id)).map_err(err)
        }
        "cancel_timer" => db
            .delete_timer(req_i64(p, "timer_id")?)
            .map(|_| Value::Null)
            .map_err(err),

        // ---------- 查询 ----------
        "bot_list" => Ok(json!(ctx
            .bots
            .iter()
            .map(|e| json!({
                "appid": *e.key(),
                "name": e.value().name(),
                "online": e.value().is_online(),
            }))
            .collect::<Vec<_>>())),
        "get_short_id" => db
            .short_id(req_i64(p, "appid")?, req_str(p, "user")?)
            .map(|n| json!(n))
            .map_err(err),
        "get_openid" => db
            .openid_of(req_i64(p, "appid")?, req_i64(p, "short_id")?)
            .map(|v| json!(v))
            .map_err(err),

        // ---------- 频道域管理/查询（QQ 群无官方群管 API） ----------
        "guild_info" => {
            let bot = bot_of(ctx, req_i64(p, "appid")?)?;
            let token = bot.access_token().await.map_err(err)?;
            api::guild_info(&ctx.http, &token, bot.api_base(), req_str(p, "guild_id")?)
                .await
                .map_err(err)
        }
        "get_member_list" => {
            let bot = bot_of(ctx, req_i64(p, "appid")?)?;
            let token = bot.access_token().await.map_err(err)?;
            api::guild_member_list(
                &ctx.http,
                &token,
                bot.api_base(),
                req_str(p, "guild_id")?,
                p["limit"].as_i64().unwrap_or(100).clamp(1, 400),
                p["after"].as_str().unwrap_or("0"),
            )
            .await
            .map_err(err)
        }
        "get_member_info" => {
            let bot = bot_of(ctx, req_i64(p, "appid")?)?;
            let token = bot.access_token().await.map_err(err)?;
            api::guild_member_info(
                &ctx.http,
                &token,
                bot.api_base(),
                req_str(p, "guild_id")?,
                req_str(p, "user_id")?,
            )
            .await
            .map_err(err)
        }
        "kick_member" => {
            let bot = bot_of(ctx, req_i64(p, "appid")?)?;
            let token = bot.access_token().await.map_err(err)?;
            api::guild_kick(
                &ctx.http,
                &token,
                bot.api_base(),
                req_str(p, "guild_id")?,
                req_str(p, "user_id")?,
                p["delete_msg_days"].as_i64().unwrap_or(0).clamp(0, 7),
            )
            .await
            .map(|_| Value::Null)
            .map_err(err)
        }

        // ---------- Markdown 图片素材（qqbot-media:// 引用，media 权限） ----------
        "material_list" => {
            let bot = bot_of(ctx, req_i64(p, "appid")?)?;
            let token = bot.access_token().await.map_err(err)?;
            api::material_list(
                &ctx.http,
                &token,
                bot.appid,
                p["cursor"].as_str().unwrap_or_default(),
            )
            .await
            .map_err(err)
        }
        "material_upload" => {
            let bot = bot_of(ctx, req_i64(p, "appid")?)?;
            let token = bot.access_token().await.map_err(err)?;
            api::material_upload(
                &ctx.http,
                &token,
                bot.appid,
                std::path::Path::new(req_str(p, "path")?),
            )
            .await
            .map_err(err)
        }
        "material_delete" => {
            let bot = bot_of(ctx, req_i64(p, "appid")?)?;
            let token = bot.access_token().await.map_err(err)?;
            api::material_delete(&ctx.http, &token, bot.appid, req_str(p, "image_id")?)
                .await
                .map(|_| Value::Null)
                .map_err(err)
        }

        // ---------- 通用 HTTP 逃生舱 ----------
        "http_request" => http_request(ctx, p).await,

        _ => Err(format!("unknown api {method}")),
    }
}

/// 发送结果落日志：成功 Send / 失败 Error；raw 存 请求参数+API响应 完整 JSON 供调试。
fn log_send(
    db: &Database,
    appid: i64,
    openid: &str,
    params: &Value,
    content: &str,
    r: &Result<Value, anyhow::Error>,
) {
    let raw = serde_json::to_string(&serde_json::json!({
        "request": params,
        "response": match r { Ok(v) => v.clone(), Err(e) => Value::String(format!("{e:#}")) },
    }))
    .unwrap_or_default();
    let _ = db.insert_log(&LogRow {
        id: 0,
        ts: chrono::Utc::now().timestamp_millis(),
        appid,
        kind: if r.is_ok() { "Send".into() } else { "Error".into() },
        group_id: openid.to_string(),
        user: String::new(),
        content: match r {
            Ok(_) => content.chars().take(200).collect(),
            Err(e) => format!("发送失败: {e:#}"),
        },
        raw,
    });
}

/// 插件发送参数 → SendOpts（msg_id / keyboard / recall_after_secs）。
fn send_opts(p: &Value) -> api::SendOpts {
    api::SendOpts {
        msg_id: p["msg_id"].as_str().map(str::to_string),
        keyboard: if p["keyboard"].is_object() {
            Some(p["keyboard"].clone())
        } else {
            None
        },
        recall_after_secs: p["recall_after_secs"].as_i64().unwrap_or(0),
    }
}

fn bot_of(ctx: &ApiCtx, appid: i64) -> Result<Arc<BotClient>, String> {
    ctx.bots
        .get(&appid)
        .map(|b| b.clone())
        .ok_or_else(|| format!("bot {appid} not running"))
}

async fn http_request(ctx: &ApiCtx, p: &Value) -> Result<Value, String> {
    let url = req_str(p, "url")?;
    let method = p["method"].as_str().unwrap_or("GET").to_uppercase();
    let m = match method.as_str() {
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        "PATCH" => reqwest::Method::PATCH,
        _ => reqwest::Method::GET,
    };
    let mut req = ctx.http.request(m, url);
    if let Some(h) = p["headers"].as_object() {
        for (k, v) in h {
            if let Some(vs) = v.as_str() {
                req = req.header(k.as_str(), vs);
            }
        }
    }
    if let Some(body) = p["body"].as_str() {
        req = req.body(body.to_string());
    }
    let resp = req.send().await.map_err(err)?;
    let status = resp.status().as_u16();
    let body = resp.text().await.map_err(err)?;
    Ok(json!({ "status": status, "body": body }))
}
