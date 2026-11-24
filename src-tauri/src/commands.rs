use std::collections::HashMap;
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
    /// 在线起始毫秒时间戳；离线为 0
    pub online_since: i64,
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
            online_since: e.value().online_since(),
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
    state.db.set_setting(&key, &value).map_err(e2s)?;
    // MCP 监听配置即改即生效：杀旧任务按新值重起（off → 仅停）
    if key == "mcp_listen" || key == "mcp_token" {
        state.restart_mcp();
    }
    Ok(())
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

// ---------- Agent 会话持久化（data/agent_sessions/<sid>.json） ----------

fn agent_dir() -> PathBuf {
    mashiro_core::storage::db::default_dir().join("agent_sessions")
}

/// sid 只许字母数字：会话 id 直接拼文件路径，放过 ../ 就是路径穿越洞。
fn valid_sid(sid: &str) -> bool {
    !sid.is_empty() && sid.len() <= 64 && sid.chars().all(|c| c.is_ascii_alphanumeric())
}

fn session_path(sid: &str) -> PathBuf {
    agent_dir().join(format!("{sid}.json"))
}

/// 落盘：文件名即 sid，内容 = {id, ...AgentSession}。写坏不影响主流程（仅 warn）。
fn save_session_disk(sid: &str, s: &AgentSession) {
    let mut v = match serde_json::to_value(s) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("agent session serialize failed: {e}");
            return;
        }
    };
    v["id"] = json!(sid);
    if std::fs::create_dir_all(agent_dir()).is_ok() {
        if let Err(e) = std::fs::write(session_path(sid), v.to_string()) {
            tracing::warn!("agent session save failed: {e}");
        }
    }
    // 上限 50 个：按修改时间清最老的，会话文件不会无限攒
    const MAX_SESSIONS: usize = 50;
    if let Ok(rd) = std::fs::read_dir(agent_dir()) {
        let mut fs: Vec<_> = rd
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
            .collect();
        if fs.len() > MAX_SESSIONS {
            let extra = fs.len() - MAX_SESSIONS;
            fs.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
            for e in fs.into_iter().take(extra) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// 从磁盘复活会话（重启后继续聊）。文件损坏就当作没有。
fn load_session_disk(sid: &str) -> Option<AgentSession> {
    if !valid_sid(sid) {
        return None;
    }
    let txt = std::fs::read_to_string(session_path(sid)).ok()?;
    serde_json::from_str(&txt).ok()
}

/// 取会话：内存优先，miss 时尝试磁盘恢复并回种内存。
fn get_or_restore_session<'a>(
    sessions: &'a mut HashMap<String, AgentSession>,
    sid: &str,
) -> Option<&'a mut AgentSession> {
    if !sessions.contains_key(sid) {
        if let Some(s) = load_session_disk(sid) {
            sessions.insert(sid.to_string(), s);
        }
    }
    sessions.get_mut(sid)
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
    app: &tauri::AppHandle,
    state: &AppState,
    sid: &str,
    files: &mut Vec<(String, String)>,
    workdir: Option<&std::path::Path>,
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
    // 文件集变化实时推给前端，生成中侧边栏就能看见产出
    let emit_files = |files: &[(String, String)]| {
        let fj: Vec<_> = files
            .iter()
            .map(|(p, c)| json!({ "path": p, "content": c }))
            .collect();
        let _ = app.emit("agent_files", json!({ "session": sid, "files": fj }));
    };
    match call.name.as_str() {
        "write_file" => {
            let path = agentmod::sanitize_path(&arg_str("path"));
            match (workdir, path) {
                // 工作区：直写磁盘真身，files 快照跟随刷新
                (Some(wd), Some(p)) => {
                    let full = wd.join(&p);
                    if let Some(par) = full.parent() {
                        let _ = std::fs::create_dir_all(par);
                    }
                    match std::fs::write(&full, arg_str("content")) {
                        Ok(_) => {
                            *files = collect_plugin_files(wd);
                            emit_files(files);
                            (format!("ok: 已写入 {p}"), format!("写入 {p}"))
                        }
                        Err(e) => (format!("error: {e}"), "写入失败".into()),
                    }
                }
                (None, Some(p)) => {
                    set_file(files, p.clone(), arg_str("content"));
                    emit_files(files);
                    (format!("ok: 已写入 {p}"), format!("写入 {p}"))
                }
                _ => ("error: 非法路径".into(), "写入失败：非法路径".into()),
            }
        }
        "read_file" => {
            // 工作区模式先消毒——wd.join("../x") 能穿出插件目录读任意文件
            let p = agentmod::sanitize_path(&arg_str("path")).unwrap_or_default();
            if p.is_empty() {
                ("error: 非法路径".into(), "读取失败：非法路径".into())
            } else if let Some(wd) = workdir {
                match std::fs::read_to_string(wd.join(&p)) {
                    Ok(c) => (c, format!("读取 {p}")),
                    Err(_) => (format!("error: {p} 不存在"), format!("读取 {p}：不存在")),
                }
            } else {
                match files.iter().find(|(n, _)| *n == p) {
                    Some((_, c)) => (c.clone(), format!("读取 {p}")),
                    None => (format!("error: {p} 不存在"), format!("读取 {p}：不存在")),
                }
            }
        }
        "delete_file" => {
            let p = agentmod::sanitize_path(&arg_str("path")).unwrap_or_default();
            if p.is_empty() {
                ("error: 非法路径".into(), "删除失败：非法路径".into())
            } else if let Some(wd) = workdir {
                match std::fs::remove_file(wd.join(&p)) {
                    Ok(_) => {
                        *files = collect_plugin_files(wd);
                        emit_files(files);
                        (format!("ok: 已删除 {p}"), format!("删除 {p}"))
                    }
                    Err(_) => (format!("error: {p} 不存在"), format!("删除 {p}：不存在")),
                }
            } else {
                let before = files.len();
                files.retain(|(n, _)| *n != p);
                if files.len() < before {
                    emit_files(files);
                    (format!("ok: 已删除 {p}"), format!("删除 {p}"))
                } else {
                    (format!("error: {p} 不存在"), format!("删除 {p}：不存在"))
                }
            }
        }
        "list_files" => {
            if let Some(wd) = workdir {
                *files = collect_plugin_files(wd);
            }
            let list: Vec<_> = files.iter().map(|(n, _)| n.as_str()).collect();
            (
                format!("[{}]", list.join(", ")),
                format!("列出 {} 个文件", list.len()),
            )
        }
        "install_plugin" => match workdir {
            // 工作区：文件已在盘上，直接装真目录（同 id 即热重载）
            Some(wd) => match install_dir(state, wd.to_path_buf()).await {
                Ok(v) => (format!("ok: {v}"), "安装校验通过".into()),
                Err(e) => (format!("error: {e}"), "安装校验失败".into()),
            },
            None => match materialize_and_install(state, files).await {
                Ok(v) => (format!("ok: {v}"), "安装校验通过".into()),
                Err(e) => (format!("error: {e}"), "安装校验失败".into()),
            },
        },
        // 进程内直调 MCP 工具集：插件调试/日志/KV/定时器/发消息全能用
        "mcp_call" => {
            let name = arg_str("name");
            let args = call.args["args"].clone();
            let deps = mashiro_core::mcp::McpDeps {
                db: Arc::clone(&state.db),
                plugins: Arc::clone(&state.plugins),
                bots: Arc::clone(&state.bots),
                http: state.http.clone(),
            };
            match mashiro_core::mcp::call_internal(deps, &name, &args).await {
                Ok(v) => (
                    serde_json::to_string_pretty(&v).unwrap_or_default(),
                    format!("mcp {name}"),
                ),
                Err(e) => (format!("error: {e}"), format!("mcp {name} 失败")),
            }
        }
        "ask_user" => {
            let questions = call.args["questions"].clone();
            let (tx, rx) = tokio::sync::oneshot::channel::<Vec<String>>();
            state.agent_answers.lock().unwrap().insert(
                call.id.clone(),
                crate::state::PendingAsk {
                    sid: sid.to_string(),
                    questions: questions.clone(),
                    tx,
                },
            );
            let _ = app.emit(
                "agent_ask",
                json!({ "session": sid, "call_id": call.id, "questions": questions }),
            );
            // 三选一：用户作答 / 600s 超时 / 点「停止」。挂起条目无论如何都得清。
            let cancel_watch = async {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    if state.agent_cancel.lock().unwrap().contains(sid) {
                        break;
                    }
                }
            };
            let waited = tokio::select! {
                r = tokio::time::timeout(std::time::Duration::from_secs(600), rx) => Some(r),
                _ = cancel_watch => None,
            };
            state.agent_answers.lock().unwrap().remove(&call.id);
            match waited {
                None => ("error: 用户取消了生成".into(), "已取消".into()),
                Some(Ok(Ok(answers))) => {
                    let qs = questions.as_array().cloned().unwrap_or_default();
                    let mut out = String::new();
                    for (i, a) in answers.iter().enumerate() {
                        let q = qs.get(i).and_then(|x| x["q"].as_str()).unwrap_or("?");
                        out.push_str(&format!("Q: {q}\nA: {a}\n"));
                    }
                    (out, format!("用户已回答 {} 个问题", answers.len()))
                }
                _ => ("error: 用户未作答（超时或取消）".into(), "提问超时".into()),
            }
        }
        other => (
            format!("error: 未知工具 {other}"),
            format!("未知工具 {other}"),
        ),
    }
}

