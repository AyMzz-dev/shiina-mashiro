//! MCP 调试服务（Streamable HTTP 传输）：设置 `mcp_listen` + `mcp_token` 启用。
//!
//! AI 客户端（Claude / Windsurf / Cline 等）配置：
//!   { "mcpServers": { "mashiro": {
//!       "url": "http://127.0.0.1:8465/mcp",
//!       "headers": { "Authorization": "Bearer <mcp_token>" } } } }
//!
//! 连接后模型可直接读写插件文件、热重载、触发 handler、看日志、发测试消息，
//! 即「远程调插件」闭环。鉴权：Authorization: Bearer <mcp_token>，不匹配一律 401。

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::post;
use axum::Router;
use dashmap::DashMap;
use serde_json::{json, Value};

use crate::bot::api;
use crate::bot::client::BotClient;
use crate::plugin::manager::PluginManager;
use crate::storage::db::Database;

pub struct McpDeps {
    pub db: Arc<Database>,
    pub plugins: Arc<PluginManager>,
    pub bots: Arc<DashMap<i64, Arc<BotClient>>>,
    pub http: reqwest::Client,
}

struct Ctx {
    deps: McpDeps,
    token: String,
}

/// 开箱即用：`mcp_listen` 留空自动绑 `127.0.0.1:8465` 并写回设置（UI 反映真实值）；
/// 填 `off` 显式关闭；`mcp_token` 缺省时自动生成写回设置。
pub fn start(deps: McpDeps) {
    let mut addr = deps.db.get_setting("mcp_listen").unwrap_or_default();
    if addr.trim().eq_ignore_ascii_case("off") {
        return;
    }
    if addr.trim().is_empty() {
        addr = "127.0.0.1:8465".to_string();
        let _ = deps.db.set_setting("mcp_listen", &addr);
    }
    let mut token = deps.db.get_setting("mcp_token").unwrap_or_default();
    if token.trim().is_empty() {
        let raw: [u8; 24] = rand::random();
        token = hex::encode(raw);
        let _ = deps.db.set_setting("mcp_token", &token);
        tracing::info!("mcp: generated access token (见设置→网络接入)");
    }
    tokio::spawn(async move {
        let ctx = Arc::new(Ctx { deps, token });
        let app = Router::new()
            .route("/mcp", post(handle).get(no_sse).delete(no_sse))
            .with_state(ctx);
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(l) => {
                tracing::info!("mcp listening on {addr} → /mcp (Bearer 鉴权)");
                if let Err(e) = axum::serve(l, app).await {
                    tracing::error!("mcp serve: {e}");
                }
            }
            Err(e) => tracing::error!("mcp bind {addr}: {e}"),
        }
    });
}

async fn no_sse() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}

/// 等长逐字节异或比较，避免 Bearer token 早退时间侧信道。
fn token_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).fold(0u8, |x, (p, q)| x | (p ^ q)) == 0
}

// ---------- JSON-RPC ----------

fn rpc_ok(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_err(id: &Value, code: i64, msg: impl Into<String>) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": msg.into() } })
}

async fn handle(
    State(ctx): State<Arc<Ctx>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    // Bearer 鉴权
    let authed = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
        })
        .map(|t| token_eq(t.trim(), &ctx.token))
        .unwrap_or(false);
    if !authed {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "invalid or missing Bearer token" })),
        )
            .into_response();
    }

    match body {
        Value::Array(batch) => {
            let mut out = Vec::new();
            for msg in batch {
                if let Some(r) = process(&ctx, msg).await {
                    out.push(r);
                }
            }
            if out.is_empty() {
                StatusCode::ACCEPTED.into_response()
            } else {
                Json(json!(out)).into_response()
            }
        }
        msg => match process(&ctx, msg).await {
            Some(r) => Json(r).into_response(),
            None => StatusCode::ACCEPTED.into_response(),
        },
    }
}

