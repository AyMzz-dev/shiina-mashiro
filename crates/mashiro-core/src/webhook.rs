//! Webhook 接入（P4）：设置 `webhook_listen`（如 `0.0.0.0:8464`）后启动 axum 服务，
//! 端点 `POST /qq/callback/{appid}`。
//!
//! 签名模型（按官方文档/Go SDK）：
//!   seed      = secret 重复至 ≥32 字节取前 32
//!   keypair   = Ed25519 from seed（双方同密钥对）
//!   验签      = verify(public, X-Signature-Ed25519, timestamp + body)
//!   op13 回签 = sign(private, seed_str + event_ts)

use std::sync::{Arc, OnceLock};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;
use axum::routing::post;
use axum::Router;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier};
use serde_json::{json, Value};

use crate::router::dispatch::Dispatcher;
use crate::router::event;
use crate::storage::db::Database;

struct WebhookCtx {
    db: Arc<Database>,
    dispatcher: Arc<OnceLock<Dispatcher>>,
}

/// 从 secret 派生 Ed25519 密钥对。
struct Keys {
    /// 32 字节 seed（同时作为 op13 签名的前半段字符串）
    seed: Vec<u8>,
    signing: SigningKey,
}

impl Keys {
    fn from_secret(secret: &str) -> Self {
        let mut s = secret.as_bytes().to_vec();
        while s.len() < ed25519_dalek::SECRET_KEY_LENGTH {
            let dup = s.clone();
            s.extend_from_slice(&dup);
        }
        s.truncate(ed25519_dalek::SECRET_KEY_LENGTH);
        let seed: [u8; ed25519_dalek::SECRET_KEY_LENGTH] =
            s[..ed25519_dalek::SECRET_KEY_LENGTH].try_into().expect("32 bytes");
        Self {
            seed: seed.to_vec(),
            signing: SigningKey::from_bytes(&seed),
        }
    }

    fn verify(&self, ts: &str, sig_hex: &str, body: &[u8]) -> bool {
        let Ok(sig_bytes) = hex::decode(sig_hex) else {
            return false;
        };
        let Ok(sig) = Signature::from_slice(&sig_bytes) else {
            return false;
        };
        let mut msg = ts.as_bytes().to_vec();
        msg.extend_from_slice(body);
        self.signing
            .verifying_key()
            .verify(&msg, &sig)
            .is_ok()
    }

    /// op13 回调地址验证：sign(seed + event_ts)，hex 输出。
    fn sign_validation(&self, event_ts: &str) -> String {
        let mut msg = self.seed.clone();
        msg.extend_from_slice(event_ts.as_bytes());
        hex::encode(self.signing.sign(&msg).to_bytes())
    }
}

/// 设置里有 webhook_listen 才启动；空 = 纯 WS 模式（默认）。
pub fn start(db: Arc<Database>, dispatcher: Arc<OnceLock<Dispatcher>>) {
    let addr = db.get_setting("webhook_listen").unwrap_or_default();
    if addr.trim().is_empty() {
        return;
    }
    tokio::spawn(async move {
        let ctx = Arc::new(WebhookCtx { db, dispatcher });
        let app = Router::new()
            .route("/qq/callback/{appid}", post(callback))
            .with_state(ctx);
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(l) => {
                tracing::info!("webhook listening on {addr} → /qq/callback/{{appid}}");
                if let Err(e) = axum::serve(l, app).await {
                    tracing::error!("webhook serve: {e}");
                }
            }
            Err(e) => tracing::error!("webhook bind {addr}: {e}"),
        }
    });
}

async fn callback(
    State(ctx): State<Arc<WebhookCtx>>,
    Path(appid): Path<i64>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, (StatusCode, String)> {
    let e = |s: &str| (StatusCode::BAD_REQUEST, s.to_string());
    let acc = ctx
        .db
        .get_account(appid)
        .map_err(|x| (StatusCode::INTERNAL_SERVER_ERROR, x.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "unknown appid".to_string()))?;
    let keys = Keys::from_secret(&acc.secret);

    let h = |k: &str| {
        headers
            .get(k)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
            .ok_or_else(|| e(&format!("missing header {k}")))
    };
    let ts = h("x-signature-timestamp")?;
    let sig = h("x-signature-ed25519")?;
    if !keys.verify(&ts, &sig, &body) {
        return Err((StatusCode::UNAUTHORIZED, "bad signature".to_string()));
    }

    let v: Value = serde_json::from_slice(&body).map_err(|_| e("bad json"))?;
    match v["op"].as_i64().unwrap_or(-1) {
        // 回调地址验证：验签通过后回签 seed+event_ts
        13 => {
            let plain_token = v["d"]["plain_token"].as_str().unwrap_or_default();
            let event_ts = v["d"]["event_ts"].as_str().unwrap_or_default();
            Ok(Json(json!({
                "plain_token": plain_token,
                "signature": keys.sign_validation(event_ts),
            })))
        }
        // dispatch：解析 → 全局分发器（与 WS 链路同一入口）
        0 => {
            let t = v["t"].as_str().unwrap_or("");
            if let Some(ev) = event::parse(appid, t, &v["d"])
                .or_else(|| event::parse_misc(appid, t, &v["d"]))
            {
                if let Some(d) = ctx.dispatcher.get() {
                    d.submit(ev).await;
                }
            }
            Ok(Json(json!({})))
        }
        _ => Ok(Json(json!({}))),
    }
}