/// 用户点了「停止」：生成循环在检查点优雅退出，已产出文件现场保留。
/// 只对活跃会话生效——误点/迟点留下的僵尸旗标会把下一次生成当场打死。
#[tauri::command]
pub fn agent_cancel(state: State<'_, AppState>, session: String) -> Result<(), String> {
    if state.agent_active.lock().unwrap().contains(&session) {
        state.agent_cancel.lock().unwrap().insert(session);
    }
    Ok(())
}

/// Agent 提问弹窗回填：call_id → 正在等答案的 ask_user 工具调用。
#[tauri::command]
pub fn agent_answer(
    state: State<'_, AppState>,
    call_id: String,
    answers: Vec<String>,
) -> Result<(), String> {
    let pa = state.agent_answers.lock().unwrap().remove(&call_id);
    match pa {
        Some(pa) => {
            let _ = pa.tx.send(answers);
            Ok(())
        }
        None => Err("该提问已失效".into()),
    }
}

/// 工具结果截断，防单个巨型文件/报错撑爆上下文（对齐原版 trimToolResponses 思路）。
/// 上限要盖过真实插件文件体量（一个上千行的 main.js 也就 ~30KB），
/// 否则 read_file 永远只能读到前半截，模型会以为文件本身就残了。
fn trunc_tool_result(s: String) -> String {
    const MAX: usize = 40000;
    if s.len() <= MAX {
        s
    } else {
        // 按字符截断：&s[..MAX] 可能切开 UTF-8 多字节直接 panic
        let head: String = s.chars().take(MAX).collect();
        format!("{head}…[截断 {} 字符]", s.len() - head.len())
    }
}

