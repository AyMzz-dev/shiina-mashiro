//! 扫码绑定登录（对齐旧框架 QQBindLogin）：
//!   1. 本地生成 32B 随机 key → POST create_bind_task 拿 task_id
//!   2. 用户扫码打开绑定页 → POST poll_bind_result 轮询
//!   3. status=2 时返回 appid + AES-256-GCM 加密的 secret，用本地 key 解密落库

use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use rand::RngCore;
use serde_json::{json, Value};

const CREATE_URL: &str = "https://q.qq.com/lite/create_bind_task";
const POLL_URL: &str = "https://q.qq.com/lite/poll_bind_result";
const QR_PAGE: &str = "https://q.qq.com/qqbot/openclaw/connect.html?source=mashiro&_wv=2";
/// 绑定会话有效期（二维码过期时间）
pub const SESSION_TTL_MS: i64 = 120_000;

const UA: &str = "Mozilla/5.0 (Linux; U; Android 14; zh-cn) AppleWebKit/537.36 \
                  Chrome/109.0.5414.118 Mobile Safari/537.36";

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// 一次绑定会话：task_id + 本地密钥 + 创建时间。
#[derive(Clone)]
pub struct BindTask {
    pub task_id: String,
    pub qr_url: String,
    key: [u8; 32],
    pub created_ms: i64,
}

/// 第一步：创建绑定任务，返回会话（qr_url 拿去渲染二维码）。
pub async fn create_task(http: &reqwest::Client) -> Result<BindTask> {
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);

    let resp: Value = http
        .post(CREATE_URL)
        .header("User-Agent", UA)
        .header("Origin", "https://q.qq.com")
        .header("Referer", "https://q.qq.com/")
        .json(&json!({ "key": B64.encode(key) }))
        .send()
        .await
        .context("create_bind_task")?
        .json()
        .await
        .context("create_bind_task parse")?;

    if resp["retcode"].as_i64().unwrap_or(-1) != 0 {
        bail!(
            "create_bind_task: {}",
            resp["msg"].as_str().unwrap_or("失败")
        );
    }
    let task_id = resp["data"]["task_id"]
        .as_str()
        .ok_or_else(|| anyhow!("create_bind_task: missing task_id"))?
        .to_string();

    Ok(BindTask {
        qr_url: format!("{QR_PAGE}&task_id={task_id}"),
        task_id,
        key,
        created_ms: chrono::Utc::now().timestamp_millis(),
    })
}

pub enum PollResult {
    Waiting,
    Expired,
    Done { appid: i64, secret: String },
}

/// 第二步：轮询扫码结果。status 1 等待 / 3 过期 / 2 完成（解密 secret）。
pub async fn poll(http: &reqwest::Client, task: &BindTask) -> Result<PollResult> {
    let resp: Value = http
        .post(POLL_URL)
        .header("User-Agent", UA)
        .header("Origin", "https://q.qq.com")
        .header("Referer", "https://q.qq.com/")
        .json(&json!({ "task_id": task.task_id }))
        .send()
        .await
        .context("poll_bind_result")?
        .json()
        .await
        .context("poll_bind_result parse")?;

    if resp["retcode"].as_i64().unwrap_or(-1) != 0 {
        // 服务抖动 → 下次重试，不算错误
        return Ok(PollResult::Waiting);
    }
    let data = &resp["data"];
    match data["status"].as_i64().unwrap_or(1) {
        1 => Ok(PollResult::Waiting),
        3 => Ok(PollResult::Expired),
        2 => {
            let appid: i64 = data["bot_appid"]
                .as_str()
                .and_then(|s| s.parse().ok())
                .or_else(|| data["bot_appid"].as_i64())
                .ok_or_else(|| anyhow!("bind result missing bot_appid"))?;
            let enc = B64
                .decode(data["bot_encrypt_secret"].as_str().unwrap_or_default())
                .context("bot_encrypt_secret base64")?;
            if enc.len() < 12 + 16 {
                bail!("bot_encrypt_secret too short");
            }
            let cipher = Aes256Gcm::new_from_slice(&task.key).context("cipher init")?;
            let plain = cipher
                .decrypt(Nonce::from_slice(&enc[..12]), &enc[12..])
                .map_err(|_| anyhow!("secret 解密失败（key 与密文不匹配）"))?;
            Ok(PollResult::Done {
                appid,
                secret: String::from_utf8(plain).context("secret utf8")?,
            })
        }
        s => bail!("poll_bind_result unknown status {s}"),
    }
}