/// 处理单条 JSON-RPC 消息；通知（无 id）返回 None。
async fn process(ctx: &Arc<Ctx>, msg: Value) -> Option<Value> {
    let method = msg["method"].as_str().unwrap_or("");
    if method.is_empty() {
        return Some(rpc_err(&Value::Null, -32600, "invalid request"));
    }
    let Some(id) = msg.get("id").cloned() else {
        return None; // notification
    };
    let params = &msg["params"];
    Some(match method {
        "initialize" => rpc_ok(
            &id,
            json!({
                "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "shiina-mashiro", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "椎名真白插件调试接口：读写插件文件、热重载、触发 handler、查日志、发测试消息。",
            }),
        ),
        "ping" => rpc_ok(&id, json!({})),
        "tools/list" => rpc_ok(&id, json!({ "tools": tools() })),
        "tools/call" => call_tool(ctx, &id, params).await,
        _ => rpc_err(&id, -32601, format!("method not found: {method}")),
    })
}

// ---------- 工具 ----------

fn tool_text(v: Value) -> Value {
    json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default() }] })
}

fn tool_err(msg: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": msg.into() }], "isError": true })
}

async fn call_tool(ctx: &Arc<Ctx>, id: &Value, params: &Value) -> Value {
    let name = params["name"].as_str().unwrap_or("");
    let args = &params["arguments"];
    let result = run_tool(ctx, name, args).await;
    rpc_ok(
        id,
        match result {
            Ok(v) => tool_text(v),
            Err(e) => tool_err(format!("{e:#}")),
        },
    )
}

fn need<'a>(args: &'a Value, k: &str) -> anyhow::Result<&'a str> {
    args[k]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("缺少参数 {k}"))
}

/// 扁平文件名白名单：插件目录一层平铺，拒绝路径穿越。
fn safe_name(f: &str) -> bool {
    !f.is_empty() && f.len() <= 96 && !f.contains(['/', '\\', ':']) && !f.contains("..")
}

fn plugins_root() -> PathBuf {
    crate::storage::db::default_dir().join("plugins")
}

fn plugin_dir(ctx: &Ctx, plugin_id: &str) -> anyhow::Result<PathBuf> {
    ctx.deps
        .db
        .list_plugins()?
        .into_iter()
        .find(|p| p.uuid == plugin_id)
        .map(|p| PathBuf::from(p.dir))
        .ok_or_else(|| anyhow::anyhow!("插件不存在: {plugin_id}"))
}