/// 上下文瘦身（无限轮次的配套）：老 Tool 结果和老 write_file 参数折成占位——
/// 文件真相在 files 集里，模型要看会走 read_file；保留首轮需求 + 最近窗口，
/// 不删消息以保持 tool_calls ↔ tool 配对完整。
fn compact_msgs(msgs: &mut Vec<agentmod::Msg>) {
    const KEEP_TAIL: usize = 24;
    if msgs.len() <= KEEP_TAIL + 4 {
        return;
    }
    let cutoff = msgs.len() - KEEP_TAIL;
    for m in msgs.iter_mut().take(cutoff).skip(1) {
        match m {
            agentmod::Msg::Tool { content, .. } if content.len() > 300 => {
                // 文案必须说清是「上下文瘦身」不是「文件截断」——否则模型会谎报文件残了
                *content = format!(
                    "[此条较早工具结果为省上下文已省略 {} 字符；文件未被截断，需要内容请用 read_file 重新读取]",
                    content.len()
                );
            }
            agentmod::Msg::Assistant { text, calls } => {
                if text.len() > 500 {
                    let head: String = text.chars().take(500).collect();
                    *text = format!("{head}…[截断]");
                }
                for c in calls.iter_mut() {
                    if c.args.to_string().len() > 300 {
                        c.args = json!({ "_omitted": "较早的调用参数已省略" });
                    }
                }
            }
            _ => {}
        }
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
    if !valid_sid(&sid) {
        return Err("非法会话 id".into());
    }

    // 同会话并发生成会互相覆盖 msgs/files，直接拒掉
    if !state.agent_active.lock().unwrap().insert(sid.clone()) {
        return Err("该会话正在生成中，请稍候".into());
    }
    // RAII：生成中途 panic 也必须放掉 active/cancel 标记，否则会话永远卡「生成中」
    struct GenGuard<'a> {
        active: &'a std::sync::Mutex<std::collections::HashSet<String>>,
        cancel: &'a std::sync::Mutex<std::collections::HashSet<String>>,
        sid: String,
    }
    impl Drop for GenGuard<'_> {
        fn drop(&mut self) {
            if let Ok(mut g) = self.active.lock() {
                g.remove(&self.sid);
            }
            if let Ok(mut g) = self.cancel.lock() {
                g.remove(&self.sid);
            }
        }
    }
    let _guard = GenGuard {
        active: &state.agent_active,
        cancel: &state.agent_cancel,
        sid: sid.clone(),
    };
    // 防御性自清：上一轮残留的取消旗标（若有）绝不能带进本轮——否则开局即「已停止」
    state.agent_cancel.lock().unwrap().remove(&sid);
    let cancelled = || state.agent_cancel.lock().unwrap().contains(&sid);

    // 锁内只借出会话数据，await 期间不持锁；内存 miss 时从磁盘复活
    let (rt, mut msgs, mut files, workdir) = {
        let mut sessions = state.agent_sessions.lock().unwrap();
        let s = get_or_restore_session(&mut sessions, &sid)
            .map(|s| {
                s.msgs.push(agentmod::Msg::User(prompt.clone()));
                s
            })
            .map(|s| {
                (
                    s.runtime.clone(),
                    s.msgs.clone(),
                    s.files.clone(),
                    s.workdir.clone(),
                )
            });
        match s {
            Some(t) => t,
            None => {
                sessions.insert(
                    sid.clone(),
                    AgentSession {
                        runtime: runtime.clone(),
                        title: prompt.chars().take(30).collect(),
                        updated_at: chrono::Utc::now().timestamp_millis(),
                        msgs: vec![agentmod::Msg::User(prompt.clone())],
                        workdir: None,
                        files: Vec::new(),
                    },
                );
                (
                    runtime.clone(),
                    vec![agentmod::Msg::User(prompt.clone())],
                    Vec::new(),
                    None,
                )
            }
        }
    };
    let workdir = workdir.map(std::path::PathBuf::from);

    // 开局先落一盘：哪怕首个 LLM 调用就暴毙，会话也已经能恢复
    persist_agent(&state, &sid, msgs.clone(), files.clone());

    let sys = agentmod::system_prompt(&rt);
    let progress = |line: &str| {
        let _ = app.emit("agent_progress", json!({ "session": sid, "line": line }));
    };
    progress(&format!("已发送给 {}，等待回复…", cfg.model));

    // 生成主循环（无限轮次 + 卡死保险丝）。包成 async block 是为了：
    // 中途报错的脏现场（已写一半的文件/上下文）也要回写落盘，不能跟着错误一起蒸发。
    let gen: Result<String, String> = async {
        // 同一工具同参数连续调 STUCK_LIMIT 次 → 停止本轮，防失控空转烧额度
        const STUCK_LIMIT: usize = 4;
        let mut last_sig = String::new();
        let mut same_calls = 0usize;
        let mut install_fails = 0usize;
        loop {
            if cancelled() {
                break Ok("（已停止：用户取消）".into());
            }
            let resp = {
                let app2 = app.clone();
                let sid2 = sid.clone();
                // select! 双保险：点「停止」立即掐掉在途 HTTP 请求，不用等流超时
                tokio::select! {
                    r = agentmod::chat_tools_stream(&state.http, &cfg, &sys, &msgs, move |d| {
                        let _ = app2.emit("agent_delta", json!({ "session": sid2, "delta": d }));
                    }) => r.map_err(|e| format!("{e:#}"))?,
                    _ = async {
                        loop {
                            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                            if cancelled() { break; }
                        }
                    } => break Ok("（已停止：用户取消）".into()),
                }
            };
            match resp {
                agentmod::ChatResp::Text(t) => {
                    msgs.push(agentmod::Msg::Assistant {
                        text: t.clone(),
                        calls: Vec::new(),
                    });
                    break Ok(t);
                }
                agentmod::ChatResp::Calls { text, calls } => {
                    // 本轮的思考文字插进流程行（say: 前缀，前端渲染成 md 块并清掉底部 stream）——
                    // 不然文字全堆在卡片底部、工具行全堆顶部，视觉上是割裂的两半
                    if !text.is_empty() {
                        progress(&format!("say:{text}"));
                    }
                    msgs.push(agentmod::Msg::Assistant {
                        text,
                        calls: calls.clone(),
                    });
                    let mut stuck = false;
                    let mut user_cancel = false;
                    for c in calls {
                        let sig = format!("{}:{}", c.name, c.args);
                        if !stuck && !user_cancel && sig == last_sig {
                            same_calls += 1;
                        } else if !stuck && !user_cancel {
                            same_calls = 0;
                            last_sig = sig;
                        }
                        if !stuck && !user_cancel && same_calls >= STUCK_LIMIT {
                            stuck = true;
                            progress("检测到重复工具调用，已停止本轮生成");
                        }
                        if !stuck && !user_cancel && cancelled() {
                            user_cancel = true;
                            progress("收到停止指令，正在收尾…");
                        }
                        // 剩余调用必须拿到结果（哪怕是占位），tool_calls↔tool 配对断了下一轮请求会被接口拒
                        let (result, display) = if user_cancel {
                            ("error: 用户取消了生成".into(), "已跳过（用户取消）".into())
                        } else if stuck {
                            ("error: 重复调用已停止".into(), "已跳过（重复调用）".into())
                        } else {
                            run_agent_tool(&app, &state, &sid, &mut files, workdir.as_deref(), &c)
                                .await
                        };
                        // 装验失败计数（中间夹写文件不清零，装成功才清零）：
                        // 宿主环境坏掉时模型会误以为代码错了一直改一直装，兜底掐停
                        if c.name == "install_plugin" && !stuck && !user_cancel {
                            if result.starts_with("error:") {
                                install_fails += 1;
                                if install_fails >= 3 {
                                    stuck = true;
                                    progress("安装校验连续失败 3 次：疑似宿主环境问题，已停止本轮");
                                }
                            } else {
                                install_fails = 0;
                            }
                        }
                        progress(&display);
                        msgs.push(agentmod::Msg::Tool {
                            id: c.id,
                            name: c.name,
                            content: trunc_tool_result(result),
                        });
                    }
                    compact_msgs(&mut msgs);
                    // 每轮工具批后增量快照：进程暴毙/被重启也能恢复到大半路
                    persist_agent(&state, &sid, msgs.clone(), files.clone());
                    if stuck {
                        break Ok(if install_fails >= 3 {
                            "（已停止：安装校验连续失败，可能是插件宿主环境问题而非代码问题）"
                                .into()
                        } else {
                            "（已停止：模型重复调用同一工具，疑似陷入死循环）".into()
                        });
                    }
                    if user_cancel {
                        break Ok("（已停止：用户取消）".into());
                    }
                    progress("工具执行完毕，继续等待回复…");
                }
            }
        }
    }
    .await;

    // 成功路径先把 <<<FILE:>>> 兜底并进文件集，再统一回写
    let final_text = match gen {
        Ok(t) => {
            for (n, c) in agentmod::parse_files(&t) {
                if let Some(p) = agentmod::sanitize_path(&n) {
                    if let Some(f) = files.iter_mut().find(|(x, _)| *x == p) {
                        f.1 = c;
                    } else {
                        files.push((p, c));
                    }
                }
            }
            t
        }
        Err(e) => {
            let _ = app.emit("agent_done", json!({ "session": sid, "error": e }));
            persist_agent(&state, &sid, msgs, files);
            return Err(e);
        }
    };

    persist_agent(&state, &sid, msgs, files.clone());
    progress(&format!("本轮结束：当前 {} 个文件", files.len()));
    // agent_done：页面中途切走再回来，靠事件收尾（invoke 返回值已经拿不到）
    let _ = app.emit(
        "agent_done",
        json!({ "session": sid, "text": agentmod::strip_file_blocks(&final_text) }),
    );
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

