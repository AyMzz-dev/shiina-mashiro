use std::time::Duration;

use serde_json::{json, Value};

/// 正式环境 / 沙盒环境 API 域名（对应旧框架的「连接沙盒」开关）。
pub fn api_base(sandbox: bool) -> &'static str {
    if sandbox {
        "https://sandbox.api.sgroup.qq.com"
    } else {
        "https://api.sgroup.qq.com"
    }
}

/// 发送可选参数：被动回复 msg_id / event_id、键盘按钮、msg_seq 去重、
/// is_wakeup 互动召回、发送后自动撤回。
#[derive(Debug, Default, Clone)]
pub struct SendOpts {
    /// 被动回复用的来源消息 id（带上有频次豁免）
    pub msg_id: Option<String>,
    /// 被动回复事件 id（INTERACTION_CREATE / GROUP_ADD_ROBOT / GROUP_MSG_RECEIVE，与 msg_id 二选一）
    pub event_id: Option<String>,
    /// 同 msg_id 多次回复的去重序号（默认 1，相同 msg_id+msg_seq 重复发会失败）
    pub msg_seq: Option<i64>,
    /// true = 互动召回消息（不校验 msg_id/event_id 有效期；官方文档仅 C2C 定义）
    pub is_wakeup: bool,
    /// 引用回复 payload（{"message_id":"REFIDX_xxx"}），原样并入消息体
    pub message_reference: Option<Value>,
    /// 内联键盘 payload（{"id":"tpl"} 或 {"content":{"rows":[...]}}），原样并入消息体
    pub keyboard: Option<Value>,
    /// >0 表示发送 N 秒后自动撤回（主动消息用返回的 id 撤）
    pub recall_after_secs: i64,
}

/// 按会话类型解析发送目标路径。
///   0 群 / 1 频道 / 2 私聊(C2C) / 3 频道私聊 —— 与旧框架的 type 语义对齐。
fn base_path(base: &str, kind: i32, openid: &str) -> String {
    match kind {
        1 => format!("{base}/channels/{openid}"),
        2 => format!("{base}/v2/users/{openid}"),
        3 => format!("{base}/dms/{openid}"),
        _ => format!("{base}/v2/groups/{openid}"),
    }
}

/// 发送纯文本消息。opts.msg_id 带上可走被动回复（无频次限制）。
pub async fn send_text(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    kind: i32,
    openid: &str,
    content: &str,
    opts: SendOpts,
) -> anyhow::Result<Value> {
    let mut body = json!({ "content": content, "msg_type": 0 });
    apply_opts(&mut body, &opts, kind);
    let resp = post(
        http,
        token,
        &format!("{}/messages", base_path(base, kind, openid)),
        &body,
    )
    .await?;
    schedule_recall(http, token, base, kind, openid, &resp, &opts);
    Ok(resp)
}

/// 发送富媒体（URL 直传）：file_type 1 图片 / 2 视频 / 3 语音 / 4 文件。
/// 流程：POST /files 拿 file_info → POST /messages 携带 media。
/// 本地文件上传（素材/分片）属于 P3+ 的增强项。
pub async fn send_media(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    kind: i32,
    openid: &str,
    file_type: i32,
    url: &str,
    opts: SendOpts,
) -> anyhow::Result<Value> {
    let up = json!({ "file_type": file_type, "url": url, "srv_send_msg": false });
    let fi = post(
        http,
        token,
        &format!("{}/files", base_path(base, kind, openid)),
        &up,
    )
    .await?;
    let file_info = fi["file_info"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("files response missing file_info: {fi}"))?;

    let mut body = json!({ "msg_type": 7, "media": { "file_info": file_info }, "content": " " });
    apply_opts(&mut body, &opts, kind);
    let resp = post(
        http,
        token,
        &format!("{}/messages", base_path(base, kind, openid)),
        &body,
    )
    .await?;
    schedule_recall(http, token, base, kind, openid, &resp, &opts);
    Ok(resp)
}

/// 通用发送：body 原样 POST 到 {base_path}/messages。
/// 卡片(msg_type=8)/ark 等自定义结构由调用方构造 body（含 msg_id/event_id/msg_seq/keyboard）。
pub async fn post_send(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    kind: i32,
    openid: &str,
    body: &Value,
) -> anyhow::Result<Value> {
    post(
        http,
        token,
        &format!("{}/messages", base_path(base, kind, openid)),
        body,
    )
    .await
}