async fn run_tool(ctx: &Arc<Ctx>, name: &str, args: &Value) -> anyhow::Result<Value> {
    match name {
        "plugin_list" => {
            let rows = ctx.deps.db.list_plugins()?;
            let out: Vec<Value> = rows
                .into_iter()
                .map(|p| {
                    let cfg: Value = serde_json::from_str(&p.config).unwrap_or_else(|_| json!({}));
                    json!({
                        "id": p.uuid, "name": p.name, "runtime": p.runtime,
                        "enabled": p.enabled, "loaded": ctx.deps.plugins.is_loaded(&p.uuid),
                        "dir": p.dir,
                        "handlers": ctx.deps.plugins.handlers_of(&p.uuid),
                        "permissions": cfg["permissions"],
                        "accounts": cfg["accounts"],
                        "version": cfg["version"],
                    })
                })
                .collect();
            Ok(json!({ "plugins": out }))
        }

        "plugin_read" => {
            let pid = need(args, "plugin")?;
            let dir = plugin_dir(ctx, pid)?;
            match args["file"].as_str() {
                None => {
                    let mut files: Vec<String> = std::fs::read_dir(&dir)?
                        .filter_map(|e| {
                            e.ok().and_then(|e| {
                                e.file_type()
                                    .ok()
                                    .filter(|t| t.is_file())
                                    .and_then(|_| e.file_name().into_string().ok())
                            })
                        })
                        .collect();
                    files.sort();
                    Ok(json!({ "dir": dir, "files": files }))
                }
                Some(f) => {
                    if !safe_name(f) {
                        anyhow::bail!("非法文件名");
                    }
                    let path = dir.join(f);
                    let content = std::fs::read_to_string(&path)?;
                    // 按字符截断：字节切片切在 UTF-8 多字节边界上会 panic
                    let (content, truncated) = if content.len() > 64 * 1024 {
                        (content.chars().take(64 * 1024).collect(), true)
                    } else {
                        (content, false)
                    };
                    Ok(json!({ "file": f, "content": content, "truncated": truncated }))
                }
            }
        }

        "plugin_write" => {
            let pid = need(args, "plugin")?;
            let f = need(args, "file")?;
            if !safe_name(f) {
                anyhow::bail!("非法文件名");
            }
            let content = need(args, "content")?;
            let dir = plugin_dir(ctx, pid)?;
            std::fs::write(dir.join(f), content)?;
            Ok(json!({ "written": f, "bytes": content.len(),
                       "hint": "改完用 plugin_reload 生效" }))
        }

        "plugin_install" => {
            let files = args["files"]
                .as_object()
                .ok_or_else(|| anyhow::anyhow!("files 必须是 {{文件名: 内容}} 对象"))?;
            let manifest_raw = files
                .get("plugin.json")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("files 缺少 plugin.json"))?;
            let manifest: Value = serde_json::from_str(manifest_raw)?;
            let pid = manifest["id"]
                .as_str()
                .filter(|s| {
                    !s.is_empty()
                        && s.len() <= 64
                        && s.chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                })
                .ok_or_else(|| anyhow::anyhow!("plugin.json 缺少合法 id（字母数字-_）"))?;
            let dir = plugins_root().join(pid);
            std::fs::create_dir_all(&dir)?;
            for (f, c) in files {
                if !safe_name(f) {
                    anyhow::bail!("非法文件名: {f}");
                }
                std::fs::write(dir.join(f), c.as_str().unwrap_or_default())?;
            }
            ctx.deps.plugins.install_and_register(dir).await
        }

        "plugin_reload" => {
            let pid = need(args, "plugin")?;
            let row = ctx
                .deps
                .db
                .list_plugins()?
                .into_iter()
                .find(|p| p.uuid == pid)
                .ok_or_else(|| anyhow::anyhow!("插件不存在"))?;
            ctx.deps.plugins.unload(pid).await?;
            if row.enabled {
                ctx.deps
                    .plugins
                    .install_and_register(PathBuf::from(row.dir))
                    .await
            } else {
                Ok(json!({ "reloaded": false, "reason": "插件处于停用状态" }))
            }
        }

        "plugin_set_enabled" => {
            let pid = need(args, "plugin")?;
            let enabled = args["enabled"].as_bool().unwrap_or(true);
            let row = ctx
                .deps
                .db
                .list_plugins()?
                .into_iter()
                .find(|p| p.uuid == pid)
                .ok_or_else(|| anyhow::anyhow!("插件不存在"))?;
            if enabled {
                if !ctx.deps.plugins.is_loaded(pid) {
                    ctx.deps.plugins.load(PathBuf::from(row.dir)).await?;
                }
            } else {
                ctx.deps.plugins.unload(pid).await?;
            }
            ctx.deps.db.set_plugin_enabled(pid, enabled)?;
            Ok(json!({ "id": pid, "enabled": enabled }))
        }

        "plugin_uninstall" => {
            let pid = need(args, "plugin")?;
            ctx.deps.plugins.unload(pid).await?;
            ctx.deps.db.remove_plugin(pid)?;
            Ok(json!({ "uninstalled": pid, "note": "插件目录保留在 data/plugins/，可手动删除" }))
        }

        "plugin_invoke" => {
            let pid = need(args, "plugin")?;
            let handler = need(args, "handler")?;
            let event = match &args["event"] {
                v if v.is_null() => json!({ "type": "debug", "source": "mcp" }),
                v => v.clone(),
            };
            ctx.deps.plugins.invoke_raw(pid, handler, event).await?;
            Ok(json!({ "invoked": handler, "plugin": pid,
                       "hint": "handler 为 fire-and-forget；结果看 logs_query" }))
        }

        "logs_query" => {
            let kinds: Vec<String> = args["kinds"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let appid = args["appid"].as_i64().unwrap_or(0);
            let limit = args["limit"].as_i64().unwrap_or(50).clamp(1, 500);
            let rows = ctx.deps.db.list_logs(&kinds, appid, limit)?;
            Ok(json!({ "logs": rows }))
        }

        "timers_list" => {
            let filter = args["plugin"].as_str().unwrap_or("");
            let rows: Vec<_> = ctx
                .deps
                .db
                .list_timers()?
                .into_iter()
                .filter(|t| filter.is_empty() || t.plugin_id == filter)
                .collect();
            Ok(json!({ "timers": rows }))
        }

        "kv_list" => {
            let pid = need(args, "plugin")?;
            let pairs = ctx.deps.db.kv_list(pid)?;
            let map: serde_json::Map<String, Value> = pairs
                .into_iter()
                .map(|(k, v)| (k, Value::String(v)))
                .collect();
            Ok(json!({ "plugin": pid, "kv": map }))
        }

        "kv_del" => {
            let pid = need(args, "plugin")?;
            let key = need(args, "key")?;
            ctx.deps.db.kv_del(pid, key)?;
            Ok(json!({ "deleted": key }))
        }

        "send_message" => {
            let appid = args["appid"]
                .as_i64()
                .ok_or_else(|| anyhow::anyhow!("缺少参数 appid"))?;
            let target = need(args, "target")?;
            let text = need(args, "text")?;
            let kind = args["kind"].as_i64().unwrap_or(0) as i32;
            let markdown = args["markdown"].as_bool().unwrap_or(false);
            let msg_id = args["msg_id"].as_str().map(str::to_string);
            let bot = ctx
                .deps
                .bots
                .get(&appid)
                .map(|b| b.clone())
                .ok_or_else(|| anyhow::anyhow!("机器人不在线: {appid}"))?;
            let token = bot.access_token().await?;
            let opts = api::SendOpts {
                msg_id: msg_id.filter(|s| !s.is_empty()),
                ..Default::default()
            };
            let r = if markdown {
                api::send_markdown(
                    &ctx.deps.http,
                    &token,
                    bot.api_base(),
                    kind,
                    target,
                    text,
                    opts,
                )
                .await
            } else {
                api::send_text(
                    &ctx.deps.http,
                    &token,
                    bot.api_base(),
                    kind,
                    target,
                    text,
                    opts,
                )
                .await
            }?;
            Ok(json!({ "sent": true, "response": r }))
        }

        "group_bot_state" => {
            let appid = args["appid"]
                .as_i64()
                .ok_or_else(|| anyhow::anyhow!("缺少参数 appid"))?;
            let gid = need(args, "group_openid")?;
            let bot = ctx
                .deps
                .bots
                .get(&appid)
                .map(|b| b.clone())
                .ok_or_else(|| anyhow::anyhow!("机器人不在线: {appid}"))?;
            let token = bot.access_token().await?;
            api::group_bot_state(&ctx.deps.http, &token, bot.api_base(), gid).await
        }

        _ => anyhow::bail!("unknown tool: {name}"),
    }
}

