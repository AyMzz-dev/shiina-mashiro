//! Webhook 接入（P4）：设置 `webhook_listen`（如 `0.0.0.0:8443`）后启动 axum 服务。
//!
//! 端点：
//!   POST /qq/callback/{appid}  —— QQ 平台 webhook 回调（官方要求 HTTPS，端口 80/443/8080/8443）
//!   GET  /qq/ws[/{appid}]      —— webhook→websocket 事件转发：把收到的 op0 事件原样推给
//!                                连上的客户端，协议层仿真官方网关（Hello/Identify/Heartbeat），
//!                                客户端填账号详情里的「自定义网关」即可直连。
//!   GET  /qq/health            —— 存活探针
//!
//! 设置键：
//!   webhook_listen      监听地址（空 = 纯 WS 模式，不启动）
//!   webhook_tls_cert    PEM 证书路径；与 webhook_tls_key 成对配置后监听 HTTPS
//!   webhook_tls_key     PEM 私钥路径；不配证书时 HTTP 监听（适合 nginx/caddy 反代终结 TLS）
//!   /qq/ws 对外暴露的地址取决于上层部署（反代域名/公网 IP），鉴权由中转层自行处理
//!
//! 签名模型（按官方文档/Go SDK）：
//!   seed      = secret 重复至 ≥32 字节取前 32
//!   keypair   = Ed25519 from seed（双方同密钥对）
//!   验签      = verify(public, X-Signature-Ed25519, timestamp + body)
//!   op13 回签 = sign(private, event_ts + plain_token)

use std::sync::{Arc, OnceLock};

use axum::body::Bytes;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json};
use axum::routing::{get, post};
use axum::Router;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::router::dispatch::Dispatcher;
use crate::router::event;
use crate::storage::db::Database;

struct WebhookCtx {
    db: Arc<Database>,
    dispatcher: Arc<OnceLock<Dispatcher>>,
    /// (appid, payload text) 广播总线：/qq/ws 客户端按 appid 过滤订阅
    hub: broadcast::Sender<(i64, String)>,
    /// /qq/ws 接入鉴权（设置 ws_token；空 = 不校验，仅建议内网/反代鉴权部署）
    ws_token: String,
}

/// 从 secret 派生 Ed25519 密钥对。
struct Keys {
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
        let seed: [u8; ed25519_dalek::SECRET_KEY_LENGTH] = s[..ed25519_dalek::SECRET_KEY_LENGTH]
            .try_into()
            .expect("32 bytes");
        Self {
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
        self.signing.verifying_key().verify(&msg, &sig).is_ok()
    }

    /// op13 回调地址验证：sign(event_ts + plain_token)，hex 输出。
    fn sign_validation(&self, event_ts: &str, plain_token: &str) -> String {
        let mut msg = event_ts.as_bytes().to_vec();
        msg.extend_from_slice(plain_token.as_bytes());
        hex::encode(self.signing.sign(&msg).to_bytes())
    }
}

/// 设置里有 webhook_listen 才启动；空 = 纯 WS 模式（默认）。
/// TLS：webhook_tls_cert + webhook_tls_key 都给了 → HTTPS；否则 HTTP（反代场景）。
pub fn start(db: Arc<Database>, dispatcher: Arc<OnceLock<Dispatcher>>) {
    let addr = db.get_setting("webhook_listen").unwrap_or_default();
    let addr = addr.trim().to_string();
    if addr.is_empty() {
        return;
    }
    let cert = db.get_setting("webhook_tls_cert").unwrap_or_default();
    let key = db.get_setting("webhook_tls_key").unwrap_or_default();
    let ws_token = db.get_setting("ws_token").unwrap_or_default();
    if ws_token.trim().is_empty() {
        tracing::warn!("/qq/ws 未配置 ws_token：事件流端点无鉴权，仅建议内网或经鉴权反代开放");
    }
    tokio::spawn(async move {
        let (hub, _) = broadcast::channel(512);
        let ctx = Arc::new(WebhookCtx {
            db,
            dispatcher,
            hub,
            ws_token,
        });
        let app = Router::new()
            .route("/qq/callback/{appid}", post(callback))
            .route("/qq/ws", get(ws_all))
            .route("/qq/ws/{appid}", get(ws_one))
            .route("/qq/health", get(|| async { Json(json!({ "ok": true })) }))
            .with_state(ctx);

        if !cert.trim().is_empty() && !key.trim().is_empty() {
            serve_tls(&addr, &cert, &key, app).await;
        } else {
            if !cert.trim().is_empty() || !key.trim().is_empty() {
                tracing::warn!("webhook_tls_cert / webhook_tls_key 需成对配置，已按 HTTP 启动");
            }
            match tokio::net::TcpListener::bind(&addr).await {
                Ok(l) => {
                    tracing::info!(
                        "webhook listening http on {addr} → /qq/callback/{{appid}} + /qq/ws"
                    );
                    if let Err(e) = axum::serve(l, app).await {
                        tracing::error!("webhook serve: {e}");
                    }
                }
                Err(e) => tracing::error!("webhook bind {addr}: {e}"),
            }
        }
    });
}

