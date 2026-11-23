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
        "send_message" | "send_markdown" | "send_card" | "send_stream" | "interaction_respond" => {
            "send"
        }
        "send_media" | "send_media_file" | "material_list" | "material_upload"
        | "material_delete" => "media",
        "recall" => "recall",
        "add_timer" | "cancel_timer" => "timer",
        "guild_info"
        | "get_member_list"
        | "get_member_info"
        | "kick_member"
        | "group_info"
        | "group_members"
        | "group_member_info"
        | "group_blacklist"
        | "group_blacklist_op"
        | "group_remove_members"
        | "group_mute_setting"
        | "group_mute_members"
        | "group_join_requests"
        | "group_approve_join"
        | "strategy_list"
        | "strategy_create"
        | "strategy_patch"
        | "strategy_delete"
        | "strategy_execute"
        | "strategy_whitelist"
        | "menu_get"
        | "menu_put"
        | "panel_list"
        | "panel_create"
        | "panel_get"
        | "panel_update"
        | "panel_delete"
        | "panel_target"
        | "user_info"
        | "generate_url_link" => "group_admin",
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
                content: match p["level"].as_str() {
                    Some(l) if l != "info" => {
                        format!("[{l}] {}", p["msg"].as_str().unwrap_or_default())
                    }
                    _ => p["msg"].as_str().unwrap_or_default().to_string(),
                },
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
            let md_on = if kind == 1 {
                bot.md_guild
            } else {
                bot.md_group
            };
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
            .delete_timer_of(&ctx.plugin, req_i64(p, "timer_id")?)
            .and_then(|n| {
                if n == 0 {
                    Err(rusqlite::Error::QueryReturnedNoRows)
                } else {
                    Ok(())
                }
            })
            .map(|_| Value::Null)
            .map_err(|_| "timer 不存在或不属于本插件".to_string()),

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

        // ---------- 图文卡片 / 流式 / 本地文件 / 互动应答 ----------
        "send_card" => {
            let appid = req_i64(p, "appid")?;
            let (token, base) = token_of(ctx, appid).await?;
            let openid = req_str(p, "openid")?;
            let kind = p["kind"].as_i64().unwrap_or(0) as i32;
            let mut body = json!({
                "msg_type": 8,
                "card": {
                    "type": p["card_type"].as_str().unwrap_or("tuwen"),
                    "content": {
                        "title": req_str(p, "title")?,
                        "description": p["desc"].as_str().unwrap_or_default(),
                        "url": p["url"].as_str().unwrap_or_default(),
                        "pic_url": p["pic_url"].as_str().unwrap_or_default(),
                    }
                }
            });
            if let Some(o) = body.as_object_mut() {
                for (k, v) in [
                    ("msg_id", p["msg_id"].clone()),
                    ("event_id", p["event_id"].clone()),
                    ("msg_seq", p["msg_seq"].clone()),
                    ("keyboard", p["keyboard"].clone()),
                ] {
                    if !v.is_null() {
                        o.insert(k.into(), v);
                    }
                }
            }
            let r = api::post_send(&ctx.http, &token, &base, kind, openid, &body).await;
            log_send(
                db,
                appid,
                openid,
                p,
                &format!("card:{}", p["title"].as_str().unwrap_or_default()),
                &r,
            );
            r.map_err(err)
        }
        "send_stream" => {
            // 仅单聊；params 去掉 appid/openid 后整体作为流式分块体透传
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            let openid = req_str(p, "openid")?;
            let mut body = p.clone();
            if let Some(o) = body.as_object_mut() {
                o.remove("appid");
                o.remove("openid");
            }
            api::send_stream(&ctx.http, &token, &base, openid, &body)
                .await
                .map_err(err)
        }
        "send_media_file" => {
            let appid = req_i64(p, "appid")?;
            let (token, base) = token_of(ctx, appid).await?;
            let openid = req_str(p, "openid")?;
            let r = api::send_media_file(
                &ctx.http,
                &token,
                &base,
                p["kind"].as_i64().unwrap_or(0) as i32,
                openid,
                p["file_type"].as_i64().unwrap_or(1) as i32,
                std::path::Path::new(req_str(p, "path")?),
                send_opts(p),
            )
            .await;
            log_send(
                db,
                appid,
                openid,
                p,
                p["path"].as_str().unwrap_or_default(),
                &r,
            );
            r.map_err(err)
        }
        "interaction_respond" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            api::interaction_respond(
                &ctx.http,
                &token,
                &base,
                req_str(p, "interaction_id")?,
                p["code"].as_i64().unwrap_or(0),
            )
            .await
            .map_err(err)
        }

        // ---------- QQ 群管理（v2/groups，group_admin 权限） ----------
        "group_info" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            api::group_info(&ctx.http, &token, &base, req_str(p, "group_openid")?)
                .await
                .map_err(err)
        }
        "group_members" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            api::group_members(
                &ctx.http,
                &token,
                &base,
                req_str(p, "group_openid")?,
                p["cursor"].as_str().unwrap_or_default(),
            )
            .await
            .map_err(err)
        }
        "group_member_info" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            api::group_member(
                &ctx.http,
                &token,
                &base,
                req_str(p, "group_openid")?,
                req_str(p, "member_openid")?,
            )
            .await
            .map_err(err)
        }
        "group_blacklist" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            api::group_blacklist(
                &ctx.http,
                &token,
                &base,
                req_str(p, "group_openid")?,
                p["cursor"].as_str().unwrap_or_default(),
                p["limit"].as_i64().unwrap_or(20).clamp(1, 50),
            )
            .await
            .map_err(err)
        }
        "group_blacklist_op" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            api::group_blacklist_op(
                &ctx.http,
                &token,
                &base,
                req_str(p, "group_openid")?,
                req_str(p, "op")?,
                &str_list(p, "member_openids")?,
            )
            .await
            .map_err(err)
        }
        "group_remove_members" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            api::group_remove_members(
                &ctx.http,
                &token,
                &base,
                req_str(p, "group_openid")?,
                &str_list(p, "member_openids")?,
            )
            .await
            .map_err(err)
        }
        "group_mute_setting" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            api::group_mute_setting(&ctx.http, &token, &base, req_str(p, "group_openid")?)
                .await
                .map_err(err)
        }
        "group_mute_members" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            let members = p["members"]
                .as_array()
                .ok_or_else(|| "param members required".to_string())?;
            api::group_mute_members(
                &ctx.http,
                &token,
                &base,
                req_str(p, "group_openid")?,
                &json!(members),
            )
            .await
            .map_err(err)
        }
        "group_join_requests" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            api::group_join_requests(
                &ctx.http,
                &token,
                &base,
                req_str(p, "group_openid")?,
                p["cursor"].as_str().unwrap_or_default(),
                p["limit"].as_i64().unwrap_or(20).clamp(1, 50),
            )
            .await
            .map_err(err)
        }
        "group_approve_join" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            api::group_approve_join(
                &ctx.http,
                &token,
                &base,
                req_str(p, "group_openid")?,
                req_str(p, "member_openid")?,
                req_str(p, "op")?,
                p["join_request_id"].as_str(),
                p["reject_reason"].as_str(),
                p["add_to_member_blacklist"].as_bool().unwrap_or(false),
            )
            .await
            .map_err(err)
        }

        // ---------- 入群自动审批策略 ----------
        "strategy_list" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            api::join_strategy_list(
                &ctx.http,
                &token,
                &base,
                p["cursor"].as_str().unwrap_or_default(),
                p["limit"].as_i64().unwrap_or(20).clamp(1, 50),
            )
            .await
            .map_err(err)
        }
        "strategy_create" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            api::join_strategy_create(&ctx.http, &token, &base, &strip_meta(p))
                .await
                .map_err(err)
        }
        "strategy_patch" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            api::join_strategy_patch(
                &ctx.http,
                &token,
                &base,
                req_str(p, "strategy_id")?,
                &strip_meta(p),
            )
            .await
            .map_err(err)
        }
        "strategy_delete" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            api::join_strategy_delete(&ctx.http, &token, &base, req_str(p, "strategy_id")?)
                .await
                .map(|_| Value::Null)
                .map_err(err)
        }
        "strategy_execute" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            api::join_strategy_execute(&ctx.http, &token, &base, req_str(p, "strategy_id")?)
                .await
                .map_err(err)
        }
        "strategy_whitelist" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            api::join_strategy_whitelist(
                &ctx.http,
                &token,
                &base,
                req_str(p, "strategy_id")?,
                req_str(p, "op")?,
                &str_list(p, "whitelist_users")?,
            )
            .await
            .map_err(err)
        }

        // ---------- 菜单 / 指令面板 ----------
        "menu_get" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            api::menu_get(&ctx.http, &token, &base).await.map_err(err)
        }
        "menu_put" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            let menu = p["menu"]
                .as_object()
                .ok_or_else(|| "param menu required".to_string())?;
            api::menu_put(&ctx.http, &token, &base, &json!(menu))
                .await
                .map_err(err)
        }
        "panel_list" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            api::panels_list(
                &ctx.http,
                &token,
                &base,
                req_str(p, "scope")?,
                p["cursor"].as_str().unwrap_or_default(),
                p["limit"].as_i64().unwrap_or(20).clamp(1, 50),
            )
            .await
            .map_err(err)
        }
        "panel_create" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            api::panel_create(&ctx.http, &token, &base, &strip_meta(p))
                .await
                .map_err(err)
        }
        "panel_get" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            api::panel_get(&ctx.http, &token, &base, req_str(p, "panel_id")?)
                .await
                .map_err(err)
        }
        "panel_update" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            api::panel_update(
                &ctx.http,
                &token,
                &base,
                req_str(p, "panel_id")?,
                &strip_meta(p),
            )
            .await
            .map_err(err)
        }
        "panel_delete" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            api::panel_delete(&ctx.http, &token, &base, req_str(p, "panel_id")?)
                .await
                .map(|_| Value::Null)
                .map_err(err)
        }
        "panel_target" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            api::panel_target(
                &ctx.http,
                &token,
                &base,
                req_str(p, "panel_id")?,
                req_str(p, "op")?,
                &str_list(p, "user_openids").unwrap_or_default(),
                &str_list(p, "group_openids").unwrap_or_default(),
            )
            .await
            .map_err(err)
        }

        // ---------- 用户资料 / 分享链接 ----------
        "user_info" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            api::user_info(&ctx.http, &token, &base, req_str(p, "user_openid")?)
                .await
                .map_err(err)
        }
        "generate_url_link" => {
            let (token, base) = token_of(ctx, req_i64(p, "appid")?).await?;
            api::generate_url_link(
                &ctx.http,
                &token,
                &base,
                p["callback_data"].as_str().unwrap_or_default(),
            )
            .await
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
        kind: if r.is_ok() {
            "Send".into()
        } else {
            "Error".into()
        },
        group_id: openid.to_string(),
        user: String::new(),
        content: match r {
            Ok(_) => content.chars().take(200).collect(),
            Err(e) => format!("发送失败: {e:#}"),
        },
        raw,
    });
}

