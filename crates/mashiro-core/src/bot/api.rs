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

/// 发送可选参数：被动回复 msg_id / 键盘按钮 / 发送后自动撤回。
#[derive(Debug, Default, Clone)]
pub struct SendOpts {
    /// 被动回复用的来源消息 id（带上有频次豁免）
    pub msg_id: Option<String>,
    /// 内联键盘 payload（{"content":{"rows":[...]}}），原样并入消息体
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
        3 => format!("{base}/dms/guilds/{openid}"),
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
    apply_opts(&mut body, &opts);
    let resp = post(http, token, &format!("{}/messages", base_path(base, kind, openid)), &body).await?;
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
    let fi = post(http, token, &format!("{}/files", base_path(base, kind, openid)), &up).await?;
    let file_info = fi["file_info"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("files response missing file_info: {fi}"))?;

    let mut body = json!({ "msg_type": 7, "media": { "file_info": file_info }, "content": " " });
    apply_opts(&mut body, &opts);
    let resp = post(http, token, &format!("{}/messages", base_path(base, kind, openid)), &body).await?;
    schedule_recall(http, token, base, kind, openid, &resp, &opts);
    Ok(resp)
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
    apply_opts(&mut body, &opts);
    let resp = post(http, token, &format!("{}/messages", base_path(base, kind, openid)), &body).await?;
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
    get(http, token, &format!("{base}/v2/groups/{group_openid}/bot_state")).await
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
        &format!("{base}/guilds/{guild_id}/members?limit={limit}&after={after}"),
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
    get(http, token, &format!("{base}/guilds/{guild_id}/members/{user_id}")).await
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
    let resp = http
        .delete(format!("{}/messages/{msg_id}", base_path(base, kind, openid)))
        .header("Authorization", format!("QQBot {token}"))
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("recall failed {}: {}", resp.status(), resp.text().await?);
    }
    Ok(())
}

fn apply_opts(body: &mut Value, opts: &SendOpts) {
    if let Some(id) = &opts.msg_id {
        body["msg_id"] = json!(id);
    }
    if let Some(kb) = &opts.keyboard {
        body["keyboard"] = kb.clone();
    }
}

/// recall_after_secs>0：优先用显式 msg_id（被动），否则取返回里的 id，延时撤回。
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
    let mid = opts
        .msg_id
        .clone()
        .or_else(|| resp["id"].as_str().map(str::to_string))
        .or_else(|| resp["msg_id"].as_str().map(str::to_string));
    let Some(mid) = mid else {
        tracing::warn!("auto-recall: no msg_id available, skip");
        return;
    };
    let (http, token, base, openid) = (
        http.clone(),
        token.to_string(),
        base.to_string(),
        openid.to_string(),
    );
    let secs = opts.recall_after_secs as u64;
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(secs)).await;
        if let Err(e) = recall(&http, &token, &base, kind, &openid, &mid).await {
            tracing::warn!("auto-recall {mid}: {e}");
        }
    });
}

async fn get(http: &reqwest::Client, token: &str, url: &str) -> anyhow::Result<Value> {
    let resp = http
        .get(url)
        .header("Authorization", format!("QQBot {token}"))
        .send()
        .await?;
    let status = resp.status();
    let text = resp.text().await?;
    if !status.is_success() {
        anyhow::bail!("{status}: {text}");
    }
    Ok(serde_json::from_str(&text).unwrap_or(json!({ "raw": text })))
}

async fn del(http: &reqwest::Client, token: &str, url: &str) -> anyhow::Result<()> {
    let resp = http
        .delete(url)
        .header("Authorization", format!("QQBot {token}"))
        .send()
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
    let resp = http
        .post(url)
        .header("Authorization", format!("QQBot {token}"))
        .json(body)
        .send()
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
async fn mat_req(
    req: reqwest::RequestBuilder,
    token: &str,
    appid: i64,
) -> anyhow::Result<Value> {
    let resp = req
        .header("Authorization", format!("QQBot {token}"))
        .header("X-Union-Appid", appid.to_string())
        .send()
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
    let url = format!("{MATERIAL_BASE}/v2/media-images/{}/delete", urlencoding(image_id));
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
    let data = std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("打不开文件 {}: {e}", path.display()))?;
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
        http.post(format!("{MATERIAL_BASE}/v2/media-images/prepare")).json(&json!({
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
    let resp = http
        .put(presigned)
        .header("Content-Type", mime)
        .body(data)
        .send()
        .await?;
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