/// 生成结束统一回写：内存会话更新 + 落盘（崩溃/重启都不丢）。
fn persist_agent(
    state: &AppState,
    sid: &str,
    msgs: Vec<agentmod::Msg>,
    files: Vec<(String, String)>,
) {
    let mut sessions = state.agent_sessions.lock().unwrap();
    if let Some(s) = sessions.get_mut(sid) {
        s.msgs = msgs;
        s.files = files;
        s.updated_at = chrono::Utc::now().timestamp_millis();
        save_session_disk(sid, s);
    }
}

/// 把会话最新文件集写入 data/plugins/<id>/ 并 load——同 id 即热重载。
#[tauri::command]
pub async fn agent_install(
    state: State<'_, AppState>,
    session: String,
) -> Result<serde_json::Value, String> {
    // 重启后内存空，从磁盘把会话捞回来再装
    let (files, workdir) = {
        let mut sessions = state.agent_sessions.lock().unwrap();
        get_or_restore_session(&mut sessions, &session)
            .map(|s| (s.files.clone(), s.workdir.clone()))
    }
    .ok_or_else(|| "会话不存在，请重新生成".to_string())?;
    // 工作区：文件本就在真目录，直接装（同 id 即热重载）；草稿模式走物化
    match workdir {
        Some(wd) => install_dir(&state, PathBuf::from(wd)).await,
        None => {
            if files.is_empty() {
                return Err("会话里还没有生成文件".into());
            }
            materialize_and_install(&state, &files).await
        }
    }
}