/// 发送 Markdown（msg_type 2）：群/C2C 走 content 模板文本。
pub async fn send_markdown(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    kind: i32,
    openid: &str,
    content: &str,
    opts: SendOpts,
) -> anyhow::Result<Value> {
    let mut body = json!({ "markdown": { "content": content }, "msg_type": 2 });
    apply_opts(&mut body, &opts, kind);
    let resp = post(
        http,
        token,
        &format!("{}/messages", base_path(base, kind, openid)),
        &body,
    )
    .await?;
    schedule_recall(http, token, base, kind, openid, &resp, &opts);
    Ok(resp)
}

/// 机器人群内状态：allow_proactive_msg / recv_msg_setting / member_role。
/// 群授权开关的**查询**入口（无代开 API，群主操作后此处可感知）。
pub async fn group_bot_state(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    group_openid: &str,
) -> anyhow::Result<Value> {
    get(
        http,
        token,
        &format!("{base}/v2/groups/{group_openid}/bot_state"),
    )
    .await
}

/// 机器人自身资料：GET /users/@me → {id, username, avatar, bot}。
/// READY 的 user.avatar 经常为空，用它兜底。
pub async fn bot_info(http: &reqwest::Client, token: &str, base: &str) -> anyhow::Result<Value> {
    get(http, token, &format!("{base}/users/@me")).await
}

// ---------- 频道(guild)域管理/查询 ----------
// 注意：QQ 群（v2/groups）官方 API 没有群管能力；以下接口只适用于频道场景。

pub async fn guild_info(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    guild_id: &str,
) -> anyhow::Result<Value> {
    get(http, token, &format!("{base}/guilds/{guild_id}")).await
}

pub async fn guild_member_list(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    guild_id: &str,
    limit: i64,
    after: &str,
) -> anyhow::Result<Value> {
    get(
        http,
        token,
        &format!(
            "{base}/guilds/{guild_id}/members?limit={limit}&after={}",
            urlencoding(after)
        ),
    )
    .await
}

pub async fn guild_member_info(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    guild_id: &str,
    user_id: &str,
) -> anyhow::Result<Value> {
    get(
        http,
        token,
        &format!("{base}/guilds/{guild_id}/members/{user_id}"),
    )
    .await
}

/// 移出频道成员。delete_child 附带删除其发言消息（可选）。
pub async fn guild_kick(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    guild_id: &str,
    user_id: &str,
    delete_msg_days: i64,
) -> anyhow::Result<()> {
    del(
        http,
        token,
        &format!(
            "{base}/guilds/{guild_id}/members/{user_id}?delete_message_days={delete_msg_days}"
        ),
    )
    .await
}

/// 撤回消息。
pub async fn recall(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    kind: i32,
    openid: &str,
    msg_id: &str,
) -> anyhow::Result<()> {
    let resp = send(
        http.delete(format!(
            "{}/messages/{msg_id}",
            base_path(base, kind, openid)
        ))
        .header("Authorization", format!("QQBot {token}")),
    )
    .await?;
    if !resp.status().is_success() {
        anyhow::bail!("recall failed {}: {}", resp.status(), resp.text().await?);
    }
    Ok(())
}

fn apply_opts(body: &mut Value, opts: &SendOpts, kind: i32) {
    if let Some(id) = &opts.msg_id {
        body["msg_id"] = json!(id);
    }
    if let Some(id) = &opts.event_id {
        body["event_id"] = json!(id);
    }
    if let Some(s) = opts.msg_seq {
        body["msg_seq"] = json!(s);
    }
    // is_wakeup（互动召回）官方文档仅 C2C 消息定义，其他场景不下发
    if opts.is_wakeup && kind == 2 {
        body["is_wakeup"] = json!(true);
    }
    if let Some(mr) = &opts.message_reference {
        body["message_reference"] = mr.clone();
    }
    if let Some(kb) = &opts.keyboard {
        body["keyboard"] = kb.clone();
    }
}

/// 自动撤回上限：对齐原版，超过 110s 按 110s 算。
const RECALL_MAX_SECS: u64 = 110;
/// 时间轮槽数：1 秒 1 槽，容纳最大撤回时长即可。
const RECALL_SLOTS: usize = 120;

struct RecallJob {
    http: reqwest::Client,
    token: String,
    base: String,
    kind: i32,
    openid: String,
    msg_id: String,
}

/// 撤回时间轮：常驻一条 1s tick 任务，到点批量取当前槽执行。
/// N 条待撤回消息只占 O(1) 常驻任务，不像「每条撤回 spawn 一个 sleep」。
struct RecallWheel {
    slots: Vec<std::sync::Mutex<Vec<RecallJob>>>,
    cursor: std::sync::atomic::AtomicUsize,
}

static RECALL_WHEEL: std::sync::OnceLock<std::sync::Arc<RecallWheel>> = std::sync::OnceLock::new();

