use std::sync::Mutex;
use std::time::{Duration, Instant};

const TOKEN_URL: &str = "https://bots.qq.com/app/getAppAccessToken";
/// 提前 5 分钟认为过期，避免用到边界 token。
const EXPIRE_MARGIN: Duration = Duration::from_secs(300);

#[derive(serde::Deserialize)]
struct TokenResp {
    access_token: String,
    /// 平台返回的是**字符串**秒数，如 "7200"。
    expires_in: serde_json::Value,
}

/// access_token 缓存：同一个 appid 只在这里刷新一次，所有调用方共享。
pub struct TokenStore {
    inner: Mutex<Option<(String, Instant)>>,
}

impl TokenStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    pub async fn get(
        &self,
        http: &reqwest::Client,
        appid: i64,
        secret: &str,
    ) -> anyhow::Result<String> {
        {
            let guard = self.inner.lock().unwrap();
            if let Some((t, expire_at)) = guard.as_ref() {
                if Instant::now() < *expire_at {
                    return Ok(t.clone());
                }
            }
        }

        let resp = http
            .post(TOKEN_URL)
            .json(&serde_json::json!({
                "appId": appid.to_string(),
                "clientSecret": secret,
            }))
            .timeout(Duration::from_secs(15))
            .send()
            .await?
            .error_for_status()?
            .json::<TokenResp>()
            .await?;

        let secs = resp
            .expires_in
            .as_str()
            .and_then(|s| s.parse::<u64>().ok())
            .or_else(|| resp.expires_in.as_u64())
            .unwrap_or(7200);
        // expires_in 小于提前量时下溢会 panic；保底 30s 缓存，避免每调用都去打 token 接口
        let ttl = secs.saturating_sub(EXPIRE_MARGIN.as_secs()).max(30);
        let expire_at = Instant::now() + Duration::from_secs(ttl);

        let mut guard = self.inner.lock().unwrap();
        *guard = Some((resp.access_token.clone(), expire_at));
        Ok(resp.access_token)
    }
}