fn tools() -> Value {
    json!([
        { "name": "plugin_list", "description": "列出全部插件：id/名称/runtime/启用/已加载/handler/权限/账号授权",
          "inputSchema": { "type": "object", "properties": {} } },
        { "name": "plugin_read", "description": "读插件文件。不带 file 返回文件清单；带 file 返回内容（64KB 截断）",
          "inputSchema": { "type": "object", "properties": {
              "plugin": { "type": "string", "description": "插件 id" },
              "file": { "type": "string", "description": "文件名，如 main.js / plugin.json" } },
              "required": ["plugin"] } },
        { "name": "plugin_write", "description": "写入插件文件（扁平文件名）。改完调 plugin_reload 生效",
          "inputSchema": { "type": "object", "properties": {
              "plugin": { "type": "string" },
              "file": { "type": "string" },
              "content": { "type": "string" } },
              "required": ["plugin", "file", "content"] } },
        { "name": "plugin_install", "description": "从文件集安装新插件：files 须含 plugin.json（含 id/runtime/entry/permissions）与入口文件，装完立即加载",
          "inputSchema": { "type": "object", "properties": {
              "files": { "type": "object", "description": "{ \"plugin.json\": \"...\", \"main.js\": \"...\" }" } },
              "required": ["files"] } },
        { "name": "plugin_reload", "description": "热重载插件：卸载→重新加载→返回注册的 handler 列表或报错",
          "inputSchema": { "type": "object", "properties": {
              "plugin": { "type": "string" } }, "required": ["plugin"] } },
        { "name": "plugin_set_enabled", "description": "启用/停用插件",
          "inputSchema": { "type": "object", "properties": {
              "plugin": { "type": "string" },
              "enabled": { "type": "boolean" } }, "required": ["plugin", "enabled"] } },
        { "name": "plugin_uninstall", "description": "卸载插件（从管理表移除，目录保留在 data/plugins/）",
          "inputSchema": { "type": "object", "properties": {
              "plugin": { "type": "string" } }, "required": ["plugin"] } },
        { "name": "plugin_invoke", "description": "直接触发插件 handler 调试：handler 形如 \"cmd:签到\" 或 \"event:timer\"；event 可自定义模拟事件 JSON",
          "inputSchema": { "type": "object", "properties": {
              "plugin": { "type": "string" },
              "handler": { "type": "string" },
              "event": { "type": "object", "description": "模拟事件负载，缺省为 {type:\"debug\"}" } },
              "required": ["plugin", "handler"] } },
        { "name": "logs_query", "description": "查最近日志：kinds 过滤（Group/C2C/Send/Error...），appid 过滤账号，limit 默认 50",
          "inputSchema": { "type": "object", "properties": {
              "kinds": { "type": "array", "items": { "type": "string" } },
              "appid": { "type": "integer" },
              "limit": { "type": "integer" } } } },
        { "name": "timers_list", "description": "列出插件注册的定时任务，可按 plugin 过滤",
          "inputSchema": { "type": "object", "properties": {
              "plugin": { "type": "string" } } } },
        { "name": "kv_list", "description": "列出插件私有 KV 全部键值",
          "inputSchema": { "type": "object", "properties": {
              "plugin": { "type": "string" } }, "required": ["plugin"] } },
        { "name": "kv_del", "description": "删除插件 KV 键",
          "inputSchema": { "type": "object", "properties": {
              "plugin": { "type": "string" },
              "key": { "type": "string" } }, "required": ["plugin", "key"] } },
        { "name": "send_message", "description": "以机器人身份发测试消息：kind 0群/1频道/2私聊(默认0)，target 为 openid，markdown=true 走 markdown",
          "inputSchema": { "type": "object", "properties": {
              "appid": { "type": "integer" },
              "target": { "type": "string", "description": "群/频道/用户 openid" },
              "text": { "type": "string" },
              "kind": { "type": "integer", "description": "0群 1频道 2私聊，默认0" },
              "markdown": { "type": "boolean" },
              "msg_id": { "type": "string", "description": "被动回复引用的消息 id" } },
              "required": ["appid", "target", "text"] } },
        { "name": "group_bot_state", "description": "查机器人群内授权状态：主动发言/消息范围/群角色",
          "inputSchema": { "type": "object", "properties": {
              "appid": { "type": "integer" },
              "group_openid": { "type": "string" } },
              "required": ["appid", "group_openid"] } },
    ])
}