/// 会话列表：内存 + 磁盘合并，按最近活动排序。切页/重启后靠它找回现场。
#[tauri::command]
pub fn agent_sessions(state: State<'_, AppState>) -> Result<Vec<serde_json::Value>, String> {
    let mut out: Vec<serde_json::Value> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    {
        let sessions = state.agent_sessions.lock().unwrap();
        for (id, s) in sessions.iter() {
            seen.insert(id.clone());
            out.push(json!({
                "id": id,
                "title": s.title,
                "runtime": s.runtime,
                "updated_at": s.updated_at,
            }));
        }
    }
    if let Ok(rd) = std::fs::read_dir(agent_dir()) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let id = p
                .file_stem()
                .and_then(|x| x.to_str())
                .unwrap_or("")
                .to_string();
            if seen.contains(&id) {
                continue;
            }
            if let Ok(txt) = std::fs::read_to_string(&p) {
                if let Ok(s) = serde_json::from_str::<AgentSession>(&txt) {
                    out.push(json!({
                        "id": id,
                        "title": s.title,
                        "runtime": s.runtime,
                        "updated_at": s.updated_at,
                    }));
                }
            }
        }
    }
    out.sort_by(|a, b| b["updated_at"].as_i64().cmp(&a["updated_at"].as_i64()));
    Ok(out)
}