/// 插件发送参数 → SendOpts（msg_id / event_id / msg_seq / is_wakeup / keyboard / recall_after_secs）。
fn send_opts(p: &Value) -> api::SendOpts {
    api::SendOpts {
        msg_id: p["msg_id"].as_str().map(str::to_string),
        event_id: p["event_id"].as_str().map(str::to_string),
        msg_seq: p["msg_seq"].as_i64(),
        is_wakeup: p["is_wakeup"].as_bool().unwrap_or(false),
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

/// appid → (access_token, api_base)：新 API 方法的通用取号模板。
async fn token_of(ctx: &ApiCtx, appid: i64) -> Result<(String, String), String> {
    let bot = bot_of(ctx, appid)?;
    let token = bot.access_token().await.map_err(err)?;
    Ok((token, bot.api_base().to_string()))
}

/// params 的字符串数组字段 → Vec<String>；缺失/非数组视为参数错误。
fn str_list(p: &Value, k: &str) -> Result<Vec<String>, String> {
    p[k].as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .ok_or_else(|| format!("param {k} required"))
}

/// 剥掉 appid 及路径参数等元字段，剩余参数原样透传成 API 请求体（策略/面板 body 由插件直接给）。
fn strip_meta(p: &Value) -> Value {
    const META: &[&str] = &["appid", "strategy_id", "panel_id"];
    let mut v = p.clone();
    if let Some(o) = v.as_object_mut() {
        for k in META {
            o.remove(*k);
        }
    }
    v
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