fn recall_wheel() -> &'static std::sync::Arc<RecallWheel> {
    RECALL_WHEEL.get_or_init(|| {
        let w = std::sync::Arc::new(RecallWheel {
            slots: (0..RECALL_SLOTS)
                .map(|_| std::sync::Mutex::new(Vec::new()))
                .collect(),
            cursor: std::sync::atomic::AtomicUsize::new(0),
        });
        let w2 = std::sync::Arc::clone(&w);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            loop {
                let idx =
                    w2.cursor.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % RECALL_SLOTS;
                let jobs = std::mem::take(&mut *w2.slots[idx].lock().unwrap());
                for j in jobs {
                    tokio::spawn(async move {
                        if let Err(e) =
                            recall(&j.http, &j.token, &j.base, j.kind, &j.openid, &j.msg_id).await
                        {
                            tracing::warn!("auto-recall {}: {e}", j.msg_id);
                        }
                    });
                }
                tick.tick().await;
            }
        });
        w
    })
}

/// recall_after_secs>0：优先撤回刚发出的回复，入时间轮到点 DELETE。
fn schedule_recall(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    kind: i32,
    openid: &str,
    resp: &Value,
    opts: &SendOpts,
) {
    if opts.recall_after_secs <= 0 {
        return;
    }
    let mid = resp["id"]
        .as_str()
        .map(str::to_string)
        .or_else(|| resp["msg_id"].as_str().map(str::to_string))
        .or_else(|| opts.msg_id.clone());
    let Some(mid) = mid else {
        tracing::warn!("auto-recall: no msg_id available, skip");
        return;
    };
    let secs = (opts.recall_after_secs as u64).min(RECALL_MAX_SECS);
    let w = recall_wheel();
    let idx = (w.cursor.load(std::sync::atomic::Ordering::Relaxed) + secs as usize) % RECALL_SLOTS;
    w.slots[idx].lock().unwrap().push(RecallJob {
        http: http.clone(),
        token: token.to_string(),
        base: base.to_string(),
        kind,
        openid: openid.to_string(),
        msg_id: mid,
    });
}

/// QQ HTTP 并发闸（Tiffany max_inflight=16 同款）：插件风暴时排队拿许可，
/// 而不是无界并发直接撞平台频控；COS 分片上传等大流量也一起限。
static INFLIGHT: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(16);

/// 带并发许可的 send：超出 16 并发时在调用侧排队等待。
async fn send(req: reqwest::RequestBuilder) -> anyhow::Result<reqwest::Response> {
    let _permit = INFLIGHT
        .acquire()
        .await
        .map_err(|_| anyhow::anyhow!("inflight semaphore closed"))?;
    Ok(req.send().await?)
}

async fn get(http: &reqwest::Client, token: &str, url: &str) -> anyhow::Result<Value> {
    let resp = send(
        http.get(url)
            .header("Authorization", format!("QQBot {token}")),
    )
    .await?;
    let status = resp.status();
    let text = resp.text().await?;
    if !status.is_success() {
        anyhow::bail!("{status}: {text}");
    }
    Ok(serde_json::from_str(&text).unwrap_or(json!({ "raw": text })))
}

async fn del(http: &reqwest::Client, token: &str, url: &str) -> anyhow::Result<()> {
    let resp = send(
        http.delete(url)
            .header("Authorization", format!("QQBot {token}")),
    )
    .await?;
    if !resp.status().is_success() {
        anyhow::bail!("{}: {}", resp.status(), resp.text().await?);
    }
    Ok(())
}

async fn post(
    http: &reqwest::Client,
    token: &str,
    url: &str,
    body: &Value,
) -> anyhow::Result<Value> {
    let resp = send(
        http.post(url)
            .header("Authorization", format!("QQBot {token}"))
            .json(body),
    )
    .await?;
    let status = resp.status();
    let text = resp.text().await?;
    if !status.is_success() {
        anyhow::bail!("{status}: {text}");
    }
    Ok(serde_json::from_str(&text).unwrap_or(json!({ "raw": text })))
}

// ---------- Markdown 图片素材（/v2/media-images） ----------
// 独立域名 api.bot.qq.com（与消息发送的 sgroup 域无关），平台只收 png/jpg/gif/webp 且 <=10MB。
// 素材按 appid 隔离：配额 100 张 / 90 天滑动有效期 / qqbot-media:// 引用链接各自独立。
const MATERIAL_BASE: &str = "https://api.bot.qq.com";

/// 平铺 / 包一层 data 两种返回都认（文档示例平铺，网关偶尔会包）。
fn media_body(v: &Value) -> &Value {
    if v["data"].is_object() {
        &v["data"]
    } else {
        v
    }
}