/// 恢复单个会话：文件集 + 轻量对话史（user/agent 文本，Tool 细节不还原——它们只是过程）。
#[tauri::command]
pub fn agent_session_state(
    state: State<'_, AppState>,
    session: String,
) -> Result<serde_json::Value, String> {
    let (runtime, files, msgs) = {
        let mut sessions = state.agent_sessions.lock().unwrap();
        let s = get_or_restore_session(&mut sessions, &session)
            .ok_or_else(|| "会话不存在".to_string())?;
        // 工作区会话恢复时从磁盘重读：外部编辑器/手动改动也看得见
        if let Some(wd) = s.workdir.clone() {
            s.files = collect_plugin_files(std::path::Path::new(&wd));
        }
        (s.runtime.clone(), s.files.clone(), s.msgs.clone())
    };
    let active = state.agent_active.lock().unwrap().contains(&session);
    let turns: Vec<_> = msgs
        .iter()
        .filter_map(|m| match m {
            agentmod::Msg::User(t) => Some(json!({ "role": "user", "text": t })),
            agentmod::Msg::Assistant { text, calls } if calls.is_empty() => {
                Some(json!({ "role": "agent", "text": agentmod::strip_file_blocks(text.as_str()) }))
            }
            _ => None,
        })
        .collect();
    let files_json: Vec<_> = files
        .iter()
        .map(|(p, c)| json!({ "path": p, "content": c }))
        .collect();
    Ok(json!({ "runtime": runtime, "files": files_json, "turns": turns, "active": active }))
}

