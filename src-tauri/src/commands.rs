use std::path::PathBuf;
use std::sync::Arc;

use mashiro_core::agent as agentmod;
use mashiro_core::bot::client::BotClient;
use mashiro_core::storage::models::{Account, LogRow, PluginRow};
use serde::Serialize;
use serde_json::json;
use tauri::{Emitter, State};

use crate::state::{AgentSession, AppState};

fn e2s(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// 插件 id 白名单：字母数字-_、≤64——会拼进 plugins_root() 路径，必须防穿越。
fn valid_plugin_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// 发送日志的 raw 字段：{request, response} 完整 JSON，日志页「查看 JSON」调试用。
fn send_raw(request: serde_json::Value, r: &Result<serde_json::Value, anyhow::Error>) -> String {
    serde_json::to_string(&json!({
        "request": request,
        "response": match r {
            Ok(v) => v.clone(),
            Err(e) => serde_json::Value::String(format!("{e:#}")),
        },
    }))
    .unwrap_or_default()
}

#[tauri::command]
pub fn greet() -> String {
    "椎名真白 ready".to_string()
}

// ---------- 账号 ----------

const SECRET_MASK: &str = "••••••";

#[tauri::command]
pub fn list_accounts(state: State<'_, AppState>) -> Result<Vec<Account>, String> {
    let mut accs = state.db.list_accounts().map_err(e2s)?;
    // 脱敏：前端只见掩码，真实 secret 不出 Rust 进程
    for a in &mut accs {
        a.secret = if a.secret.is_empty() {
            String::new()
        } else {
            SECRET_MASK.to_string()
        };
    }
    Ok(accs)
}

/// 新增/更新账号全量配置（appid+secret+连接方式+沙盒+Markdown+intents）。
/// secret 传掩码或空串 = 保留库内原值；新建账号必须给真 secret。
#[tauri::command]
pub fn save_account(state: State<'_, AppState>, mut account: Account) -> Result<(), String> {
    if account.appid <= 0 {
        return Err("appid 不能为空".to_string());
    }
    if account.secret == SECRET_MASK || account.secret.trim().is_empty() {
        if state.db.get_account(account.appid).map_err(e2s)?.is_none() {
            return Err("新账号必须提供 secret".to_string());
        }
        account.secret = String::new(); // 落库时保留原值
    }
    state.db.save_account(&account).map_err(e2s)
}

// ---------- 扫码绑定登录（对齐旧框架 QQBindLogin） ----------

/// 第一步：创建绑定任务 → {task_id, qr_url}，前端把 qr_url 渲染成二维码。
#[tauri::command]
pub async fn qr_bind_start(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    // 顺手清过期会话
    {
        let now = chrono::Utc::now().timestamp_millis();
        let mut tasks = state.bind_tasks.lock().unwrap();
        tasks.retain(|_, t| now - t.created_ms < mashiro_core::bot::bind::SESSION_TTL_MS);
    }
    let task = mashiro_core::bot::bind::create_task(&state.http)
        .await
        .map_err(e2s)?;
    let out = json!({ "task_id": task.task_id, "qr_url": task.qr_url });
    state
        .bind_tasks
        .lock()
        .unwrap()
        .insert(task.task_id.clone(), task);
    Ok(out)
}

/// 第二步：轮询扫码结果 → {status: waiting|expired|done, appid?, secret?}。
/// 完成后 secret 直接回前端表单回填（不写库，由用户确认后走 save_account）。
#[tauri::command]
pub async fn qr_bind_poll(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<serde_json::Value, String> {
    let task = {
        let tasks = state.bind_tasks.lock().unwrap();
        tasks.get(&task_id).cloned()
    };
    let Some(task) = task else {
        return Ok(json!({ "status": "expired" }));
    };
    if chrono::Utc::now().timestamp_millis() - task.created_ms
        > mashiro_core::bot::bind::SESSION_TTL_MS
    {
        state.bind_tasks.lock().unwrap().remove(&task_id);
        return Ok(json!({ "status": "expired" }));
    }
    match mashiro_core::bot::bind::poll(&state.http, &task)
        .await
        .map_err(e2s)?
    {
        mashiro_core::bot::bind::PollResult::Waiting => Ok(json!({ "status": "waiting" })),
        mashiro_core::bot::bind::PollResult::Expired => {
            state.bind_tasks.lock().unwrap().remove(&task_id);
            Ok(json!({ "status": "expired" }))
        }
        mashiro_core::bot::bind::PollResult::Done { appid, secret } => {
            state.bind_tasks.lock().unwrap().remove(&task_id);
            Ok(json!({ "status": "done", "appid": appid, "secret": secret }))
        }
    }
}

#[tauri::command]
pub async fn remove_account(state: State<'_, AppState>, appid: i64) -> Result<(), String> {
    if let Some((_, bot)) = state.bots.remove(&appid) {
        bot.stop().await;
    }
    state.db.remove_account(appid).map_err(e2s)
}

#[tauri::command]
pub async fn start_bot(state: State<'_, AppState>, appid: i64) -> Result<(), String> {
    if state.bots.contains_key(&appid) {
        return Ok(());
    }
    let acc = state
        .db
        .get_account(appid)
        .map_err(e2s)?
        .ok_or_else(|| "账号不存在".to_string())?;
    let client = BotClient::new(
        acc,
        state.http.clone(),
        Arc::clone(&state.db),
        Arc::clone(&state.dispatcher),
    );
    client.start().await.map_err(e2s)?;
    state.bots.insert(appid, client);
    Ok(())
}

#[tauri::command]
pub async fn stop_bot(state: State<'_, AppState>, appid: i64) -> Result<(), String> {
    if let Some((_, bot)) = state.bots.remove(&appid) {
        bot.stop().await;
    }
    Ok(())
}

#[derive(Serialize)]
pub struct BotInfo {
    pub appid: i64,
    pub name: String,
    pub avatar: String,
    pub online: bool,
}

#[tauri::command]
pub fn bot_status(state: State<'_, AppState>) -> Vec<BotInfo> {
    state
        .bots
        .iter()
        .map(|e| BotInfo {
            appid: *e.key(),
            name: e.value().name(),
            avatar: e.value().avatar(),
            online: e.value().is_online(),
        })
        .collect()
}

// ---------- 首页统计 ----------

#[derive(Serialize)]
pub struct Stats {
    pub accounts: i64,
    pub online: i64,
    pub msgs_today: i64,
    pub plugins_loaded: i64,
    pub recv_today: i64,
    pub sent_today: i64,
    /// 24 桶：[recv, send]，下标 = 小时
    pub hourly: [[i64; 2]; 24],
    /// 今日插件消息 TOP：(name, count)
    pub top_plugins: Vec<(String, i64)>,
}

#[tauri::command]
pub fn get_stats(state: State<'_, AppState>) -> Result<Stats, String> {
    let today = chrono::Local::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|t| t.and_local_timezone(chrono::Local).earliest())
        .map(|t| t.timestamp_millis())
        .unwrap_or(0);
    let mut hourly = [[0i64; 2]; 24];
    let (mut recv, mut sent) = (0i64, 0i64);
    for (h, r, s) in state.db.hourly_msgs(today).unwrap_or_default() {
        if (h as usize) < 24 {
            hourly[h as usize] = [r, s];
            recv += r;
            sent += s;
        }
    }
    Ok(Stats {
        accounts: state.db.list_accounts().map_err(e2s)?.len() as i64,
        online: state.bots.iter().filter(|b| b.is_online()).count() as i64,
        msgs_today: state.db.count_logs_since(today).unwrap_or(0),
        plugins_loaded: state.plugins.loaded_count() as i64,
        recv_today: recv,
        sent_today: sent,
        hourly,
        top_plugins: state.db.top_plugins(6).unwrap_or_default(),
    })
}

// ---------- 插件 ----------

#[tauri::command]
pub fn list_plugins(state: State<'_, AppState>) -> Result<Vec<PluginRow>, String> {
    state.db.list_plugins().map_err(e2s)
}

/// 插件已注册的 handler 清单（规则页挑指令用）。
#[tauri::command]
pub fn plugin_handlers(state: State<'_, AppState>, plugin_id: String) -> Vec<String> {
    state.plugins.handlers_of(&plugin_id)
}

/// 用户插件落地目录：<运行目录>/data/plugins（与 mashiro.db 同一数据根）。
fn plugins_root() -> PathBuf {
    mashiro_core::storage::db::default_dir().join("plugins")
}

async fn install_dir(state: &AppState, dir: PathBuf) -> Result<serde_json::Value, String> {
    state.plugins.install_and_register(dir).await.map_err(e2s)
}

#[tauri::command]
pub async fn install_plugin(
    state: State<'_, AppState>,
    dir: String,
) -> Result<serde_json::Value, String> {
    install_dir(&state, PathBuf::from(&dir)).await
}

/// 生成插件脚手架（plugin.json + 入口模板）→ 立即安装。
/// 也是「Agent 生成插件」的落点：Agent 产出文件 → scaffold/install 即上线。
#[tauri::command]
pub async fn scaffold_plugin(
    state: State<'_, AppState>,
    id: String,
    name: String,
    runtime: String,
) -> Result<serde_json::Value, String> {
    if !valid_plugin_id(&id) || !["node", "python"].contains(&runtime.as_str()) {
        return Err("id 必填且仅限字母数字-_（≤64），runtime 只能 node/python".to_string());
    }
    let dir = plugins_root().join(&id);
    if dir.join("plugin.json").exists() {
        return Err("插件 id 已存在".to_string());
    }
    std::fs::create_dir_all(&dir).map_err(e2s)?;

    let (entry, code) = match runtime.as_str() {
        "python" => (
            "main.py",
            r#"import mashiro

bot = mashiro.Bot()


@bot.command("ping")
def ping(ctx):
    ctx.reply("pong")


@bot.timer(3600, 3600, "每小时")
def tick(ctx):
    ctx.log("timer fired")
"#,
        ),
        _ => (
            "main.js",
            r#"const bot = require("mashiro");

bot.command("ping", async (ctx) => {
  await ctx.reply("pong");
});

bot.on("timer", async (ctx) => {
  await ctx.log("timer fired");
});
bot.timer(3600, 3600, "每小时");
"#,
        ),
    };
    let name = if name.is_empty() { id.clone() } else { name };
    std::fs::write(
        dir.join("plugin.json"),
        json!({
            "id": id,
            "name": name,
            "version": "0.1.0",
            "runtime": runtime,
            "entry": entry,
            "description": "",
            "author": "scaffold",
            "permissions": ["send", "kv", "timer", "http"],
        })
        .to_string(),
    )
    .map_err(e2s)?;
    std::fs::write(dir.join(entry), code).map_err(e2s)?;
    install_dir(&state, dir).await
}

#[tauri::command]
pub async fn set_plugin_enabled(
    state: State<'_, AppState>,
    uuid: String,
    enabled: bool,
) -> Result<(), String> {
    if enabled {
        let dir = state
            .db
            .list_plugins()
            .map_err(e2s)?
            .into_iter()
            .find(|p| p.uuid == uuid)
            .map(|p| p.dir)
            .ok_or_else(|| "插件不存在".to_string())?;
        if !state.plugins.is_loaded(&uuid) {
            state.plugins.load(PathBuf::from(dir)).await.map_err(e2s)?;
        }
    } else {
        state.plugins.unload(&uuid).await.map_err(e2s)?;
    }
    state.db.set_plugin_enabled(&uuid, enabled).map_err(e2s)
}

#[tauri::command]
pub async fn unload_plugin(state: State<'_, AppState>, uuid: String) -> Result<(), String> {
    state.plugins.unload(&uuid).await.map_err(e2s)
}

#[tauri::command]
pub async fn reload_plugin(state: State<'_, AppState>, uuid: String) -> Result<(), String> {
    let row = state
        .db
        .list_plugins()
        .map_err(e2s)?
        .into_iter()
        .find(|p| p.uuid == uuid)
        .ok_or_else(|| "插件不存在".to_string())?;
    state.plugins.unload(&uuid).await.map_err(e2s)?;
    if row.enabled {
        install_dir(&state, PathBuf::from(row.dir)).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn set_plugin_accounts(
    state: State<'_, AppState>,
    uuid: String,
    accounts: Vec<i64>,
) -> Result<(), String> {
    let row = state
        .db
        .list_plugins()
        .map_err(e2s)?
        .into_iter()
        .find(|p| p.uuid == uuid)
        .ok_or_else(|| "插件不存在".to_string())?;
    let mut config: serde_json::Value =
        serde_json::from_str(&row.config).unwrap_or_else(|_| json!({}));
    config["accounts"] = json!(accounts);
    state
        .db
        .update_plugin_config(&uuid, &config.to_string())
        .map_err(e2s)
}

#[tauri::command]
pub async fn open_path(path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer")
        .arg(&path)
        .spawn()
        .map_err(e2s)?;
    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg(&path)
        .spawn()
        .map_err(e2s)?;
    #[cfg(all(unix, not(target_os = "macos")))]
    std::process::Command::new("xdg-open")
        .arg(&path)
        .spawn()
        .map_err(e2s)?;
    Ok(())
}

#[tauri::command]
pub async fn remove_plugin(state: State<'_, AppState>, uuid: String) -> Result<(), String> {
    state.plugins.unload(&uuid).await.map_err(e2s)?;
    state.db.remove_plugin(&uuid).map_err(e2s)
}

// ---------- 设置 ----------

/// 检测解释器可执行性：跑 `path --version`，返回版本串或错误。
#[tauri::command]
pub async fn check_program(path: String) -> Result<String, String> {
    let program = if path.trim().is_empty() {
        return Err("路径为空".into());
    } else {
        path.trim().to_string()
    };
    let out = tokio::process::Command::new(&program)
        .arg("--version")
        .output()
        .await;
    match out {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let e = String::from_utf8_lossy(&o.stderr).trim().to_string();
            let v = if s.is_empty() { e } else { s };
            if v.is_empty() {
                Err("无版本输出".into())
            } else {
                Ok(v.lines().next().unwrap_or("").to_string())
            }
        }
        Err(e) => Err(format!("启动失败：{e}")),
    }
}

/// 数据目录（mashiro.db 所在）路径，设置页展示/打开用。
#[tauri::command]
pub fn data_dir() -> Result<String, String> {
    Ok(mashiro_core::storage::db::default_dir()
        .to_string_lossy()
        .to_string())
}



/// 粘贴的 PEM 内容落盘为 <data>/tls/webhook-{cert,key}.pem，返回路径供设置回填。
#[tauri::command]
pub fn write_pem_file(kind: String, content: String) -> Result<String, String> {
    let name = match kind.as_str() {
        "cert" => "webhook-cert.pem",
        "key" => "webhook-key.pem",
        _ => return Err("kind 只支持 cert / key".to_string()),
    };
    if content.trim().is_empty() {
        return Err("PEM 内容为空".to_string());
    }
    let dir = mashiro_core::storage::db::default_dir().join("tls");
    std::fs::create_dir_all(&dir).map_err(e2s)?;
    let p = dir.join(name);
    std::fs::write(&p, content).map_err(e2s)?;
    Ok(p.to_string_lossy().to_string())
}

#[tauri::command]
pub fn get_setting(state: State<'_, AppState>, key: String) -> Result<String, String> {
    state.db.get_setting(&key).map_err(e2s)
}

#[tauri::command]
pub fn set_setting(state: State<'_, AppState>, key: String, value: String) -> Result<(), String> {
    state.db.set_setting(&key, &value).map_err(e2s)
}

// ---------- 日志 ----------

#[tauri::command]
pub fn list_logs(
    state: State<'_, AppState>,
    kinds: Option<Vec<String>>,
    appid: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<LogRow>, String> {
    state
        .db
        .list_logs(
            &kinds.unwrap_or_default(),
            appid.unwrap_or(0),
            limit.unwrap_or(200),
        )
        .map_err(e2s)
}

#[tauri::command]
pub fn delete_log(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    state.db.delete_log(id).map_err(e2s)
}

#[tauri::command]
pub fn clear_logs(state: State<'_, AppState>) -> Result<(), String> {
    state.db.clear_logs().map_err(e2s)
}

/// 按 log_keep_days 立即清理过期日志，返回删除行数。
#[tauri::command]
pub fn purge_logs(state: State<'_, AppState>) -> Result<usize, String> {
    let days: i64 = state
        .db
        .get_setting("log_keep_days")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30)
        .clamp(1, 3650);
    let cutoff = chrono::Utc::now().timestamp_millis() - days * 86400_000;
    state.db.delete_logs_before(cutoff).map_err(e2s)
}

// ---------- Agent 生成 ----------

fn agent_sid() -> String {
    format!(
        "s{:x}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    )
}

/// 把文件集物化到 data/plugins/<id>/ 并真实加载（load+describe）——安装与热重载共用。
async fn materialize_and_install(
    state: &AppState,
    files: &[(String, String)],
) -> Result<serde_json::Value, String> {
    let manifest: serde_json::Value = files
        .iter()
        .find(|(n, _)| n == "plugin.json")
        .and_then(|(_, c)| serde_json::from_str(c).ok())
        .ok_or_else(|| "plugin.json 缺失或非法".to_string())?;
    let id = manifest["id"].as_str().unwrap_or("").to_string();
    if !valid_plugin_id(&id) {
        return Err("plugin.json 缺少合法 id（仅限字母数字-_，≤64）".into());
    }
    let dir = plugins_root().join(&id);
    std::fs::create_dir_all(&dir).map_err(e2s)?;
    for (name, content) in files {
        let path = dir.join(name);
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).map_err(e2s)?;
        }
        std::fs::write(&path, content).map_err(e2s)?;
    }
    install_dir(&state, dir).await
}

/// 执行一次工具调用，返回 (喂给模型的结果, 展示给用户的行)。
async fn run_agent_tool(
    state: &AppState,
    files: &mut Vec<(String, String)>,
    call: &agentmod::ToolCall,
) -> (String, String) {
    let arg_str = |k: &str| call.args[k].as_str().unwrap_or("").to_string();
    let set_file = |files: &mut Vec<(String, String)>, n: String, c: String| {
        if let Some(f) = files.iter_mut().find(|(p, _)| *p == n) {
            f.1 = c;
        } else {
            files.push((n, c));
        }
    };
    match call.name.as_str() {
        "write_file" => {
            let path = agentmod::sanitize_path(&arg_str("path"));
            match path {
                Some(p) => {
                    set_file(files, p.clone(), arg_str("content"));
                    (format!("ok: 已写入 {p}"), format!("写入 {p}"))
                }
                None => ("error: 非法路径".into(), "写入失败：非法路径".into()),
            }
        }
        "read_file" => {
            let p = arg_str("path");
            match files.iter().find(|(n, _)| *n == p) {
                Some((_, c)) => (c.clone(), format!("读取 {p}")),
                None => (format!("error: {p} 不存在"), format!("读取 {p}：不存在")),
            }
        }
        "delete_file" => {
            let p = arg_str("path");
            let before = files.len();
            files.retain(|(n, _)| *n != p);
            if files.len() < before {
                (format!("ok: 已删除 {p}"), format!("删除 {p}"))
            } else {
                (format!("error: {p} 不存在"), format!("删除 {p}：不存在"))
            }
        }
        "list_files" => {
            let list: Vec<_> = files.iter().map(|(n, _)| n.as_str()).collect();
            (
                format!("[{}]", list.join(", ")),
                format!("列出 {} 个文件", list.len()),
            )
        }
        "install_plugin" => match materialize_and_install(state, files).await {
            Ok(v) => (format!("ok: {v}"), "安装校验通过".into()),
            Err(e) => (format!("error: {e}"), "安装校验失败".into()),
        },
        other => (
            format!("error: 未知工具 {other}"),
            format!("未知工具 {other}"),
        ),
    }
}

/// 工具结果截断，防单个巨型文件/报错撑爆上下文（对齐原版 trimToolResponses 思路）。
fn trunc_tool_result(s: String) -> String {
    const MAX: usize = 6000;
    if s.len() <= MAX {
        s
    } else {
        // 按字符截断：&s[..MAX] 可能切开 UTF-8 多字节直接 panic
        let head: String = s.chars().take(MAX).collect();
        format!("{head}…[截断 {} 字符]", s.len() - head.len())
    }
}

/// 多轮生成：session 空=新会话。模型经工具调用自主写文件/装验/修复，最多 16 轮。
/// 进度经 `agent_progress` 事件推送（{session, line}）。
#[tauri::command]
pub async fn agent_generate(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    session: Option<String>,
    runtime: String,
    prompt: String,
) -> Result<serde_json::Value, String> {
    let cfg = agentmod::ProviderCfg::from_db(&state.db).map_err(|e| format!("{e:#}"))?;
    let sid = session.unwrap_or_else(agent_sid);

    // 锁内只借出会话数据，await 期间不持锁
    let (rt, mut msgs, mut files) = {
        let mut sessions = state.agent_sessions.lock().unwrap();
        let s = sessions.entry(sid.clone()).or_insert_with(|| AgentSession {
            runtime: runtime.clone(),
            msgs: Vec::new(),
            files: Vec::new(),
        });
        (s.runtime.clone(), s.msgs.clone(), s.files.clone())
    };
    msgs.push(agentmod::Msg::User(prompt));

    let sys = agentmod::system_prompt(&rt);
    let progress = |line: &str| {
        let _ = app.emit("agent_progress", json!({ "session": sid, "line": line }));
    };
    progress(&format!("已发送给 {}，等待回复…", cfg.model));

    let mut final_text = String::new();
    const MAX_ROUNDS: usize = 16;
    for _ in 0..MAX_ROUNDS {
        let resp = agentmod::chat_tools(&state.http, &cfg, &sys, &msgs)
            .await
            .map_err(|e| format!("{e:#}"))?;
        match resp {
            agentmod::ChatResp::Text(t) => {
                final_text = t.clone();
                msgs.push(agentmod::Msg::Assistant {
                    text: t,
                    calls: Vec::new(),
                });
                break;
            }
            agentmod::ChatResp::Calls { text, calls } => {
                msgs.push(agentmod::Msg::Assistant {
                    text,
                    calls: calls.clone(),
                });
                for c in calls {
                    let (result, display) = run_agent_tool(&state, &mut files, &c).await;
                    progress(&display);
                    msgs.push(agentmod::Msg::Tool {
                        id: c.id,
                        name: c.name,
                        content: trunc_tool_result(result),
                    });
                }
                progress("工具执行完毕，继续等待回复…");
            }
        }
    }

    // 兜底：最终文本里带的 <<<FILE:>>> 块也并入文件集
    for (n, c) in agentmod::parse_files(&final_text) {
        if let Some(p) = agentmod::sanitize_path(&n) {
            if let Some(f) = files.iter_mut().find(|(x, _)| *x == p) {
                f.1 = c;
            } else {
                files.push((p, c));
            }
        }
    }

    {
        let mut sessions = state.agent_sessions.lock().unwrap();
        if let Some(s) = sessions.get_mut(&sid) {
            s.msgs = msgs;
            s.files = files.clone();
        }
    }
    progress(&format!("本轮结束：当前 {} 个文件", files.len()));
    let files_json: Vec<_> = files
        .iter()
        .map(|(p, c)| json!({ "path": p, "content": c }))
        .collect();
    Ok(json!({
        "session": sid,
        "files": files_json,
        "text": agentmod::strip_file_blocks(&final_text),
    }))
}

/// 把会话最新文件集写入 data/plugins/<id>/ 并 load——同 id 即热重载。
#[tauri::command]
pub async fn agent_install(
    state: State<'_, AppState>,
    session: String,
) -> Result<serde_json::Value, String> {
    let files = {
        let sessions = state.agent_sessions.lock().unwrap();
        sessions.get(&session).map(|s| s.files.clone())
    }
    .ok_or_else(|| "会话不存在，请重新生成".to_string())?;
    if files.is_empty() {
        return Err("会话里还没有生成文件".into());
    }
    materialize_and_install(&state, &files).await
}

/// 拉取模型列表（OpenAI /models · Anthropic /v1/models）。
#[tauri::command]
pub async fn agent_models(
    state: State<'_, AppState>,
    provider: String,
    base: String,
    key: String,
) -> Result<Vec<String>, String> {
    if key.is_empty() {
        return Err("API Key 不能为空".into());
    }
    agentmod::models(&state.http, &provider, &base, &key)
        .await
        .map_err(|e| format!("{e:#}"))
}

/// 设置页「测试连接」：直接用表单参数做一轮最小对话，不读库。
#[tauri::command]
pub async fn agent_test(
    state: State<'_, AppState>,
    provider: String,
    base: String,
    key: String,
    model: String,
) -> Result<String, String> {
    let cfg = agentmod::ProviderCfg {
        base: agentmod::default_base(&provider, base),
        provider,
        key,
        model,
    };
    if cfg.key.is_empty() || cfg.model.is_empty() {
        return Err("API Key / 模型名不能为空".into());
    }
    let text = agentmod::chat(&state.http, &cfg, "", "回复 pong")
        .await
        .map_err(|e| format!("{e:#}"))?;
    Ok(text.chars().take(120).collect())
}

// ---------- 定时任务管理 ----------

#[tauri::command]
pub fn list_timers(
    state: State<'_, AppState>,
) -> Result<Vec<mashiro_core::storage::models::TimerRow>, String> {
    state.db.list_timers().map_err(e2s)
}

#[tauri::command]
pub fn set_timer_enabled(state: State<'_, AppState>, id: i64, enabled: bool) -> Result<(), String> {
    state.db.set_timer_enabled(id, enabled).map_err(e2s)
}

#[tauri::command]
pub fn remove_timer(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    state.db.delete_timer(id).map_err(e2s)
}

// ---------- 聊天室 ----------

#[tauri::command]
pub fn list_conversations(
    state: State<'_, AppState>,
) -> Result<Vec<mashiro_core::storage::models::Conversation>, String> {
    state.db.list_conversations().map_err(e2s)
}

#[tauri::command]
pub fn chat_history(
    state: State<'_, AppState>,
    appid: i64,
    group_id: String,
    before_id: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<LogRow>, String> {
    state
        .db
        .chat_history(
            appid,
            &group_id,
            before_id.unwrap_or(0),
            limit.unwrap_or(100),
        )
        .map_err(e2s)
}

/// 聊天室手动发送：kind 0 群/1 频道/2 私聊/3 频道私聊；可附被动 msg_id 与 keyboard。
#[tauri::command]
pub async fn send_chat(
    state: State<'_, AppState>,
    appid: i64,
    openid: String,
    kind: i32,
    content: String,
    markdown: bool,
    keyboard: Option<serde_json::Value>,
    msg_id: Option<String>,
) -> Result<(), String> {
    let bot = state
        .bots
        .get(&appid)
        .map(|b| b.clone())
        .ok_or_else(|| "机器人不在线".to_string())?;
    let token = bot.access_token().await.map_err(e2s)?;
    let req_json = json!({ "kind": kind, "openid": openid, "markdown": markdown,
                           "content": content, "msg_id": msg_id, "keyboard": keyboard });
    let opts = mashiro_core::bot::api::SendOpts {
        msg_id: msg_id.filter(|s| !s.is_empty()),
        keyboard: keyboard.filter(|k| k.is_object()),
        ..Default::default()
    };
    let r = if markdown {
        mashiro_core::bot::api::send_markdown(
            &state.http,
            &token,
            bot.api_base(),
            kind,
            &openid,
            &content,
            opts,
        )
        .await
    } else {
        mashiro_core::bot::api::send_text(
            &state.http,
            &token,
            bot.api_base(),
            kind,
            &openid,
            &content,
            opts,
        )
        .await
    };
    let _ = state.db.insert_log(&LogRow {
        id: 0,
        ts: chrono::Utc::now().timestamp_millis(),
        appid,
        kind: if r.is_ok() {
            "Send".into()
        } else {
            "Error".into()
        },
        group_id: openid,
        user: String::new(),
        content: match &r {
            Ok(_) => content.chars().take(200).collect(),
            Err(e) => format!("发送失败: {e:#}"),
        },
        raw: send_raw(req_json, &r),
    });
    r.map(|_| ()).map_err(e2s)
}

// ---------- Markdown 图片素材 ----------

#[tauri::command]
pub async fn material_list(
    state: State<'_, AppState>,
    appid: i64,
    cursor: Option<String>,
) -> Result<serde_json::Value, String> {
    let bot = state
        .bots
        .get(&appid)
        .map(|b| b.clone())
        .ok_or_else(|| "机器人不在线".to_string())?;
    let token = bot.access_token().await.map_err(e2s)?;
    mashiro_core::bot::api::material_list(&state.http, &token, appid, &cursor.unwrap_or_default())
        .await
        .map_err(e2s)
}

#[tauri::command]
pub async fn material_upload(
    state: State<'_, AppState>,
    appid: i64,
    path: String,
) -> Result<serde_json::Value, String> {
    let bot = state
        .bots
        .get(&appid)
        .map(|b| b.clone())
        .ok_or_else(|| "机器人不在线".to_string())?;
    let token = bot.access_token().await.map_err(e2s)?;
    mashiro_core::bot::api::material_upload(
        &state.http,
        &token,
        appid,
        PathBuf::from(&path).as_path(),
    )
    .await
    .map_err(e2s)
}

#[tauri::command]
pub async fn material_delete(
    state: State<'_, AppState>,
    appid: i64,
    image_id: String,
) -> Result<(), String> {
    let bot = state
        .bots
        .get(&appid)
        .map(|b| b.clone())
        .ok_or_else(|| "机器人不在线".to_string())?;
    let token = bot.access_token().await.map_err(e2s)?;
    mashiro_core::bot::api::material_delete(&state.http, &token, appid, &image_id)
        .await
        .map_err(e2s)
}

// ---------- 群授权助手 ----------

/// 查询机器人群内状态：主动发言开关 / 消息范围 / 群角色。
/// 接口限白名单机器人时返回 11253，前端据此降级为指引说明。
#[tauri::command]
pub async fn group_bot_state(
    state: State<'_, AppState>,
    appid: i64,
    group_openid: String,
) -> Result<serde_json::Value, String> {
    let bot = state
        .bots
        .get(&appid)
        .map(|b| b.clone())
        .ok_or_else(|| "机器人不在线".to_string())?;
    let token = bot.access_token().await.map_err(e2s)?;
    mashiro_core::bot::api::group_bot_state(&state.http, &token, bot.api_base(), &group_openid)
        .await
        .map_err(e2s)
}

/// 发送群授权指引卡：Markdown 步骤说明（纯文字，发不出降级文本）。
/// 群主在群成员列表 → 机器人资料页⚙，或聊天信息 → 群机器人 → 管理 两条路进设置。
#[tauri::command]
pub async fn send_auth_card(
    state: State<'_, AppState>,
    appid: i64,
    group_openid: String,
    msg_id: Option<String>,
) -> Result<(), String> {
    let bot = state
        .bots
        .get(&appid)
        .map(|b| b.clone())
        .ok_or_else(|| "机器人不在线".to_string())?;
    let token = bot.access_token().await.map_err(e2s)?;
    let md = concat!(
        "## 群机器人授权指引\n\n",
        "本群需**群主**完成两项设置（仅手机 QQ，电脑端无法操作）：\n\n",
        "**方式一**\n",
        "1. 本群「群成员」列表 →「机器人」分组 → 点本机器人头像\n",
        "2. 资料页右上角「⚙」进入设置\n",
        "3. 「机器人可获取的群聊消息范围」→ 获取群内全部消息\n",
        "4. 打开「机器人主动在群聊内发言」\n\n",
        "**方式二**\n",
        "群聊页右上角「☰」→「聊天信息」→「群机器人」→ 本机器人右侧「管理」，同上设置\n\n",
        "设置即时生效，无需重拉机器人。"
    );
    let opts = mashiro_core::bot::api::SendOpts {
        msg_id: msg_id.filter(|s| !s.is_empty()),
        ..Default::default()
    };
    let mut r = mashiro_core::bot::api::send_markdown(
        &state.http,
        &token,
        bot.api_base(),
        0,
        &group_openid,
        md,
        opts.clone(),
    )
    .await;
    if r.is_err() {
        // 群 markdown 可能需要模板/资格，降级纯文本
        let text = md.replace("## ", "").replace("**", "");
        r = mashiro_core::bot::api::send_text(
            &state.http,
            &token,
            bot.api_base(),
            0,
            &group_openid,
            &text,
            opts,
        )
        .await;
    }
    let _ = state.db.insert_log(&LogRow {
        id: 0,
        ts: chrono::Utc::now().timestamp_millis(),
        appid,
        kind: if r.is_ok() {
            "Send".into()
        } else {
            "Error".into()
        },
        group_id: group_openid.clone(),
        user: String::new(),
        content: match &r {
            Ok(_) => "[授权指引卡]".to_string(),
            Err(e) => format!("授权指引卡发送失败: {e:#}"),
        },
        raw: send_raw(json!({ "group_openid": group_openid, "markdown": md }), &r),
    });
    r.map(|_| ()).map_err(e2s)
}