fn material_mime(name: &str) -> Option<&'static str> {
    match name.rsplit('.').next()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// 素材域请求：鉴权头之外带 X-Union-Appid（对齐旧版 Post 的行为）。
async fn mat_req(req: reqwest::RequestBuilder, token: &str, appid: i64) -> anyhow::Result<Value> {
    let resp = send(
        req.header("Authorization", format!("QQBot {token}"))
            .header("X-Union-Appid", appid.to_string()),
    )
    .await?;
    let status = resp.status();
    let text = resp.text().await?;
    if !status.is_success() {
        anyhow::bail!("{status}: {text}");
    }
    Ok(serde_json::from_str(&text).unwrap_or(json!({ "raw": text })))
}

/// GET /v2/media-images —— 按上传时间倒序，翻页传上一页 next_cursor。
pub async fn material_list(
    http: &reqwest::Client,
    token: &str,
    appid: i64,
    cursor: &str,
) -> anyhow::Result<Value> {
    let mut url = format!("{MATERIAL_BASE}/v2/media-images");
    if !cursor.is_empty() {
        url.push_str(&format!("?cursor={}", urlencoding(cursor)));
    }
    let v = mat_req(http.get(url), token, appid).await?;
    Ok(media_body(&v).clone())
}

/// DELETE /v2/media-images/{image_id}/delete —— 释放配额；已发出的消息不受影响。
pub async fn material_delete(
    http: &reqwest::Client,
    token: &str,
    appid: i64,
    image_id: &str,
) -> anyhow::Result<()> {
    let url = format!(
        "{MATERIAL_BASE}/v2/media-images/{}/delete",
        urlencoding(image_id)
    );
    mat_req(http.delete(url), token, appid).await.map(|_| ())
}

/// 本地文件 → prepare → PUT presigned → complete 一条龙。
/// 返回 complete 响应（image_id / url=qqbot-media://… / file_name 等）。
pub async fn material_upload(
    http: &reqwest::Client,
    token: &str,
    appid: i64,
    path: &std::path::Path,
) -> anyhow::Result<Value> {
    let data =
        std::fs::read(path).map_err(|e| anyhow::anyhow!("打不开文件 {}: {e}", path.display()))?;
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "image.png".into());
    anyhow::ensure!(!data.is_empty(), "{name} 是空文件");
    anyhow::ensure!(
        data.len() <= 10 * 1024 * 1024,
        "{name} 有 {:.1}MB，超过平台 10MB 上限",
        data.len() as f64 / 1048576.0
    );
    let mime = material_mime(&name)
        .ok_or_else(|| anyhow::anyhow!("{name} 不是支持的格式（只收 png/jpg/gif/webp）"))?;
    // 摘要平台必填；complete 时服务端拿 md5 与 COS ETag 比对
    use sha1::Digest;
    let md5 = hex::encode(md5::compute(&data).0);
    let sha1 = hex::encode(sha1::Sha1::digest(&data));

    // 1. 申请上传：upload_id 4 小时内有效
    let prep = mat_req(
        http.post(format!("{MATERIAL_BASE}/v2/media-images/prepare"))
            .json(&json!({
                "file_name": name,
                "file_size": data.len(),
                "content_type": mime,
                "md5": md5,
                "sha1": sha1,
            })),
        token,
        appid,
    )
    .await
    .map_err(|e| anyhow::anyhow!("申请上传失败: {e:#}"))?;
    let prep = media_body(&prep);
    let upload_id = prep["upload_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("prepare 缺少 upload_id: {prep}"))?;
    let presigned = prep["presigned_url"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("prepare 缺少 presigned_url: {prep}"))?;

    // 2. 直传 COS：不带 QQ 鉴权头（签名在 URL query），Content-Type 必须与 prepare 一致
    let resp = send(http.put(presigned).header("Content-Type", mime).body(data)).await?;
    if !resp.status().is_success() {
        anyhow::bail!("上传图片数据失败 {}: {}", resp.status(), resp.text().await?);
    }

    // 3. 完成上传：md5 与 COS ETag 不一致会报 40035003
    let done = mat_req(
        http.post(format!("{MATERIAL_BASE}/v2/media-images/complete"))
            .json(&json!({ "upload_id": upload_id, "md5": md5 })),
        token,
        appid,
    )
    .await
    .map_err(|e| anyhow::anyhow!("完成上传失败: {e:#}"))?;
    let done = media_body(&done);
    anyhow::ensure!(
        !done["image_id"].as_str().unwrap_or_default().is_empty()
            && !done["url"].as_str().unwrap_or_default().is_empty(),
        "complete 缺少 image_id/url: {done}"
    );
    Ok(done.clone())
}