/// 删除会话（内存 + 磁盘一起清）。
#[tauri::command]
pub fn agent_session_delete(state: State<'_, AppState>, session: String) -> Result<(), String> {
    if !valid_sid(&session) {
        return Err("非法会话 id".into());
    }
    state.agent_sessions.lock().unwrap().remove(&session);
    // 挂起中的提问一并清：sender drop 后等待方收到取消，不会成孤儿
    state
        .agent_answers
        .lock()
        .unwrap()
        .retain(|_, p| p.sid != session);
    let _ = std::fs::remove_file(session_path(&session));
    Ok(())
}

/// 插件可视化配置 schema（从 plugin.json 读，不要求插件已加载）。
#[tauri::command]
pub fn plugin_config_schema(
    state: State<'_, AppState>,
    plugin_id: String,
) -> Result<serde_json::Value, String> {
    // 已加载的从内存拿；没加载的读磁盘 manifest
    let schema = state.plugins.config_schema_of(&plugin_id);
    if !schema.is_empty() {
        return Ok(json!(schema));
    }
    let dir = state
        .db
        .list_plugins()
        .map_err(e2s)?
        .into_iter()
        .find(|p| p.uuid == plugin_id)
        .map(|p| PathBuf::from(p.dir))
        .ok_or_else(|| "插件不存在".to_string())?;
    let m = mashiro_core::plugin::manifest::Manifest::load(&dir).map_err(e2s)?;
    Ok(json!(m.config_schema))
}

/// 已保存的配置值（kv cfg: 前缀，JSON 解出）。
#[tauri::command]
pub fn plugin_config_get(
    state: State<'_, AppState>,
    plugin_id: String,
) -> Result<serde_json::Value, String> {
    let mut out = serde_json::Map::new();
    for (k, v) in state.db.kv_list(&plugin_id).map_err(e2s)? {
        if let Some(key) = k.strip_prefix("cfg:") {
            out.insert(
                key.to_string(),
                serde_json::from_str(&v).unwrap_or(json!(v)),
            );
        }
    }
    Ok(json!(out))
}

/// 保存配置：{key: value} → kv cfg:<key>（JSON 编码保类型）。
#[tauri::command]
pub fn plugin_config_set(
    state: State<'_, AppState>,
    plugin_id: String,
    values: serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    for (k, v) in values {
        let raw = serde_json::to_string(&v).map_err(e2s)?;
        state
            .db
            .kv_set(&plugin_id, &format!("cfg:{k}"), &raw)
            .map_err(e2s)?;
    }
    Ok(())
}