async fn serve_tls(addr: &str, cert: &str, key: &str, app: Router) {
    let sock: std::net::SocketAddr = match addr.parse() {
        Ok(a) => a,
        Err(e) => {
            tracing::error!("webhook addr {addr}: {e}");
            return;
        }
    };
    let tls = match axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("webhook tls load ({cert}, {key}): {e}");
            return;
        }
    };
    tracing::info!("webhook listening https on {addr} → /qq/callback/{{appid}} + /qq/ws");
    if let Err(e) = axum_server::bind_rustls(sock, tls)
        .serve(app.into_make_service())
        .await
    {
        tracing::error!("webhook tls serve: {e}");
    }
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
        // 回调地址验证：验签通过后回签 event_ts + plain_token
        13 => {
            let plain_token = v["d"]["plain_token"].as_str().unwrap_or_default();
            let event_ts = v["d"]["event_ts"].as_str().unwrap_or_default();
            Ok(Json(json!({
                "plain_token": plain_token,
                "signature": keys.sign_validation(event_ts, plain_token),
            })))
        }
        // dispatch：解析 → 全局分发器（与 WS 链路同一入口）；同时广播给 /qq/ws 订阅端
        0 => {
            let t = v["t"].as_str().unwrap_or("");
            if let Some(ev) =
                event::parse(appid, t, &v["d"]).or_else(|| event::parse_misc(appid, t, &v["d"]))
            {
                if let Some(d) = ctx.dispatcher.get() {
                    d.submit(ev).await;
                }
            }
            let _ = ctx.hub.send((appid, v.to_string()));
            Ok(Json(json!({})))
        }
        _ => Ok(Json(json!({}))),
    }
}

// ---------- webhook → websocket 事件转发 ----------

/// /qq/ws 全量事件；/qq/ws/{appid} 单账号事件。
/// 面向远程中转服务；设置 ws_token 时要求 ?token= 或 Authorization: Bearer 匹配。
async fn ws_all(
    ws: WebSocketUpgrade,
    State(ctx): State<Arc<WebhookCtx>>,
    headers: HeaderMap,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    match check_ws_auth(&ctx, &headers, &q) {
        Ok(()) => ws.on_upgrade(move |s| ws_relay(s, ctx, None)),
        Err(r) => r,
    }
}

async fn ws_one(
    ws: WebSocketUpgrade,
    State(ctx): State<Arc<WebhookCtx>>,
    Path(appid): Path<i64>,
    headers: HeaderMap,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    match check_ws_auth(&ctx, &headers, &q) {
        Ok(()) => ws.on_upgrade(move |s| ws_relay(s, ctx, Some(appid))),
        Err(r) => r,
    }
}

/// ws_token 非空时校验：query token 参数或 Authorization: Bearer 二选一。
fn check_ws_auth(
    ctx: &WebhookCtx,
    headers: &HeaderMap,
    q: &std::collections::HashMap<String, String>,
) -> Result<(), axum::response::Response> {
    let expected = ctx.ws_token.trim();
    if expected.is_empty() {
        return Ok(());
    }
    let given = q
        .get("token")
        .map(String::as_str)
        .or_else(|| {
            headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
        })
        .unwrap_or("");
    if given == expected {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "invalid or missing ws token" })),
        )
            .into_response())
    }
}

/// 仿真官方网关节点：连上先发 op10 Hello；客户端 op1 心跳回 op11、op2 鉴权回 READY、
/// op6 续传回 RESUMED；后台广播 op0 事件帧原样下发。dumb relay 客户端收到即可解析。
async fn ws_relay(socket: WebSocket, ctx: Arc<WebhookCtx>, appid: Option<i64>) {
    let (mut tx, mut rx) = socket.split();
    let hello = json!({ "op": 10, "d": { "heartbeat_interval": 41250 } }).to_string();
    if tx.send(WsMessage::Text(hello.into())).await.is_err() {
        return;
    }
    let mut sub = ctx.hub.subscribe();
    loop {
        tokio::select! {
            // 平台事件 → 客户端（按 appid 过滤）
            frame = sub.recv() => {
                match frame {
                    Ok((a, text)) => {
                        if appid.is_some() && appid != Some(a) {
                            continue;
                        }
                        if tx.send(WsMessage::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("ws_relay lagged {n} frames");
                        continue;
                    }
                    Err(_) => break,
                }
            }
            // 客户端帧 → 仿真官方应答（只读不转发）
            msg = rx.next() => {
                let m = match msg {
                    Some(Ok(m)) => m,
                    Some(Err(_)) | None => break,
                };
                let WsMessage::Text(t) = m else {
                    if matches!(m, WsMessage::Close(_)) { break; }
                    continue;
                };
                let Ok(v) = serde_json::from_str::<Value>(&t) else { continue };
                let reply = match v["op"].as_i64().unwrap_or(-1) {
                    // 心跳 → ACK
                    1 => Some(json!({ "op": 11 }).to_string()),
                    // Identify → 伪造 READY（session 由中继持有）
                    2 => Some(json!({
                        "op": 0, "s": 0, "t": "READY",
                        "d": { "session_id": format!("relay-{}", rand::random::<u32>()),
                               "user": { "id": "", "username": "", "bot": true } },
                    }).to_string()),
                    // Resume → RESUMED
                    6 => Some(json!({ "op": 0, "s": 0, "t": "RESUMED", "d": "" }).to_string()),
                    _ => None,
                };
                if let Some(r) = reply {
                    if tx.send(WsMessage::Text(r.into())).await.is_err() {
                        break;
                    }
                }
            }
        }
    }
}