/// URL query 转义（cursor/image_id 里可能有特殊字符）。
fn urlencoding(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

async fn put_json(
    http: &reqwest::Client,
    token: &str,
    url: &str,
    body: &Value,
) -> anyhow::Result<Value> {
    let resp = send(
        http.put(url)
            .header("Authorization", format!("QQBot {token}"))
            .json(body),
    )
    .await?;
    let status = resp.status();
    let text = resp.text().await?;
    if !status.is_success() {
        anyhow::bail!("{status}: {text}");
    }
    Ok(serde_json::from_str(&text).unwrap_or(json!({ "raw": text })))
}

async fn patch_json(
    http: &reqwest::Client,
    token: &str,
    url: &str,
    body: &Value,
) -> anyhow::Result<Value> {
    let resp = send(
        http.patch(url)
            .header("Authorization", format!("QQBot {token}"))
            .json(body),
    )
    .await?;
    let status = resp.status();
    let text = resp.text().await?;
    if !status.is_success() {
        anyhow::bail!("{status}: {text}");
    }
    Ok(serde_json::from_str(&text).unwrap_or(json!({ "raw": text })))
}

// ---------- v2 群管理（QQ 群官方群管 API，需机器人为群管理员） ----------

/// 群资料：群名/人数/标签/union_group_openid 等。
pub async fn group_info(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    group_openid: &str,
) -> anyhow::Result<Value> {
    get(
        http,
        token,
        &format!("{base}/v2/groups/{group_openid}/info"),
    )
    .await
}

/// 群成员列表（cursor 分页，翻页传上次响应的 next_cursor）。
pub async fn group_members(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    group_openid: &str,
    cursor: &str,
) -> anyhow::Result<Value> {
    let mut url = format!("{base}/v2/groups/{group_openid}/members");
    if !cursor.is_empty() {
        url.push_str(&format!("?cursor={}", urlencoding(cursor)));
    }
    get(http, token, &url).await
}

/// 单个群成员资料。
pub async fn group_member(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    group_openid: &str,
    member_openid: &str,
) -> anyhow::Result<Value> {
    get(
        http,
        token,
        &format!("{base}/v2/groups/{group_openid}/members/{member_openid}"),
    )
    .await
}

/// 群黑名单列表（cursor/limit 分页）。
pub async fn group_blacklist(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    group_openid: &str,
    cursor: &str,
    limit: i64,
) -> anyhow::Result<Value> {
    let mut url = format!("{base}/v2/groups/{group_openid}/member_blacklist?limit={limit}");
    if !cursor.is_empty() {
        url.push_str(&format!("&cursor={}", urlencoding(cursor)));
    }
    get(http, token, &url).await
}

/// 群黑名单操作：op "add" / "del"，单次最多 20 个 member_openid。
pub async fn group_blacklist_op(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    group_openid: &str,
    op: &str,
    member_openids: &[String],
) -> anyhow::Result<Value> {
    post(
        http,
        token,
        &format!("{base}/v2/groups/{group_openid}/member_blacklist"),
        &json!({ "op": op, "member_openids": member_openids }),
    )
    .await
}

/// 批量移出群成员，返回 {remove_members_result:"success"}。
pub async fn group_remove_members(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    group_openid: &str,
    member_openids: &[String],
) -> anyhow::Result<Value> {
    post(
        http,
        token,
        &format!("{base}/v2/groups/{group_openid}/batch_remove_members"),
        &json!({ "member_openids": member_openids }),
    )
    .await
}

/// 群禁言状态：全局规则 global_rule + 被禁言成员 members。
pub async fn group_mute_setting(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    group_openid: &str,
) -> anyhow::Result<Value> {
    get(
        http,
        token,
        &format!("{base}/v2/groups/{group_openid}/restrict_chat_setting"),
    )
    .await
}

/// 设置成员级禁言。members 每项 {op:"add|update|del", member_openid, mute_expire_at(RFC3339)}，
/// 单次最多 20 项；只能操作普通成员，最长 30 天。
pub async fn group_mute_members(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    group_openid: &str,
    members: &Value,
) -> anyhow::Result<Value> {
    post(
        http,
        token,
        &format!("{base}/v2/groups/{group_openid}/restrict_chat_setting"),
        &json!({ "members": members }),
    )
    .await
}

/// 入群申请列表（cursor/limit 分页，机器人需为群管理员）。
pub async fn group_join_requests(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    group_openid: &str,
    cursor: &str,
    limit: i64,
) -> anyhow::Result<Value> {
    let mut url = format!("{base}/v2/groups/{group_openid}/join_request_list?limit={limit}");
    if !cursor.is_empty() {
        url.push_str(&format!("&cursor={}", urlencoding(cursor)));
    }
    get(http, token, &url).await
}

/// 入群申请审批：op "approve" 通过 / "decline" 拒绝；
/// join_request_id 来自申请列表；reject_reason/add_to_member_blacklist 仅 decline 有效。
pub async fn group_approve_join(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    group_openid: &str,
    member_openid: &str,
    op: &str,
    join_request_id: Option<&str>,
    reject_reason: Option<&str>,
    add_to_member_blacklist: bool,
) -> anyhow::Result<Value> {
    let mut body = json!({ "op": op });
    if let Some(id) = join_request_id.filter(|s| !s.is_empty()) {
        body["join_request_id"] = json!(id);
    }
    if let Some(r) = reject_reason.filter(|s| !s.is_empty()) {
        body["reject_reason"] = json!(r);
    }
    if add_to_member_blacklist {
        body["add_to_member_blacklist"] = json!(true);
    }
    post(
        http,
        token,
        &format!("{base}/v2/groups/{group_openid}/approval_join_request/{member_openid}"),
        &body,
    )
    .await
}

// ---------- 入群自动审批策略（/v2/groups/join_approval_strategy） ----------

/// 策略列表（cursor/limit 分页）。
pub async fn join_strategy_list(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    cursor: &str,
    limit: i64,
) -> anyhow::Result<Value> {
    let mut url = format!("{base}/v2/groups/join_approval_strategy?limit={limit}");
    if !cursor.is_empty() {
        url.push_str(&format!("&cursor={}", urlencoding(cursor)));
    }
    get(http, token, &url).await
}

/// 创建策略：body 至少含 group_openids 或 group_ids 其一；
/// 可选 is_enable("on"/"off") / expire_at(RFC3339) / remark。
pub async fn join_strategy_create(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    body: &Value,
) -> anyhow::Result<Value> {
    post(
        http,
        token,
        &format!("{base}/v2/groups/join_approval_strategy"),
        body,
    )
    .await
}

/// 修改策略：is_enable / expire_at / group_action{op,group_openids|group_ids} / remark。
pub async fn join_strategy_patch(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    strategy_id: &str,
    body: &Value,
) -> anyhow::Result<Value> {
    patch_json(
        http,
        token,
        &format!("{base}/v2/groups/join_approval_strategy/{strategy_id}"),
        body,
    )
    .await
}

pub async fn join_strategy_delete(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    strategy_id: &str,
) -> anyhow::Result<()> {
    del(
        http,
        token,
        &format!("{base}/v2/groups/join_approval_strategy/{strategy_id}"),
    )
    .await
}

/// 手动全量执行策略：对关联群扫描，命中白名单的入群申请自动通过（约 10 分钟）。
pub async fn join_strategy_execute(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    strategy_id: &str,
) -> anyhow::Result<Value> {
    post(
        http,
        token,
        &format!("{base}/v2/groups/join_approval_strategy/{strategy_id}/execute"),
        &json!({}),
    )
    .await
}

/// 白名单 QQ 号码批量增删：op "add"|"del"，单次最多 10000 个。
pub async fn join_strategy_whitelist(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    strategy_id: &str,
    op: &str,
    whitelist_users: &[String],
) -> anyhow::Result<Value> {
    post(
        http,
        token,
        &format!("{base}/v2/groups/join_approval_strategy/{strategy_id}/whitelist_users"),
        &json!({ "op": op, "whitelist_users": whitelist_users }),
    )
    .await
}

// ---------- 交互应答 / 用户信息 / 分享链接 ----------

/// INTERACTION_CREATE 事件应答（type=11 按钮 / type=12 菜单回调必须调，否则客户端一直 loading）。
/// interaction_id 取事件 d.id（不带 "INTERACTION_CREATE:" 前缀），code: 0 成功/1 失败/2 频繁/3 重复/4 无权限/5 仅管理员。
pub async fn interaction_respond(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    interaction_id: &str,
    code: i64,
) -> anyhow::Result<Value> {
    put_json(
        http,
        token,
        &format!("{base}/interactions/{interaction_id}"),
        &json!({ "code": code }),
    )
    .await
}

/// 用户基本资料：username / avatar / union_openid / allow_proactive_msg。
pub async fn user_info(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    user_openid: &str,
) -> anyhow::Result<Value> {
    get(http, token, &format!("{base}/v2/users/{user_openid}/info")).await
}

/// 生成机器人分享链接（callback_data ≤32 字符，随加好友回传）。
pub async fn generate_url_link(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    callback_data: &str,
) -> anyhow::Result<Value> {
    post(
        http,
        token,
        &format!("{base}/v2/generate_url_link"),
        &json!({ "callback_data": callback_data }),
    )
    .await
}

// ---------- 私聊菜单 / 指令面板 ----------

/// 查询全局自定义菜单（仅 C2C 场景生效）：{version, menu:{items}}。
pub async fn menu_get(http: &reqwest::Client, token: &str, base: &str) -> anyhow::Result<Value> {
    get(http, token, &format!("{base}/v2/menu")).await
}

/// 覆盖式修改全局自定义菜单。menu 结构 {items:[{name,type,send_message|link|switch|sub_menu_items}]}。
pub async fn menu_put(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    menu: &Value,
) -> anyhow::Result<Value> {
    put_json(
        http,
        token,
        &format!("{base}/v2/menu"),
        &json!({ "menu": menu }),
    )
    .await
}

/// 指令面板列表：scope = c2c/group/channel/dm，cursor 分页。
pub async fn panels_list(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    scope: &str,
    cursor: &str,
    limit: i64,
) -> anyhow::Result<Value> {
    let mut url = format!(
        "{base}/v2/panels?scope={}&limit={limit}",
        urlencoding(scope)
    );
    if !cursor.is_empty() {
        url.push_str(&format!("&cursor={}", urlencoding(cursor)));
    }
    get(http, token, &url).await
}

/// 创建指令面板：body {scope, target_type?, user_openids?, group_openids?, panel:{items,remark}}。
pub async fn panel_create(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    body: &Value,
) -> anyhow::Result<Value> {
    post(http, token, &format!("{base}/v2/panels"), body).await
}

/// 查询单个面板详情。
pub async fn panel_get(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    panel_id: &str,
) -> anyhow::Result<Value> {
    get(http, token, &format!("{base}/v2/panels/{panel_id}")).await
}

/// 覆盖式更新面板内容 {panel:{items,remark}}。
pub async fn panel_update(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    panel_id: &str,
    body: &Value,
) -> anyhow::Result<Value> {
    put_json(http, token, &format!("{base}/v2/panels/{panel_id}"), body).await
}

pub async fn panel_delete(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    panel_id: &str,
) -> anyhow::Result<()> {
    del(http, token, &format!("{base}/v2/panels/{panel_id}")).await
}

/// 面板关联对象增删：op "add"|"del"，c2c 场景传 user_openids、group 场景传 group_openids，单次 ≤20。
pub async fn panel_target(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    panel_id: &str,
    op: &str,
    user_openids: &[String],
    group_openids: &[String],
) -> anyhow::Result<Value> {
    let mut body = json!({ "op": op });
    if !user_openids.is_empty() {
        body["user_openids"] = json!(user_openids);
    }
    if !group_openids.is_empty() {
        body["group_openids"] = json!(group_openids);
    }
    put_json(
        http,
        token,
        &format!("{base}/v2/panels/{panel_id}/target"),
        &body,
    )
    .await
}

// ---------- 单聊流式消息 ----------

/// 流式消息分块发送（仅 C2C）。msg 字段：input_mode/input_state/index/
/// content_type/content_raw + event_id 或 msg_id；首块不带 stream_msg_id，
/// 响应的 id 即 stream_msg_id，后续分块回传同一值。input_state: 1=生成中 10=结束。
pub async fn send_stream(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    user_openid: &str,
    msg: &Value,
) -> anyhow::Result<Value> {
    post(
        http,
        token,
        &format!("{base}/v2/users/{user_openid}/stream_messages"),
        msg,
    )
    .await
}

// ---------- 本地文件发送（分片上传 → files 合并 → msg_type 7） ----------

/// 本地文件发送为富媒体消息（群 kind=0 / 单聊 kind=2）。
/// 流程：upload_prepare → 逐片 PUT 预签名 URL + upload_part_finish →
/// /files{upload_id} 合并拿 file_info → /messages msg_type=7。
/// 软限制：图片 20MB / 视频 30MB / 语音 20MB / 文件 200MB（超限平台降级为文件类型）。
pub async fn send_media_file(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    kind: i32,
    openid: &str,
    file_type: i32,
    path: &std::path::Path,
    opts: SendOpts,
) -> anyhow::Result<Value> {
    anyhow::ensure!(kind == 0 || kind == 2, "本地文件上传仅支持群/单聊场景");
    let data = tokio::fs::read(path)
        .await
        .map_err(|e| anyhow::anyhow!("打不开文件 {}: {e}", path.display()))?;
    anyhow::ensure!(!data.is_empty(), "{} 是空文件", path.display());
    anyhow::ensure!(
        data.len() <= 200 * 1024 * 1024,
        "{} 有 {:.1}MB，超过平台 200MB 硬限制",
        path.display(),
        data.len() as f64 / 1048576.0
    );
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    let file_info = upload_chunked(
        http, token, base, kind, openid, file_type, &file_name, &data,
    )
    .await?;

    let mut body = json!({ "msg_type": 7, "media": { "file_info": file_info }, "content": " " });
    apply_opts(&mut body, &opts, kind);
    let resp = post(
        http,
        token,
        &format!("{}/messages", base_path(base, kind, openid)),
        &body,
    )
    .await?;
    schedule_recall(http, token, base, kind, openid, &resp, &opts);
    Ok(resp)
}

/// 本地文件仅上传取 file_info（不直接发消息），返回 files 接口响应 {file_uuid,file_info,ttl}。
pub async fn upload_file(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    kind: i32,
    openid: &str,
    file_type: i32,
    path: &std::path::Path,
) -> anyhow::Result<Value> {
    anyhow::ensure!(kind == 0 || kind == 2, "本地文件上传仅支持群/单聊场景");
    let data =
        std::fs::read(path).map_err(|e| anyhow::anyhow!("打不开文件 {}: {e}", path.display()))?;
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    upload_chunked(
        http, token, base, kind, openid, file_type, &file_name, &data,
    )
    .await
    .map(|fi| json!({ "file_info": fi }))
}

/// 分片上传合并且取回 file_info。
async fn upload_chunked(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    kind: i32,
    openid: &str,
    file_type: i32,
    file_name: &str,
    data: &[u8],
) -> anyhow::Result<String> {
    use sha1::Digest;
    let target = base_path(base, kind, openid);
    let md5 = hex::encode(md5::compute(data).0);
    let sha1 = hex::encode(sha1::Sha1::digest(data));
    // md5_10m 官方定义为文件前 10002432 字节（约 10MB）的 MD5，不是 10MiB
    let head = &data[..data.len().min(10_002_432)];
    let md5_10m = hex::encode(md5::compute(head).0);

    // 1. 预上传：拿到 upload_id 与每片预签名 URL
    let prep = post(
        http,
        token,
        &format!("{target}/upload_prepare"),
        &json!({
            "file_type": file_type,
            "file_size": data.len().to_string(),
            "file_name": file_name,
            "md5": md5,
            "sha1": sha1,
            "md5_10m": md5_10m,
        }),
    )
    .await
    .map_err(|e| anyhow::anyhow!("upload_prepare 失败: {e:#}"))?;
    let upload_id = prep["upload_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("upload_prepare 缺少 upload_id: {prep}"))?;
    let mut parts: Vec<(i64, String, usize)> = prep["parts"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("upload_prepare 缺少 parts: {prep}"))?
        .iter()
        .filter_map(|p| {
            let index = p["index"].as_i64()?;
            let url = p["presigned_url"].as_str()?.to_string();
            let bs = p["block_size"].as_str()?.parse::<usize>().ok()?;
            Some((index, url, bs))
        })
        .collect();
    parts.sort_by_key(|p| p.0);
    anyhow::ensure!(!parts.is_empty(), "upload_prepare 返回空 parts: {prep}");

    // 2. 逐片 PUT 预签名 URL（不带 QQ 鉴权头），成功后调 part_finish 知会服务端
    let mut offset = 0usize;
    for (index, url, bs) in &parts {
        let end = (offset + *bs).min(data.len());
        let chunk = &data[offset.min(end)..end];
        let resp = send(http.put(url).body(chunk.to_vec())).await?;
        if !resp.status().is_success() {
            anyhow::bail!(
                "分片 {index} PUT 失败 {}: {}",
                resp.status(),
                resp.text().await?
            );
        }
        post(
            http,
            token,
            &format!("{target}/upload_part_finish"),
            &json!({
                "upload_id": upload_id,
                "part_index": index,
                "block_size": chunk.len().to_string(),
                "md5": hex::encode(md5::compute(chunk).0),
            }),
        )
        .await
        .map_err(|e| anyhow::anyhow!("part {index} finish 失败: {e:#}"))?;
        offset = end;
    }

    // 3. /files 合并分片 → file_info
    let merged = post(
        http,
        token,
        &format!("{target}/files"),
        &json!({
            "file_type": file_type,
            "srv_send_msg": false,
            "file_name": file_name,
            "upload_id": upload_id,
        }),
    )
    .await
    .map_err(|e| anyhow::anyhow!("分片合并失败: {e:#}"))?;
    merged["file_info"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("files 响应缺少 file_info: {merged}"))
}