/// 递归收集插件目录文本文件（相对路径 → 内容），跳过大文件/依赖目录。
fn collect_plugin_files(dir: &std::path::Path) -> Vec<(String, String)> {
    const SKIP: &[&str] = &["node_modules", "__pycache__", ".git", "dist"];
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                if !SKIP.contains(&name.as_str()) {
                    stack.push(p);
                }
            } else if p.is_file() {
                // 单文件 >256KB 或不是 UTF-8 的大概率是二进制，跳过
                if e.metadata().map(|m| m.len() > 256 * 1024).unwrap_or(true) {
                    continue;
                }
                if let Ok(c) = std::fs::read_to_string(&p) {
                    let rel = p
                        .strip_prefix(dir)
                        .unwrap_or(&p)
                        .to_string_lossy()
                        .replace('\\', "/");
                    out.push((rel, c));
                }
            }
        }
        if out.len() > 50 {
            break;
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// 载入已有插件到会话（改插件模式）：目录文件合进会话文件集，runtime 跟随 manifest。
#[tauri::command]
pub fn agent_attach_plugin(
    state: State<'_, AppState>,
    session: Option<String>,
    plugin_id: String,
) -> Result<serde_json::Value, String> {
    // plugin_id 拼进文件路径与会话 id——必须白名单（插件 id 规范本身就要求 slug）
    if plugin_id.is_empty()
        || plugin_id.len() > 64
        || !plugin_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("非法插件 id".into());
    }
    let dir = plugins_root().join(&plugin_id);
    if !dir.is_dir() {
        return Err("插件目录不存在".into());
    }
    let files = collect_plugin_files(&dir);
    if files.is_empty() {
        return Err("插件目录是空的".into());
    }
    // manifest 里的 runtime 决定会话语言（系统提示词/SDK 注入按它走）
    let rt = mashiro_core::plugin::manifest::Manifest::load(&dir)
        .map(|m| match m.runtime {
            mashiro_core::plugin::manifest::Runtime::Python => "python".to_string(),
            _ => "node".to_string(),
        })
        .unwrap_or_else(|_| "node".to_string());
    let sid = session.unwrap_or_else(agent_sid);
    if !valid_sid(&sid) {
        return Err("非法会话 id".into());
    }
    let (msgs_snap, files_out) = {
        let mut sessions = state.agent_sessions.lock().unwrap();
        let s = sessions.entry(sid.clone()).or_insert_with(|| AgentSession {
            runtime: rt.clone(),
            title: format!("修改插件 {plugin_id}"),
            updated_at: 0,
            msgs: Vec::new(),
            workdir: None,
            files: Vec::new(),
        });
        s.runtime = rt.clone();
        if s.title.is_empty() {
            s.title = format!("修改插件 {plugin_id}");
        }
        // 工作区模式：真相在磁盘，files 只是给 UI 看的快照
        s.workdir = Some(dir.to_string_lossy().to_string());
        s.files = files;
        (s.msgs.clone(), s.files.clone())
    };
    persist_agent(&state, &sid, msgs_snap, files_out.clone());
    let files_json: Vec<_> = files_out
        .iter()
        .map(|(p, c)| json!({ "path": p, "content": c }))
        .collect();
    Ok(json!({ "session": sid, "files": files_json, "runtime": rt }))
}

/// 每指令触发权限：{cmd: {enabled, perm}}（perm: all/admin/owner）。
#[tauri::command]
pub fn plugin_cmd_cfg_get(
    state: State<'_, AppState>,
    plugin_id: String,
) -> Result<serde_json::Value, String> {
    Ok(json!(state.db.plugin_cmd_cfg(&plugin_id)))
}

#[tauri::command]
pub fn plugin_cmd_cfg_set(
    state: State<'_, AppState>,
    plugin_id: String,
    cfg: serde_json::Value,
) -> Result<(), String> {
    state.db.set_plugin_cmd_cfg(&plugin_id, cfg).map_err(e2s)
}

/// 找回某会话挂起中的 ask_user 提问（切页把弹窗弄丢了还能救回来）。
#[tauri::command]
pub fn agent_pending_ask(
    state: State<'_, AppState>,
    session: String,
) -> Result<Option<serde_json::Value>, String> {
    let answers = state.agent_answers.lock().unwrap();
    Ok(answers
        .iter()
        .find(|(_, p)| p.sid == session)
        .map(|(call_id, p)| json!({ "call_id": call_id, "questions": p.questions })))
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
