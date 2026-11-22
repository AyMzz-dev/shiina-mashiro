use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex, Notify};
use tokio::time::{interval, timeout};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

/// 默认订阅：GROUP_AND_C2C(1<<25) + DIRECT_MESSAGE(1<<12) + PUBLIC_GUILD_MESSAGES(1<<30)。
/// 与旧框架 kDefaultWsIntents 同值；账号 ws_intents=0 时用它。
pub const DEFAULT_INTENTS: i64 = (1 << 30) | (1 << 12) | (1 << 25);

/// 网关主循环：断线 → 指数退避重连；有 session_id 时走 op6 Resume 续传。
pub struct Gateway {
    pub appid: i64,
    pub intents: i64,
    pub sandbox: bool,
    pub stop: Arc<Notify>,
    pub event_tx: mpsc::UnboundedSender<Value>,
    pub seq: Arc<AtomicI64>,
    pub session_id: Arc<Mutex<String>>,
}

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
type WsRead = futures_util::stream::SplitStream<WsStream>;

impl Gateway {
    /// 取 wss 地址 → 连接 → 鉴权/恢复 → 心跳 → 读循环。
    /// 返回 Ok 表示收到了 stop 信号。
    pub async fn run(&self, http: &reqwest::Client, token: &str) -> Result<()> {
        let url = get_ws_url(http, token, self.sandbox).await?;
        let (ws, _) = connect_async(&url).await.context("ws connect")?;
        let (write, mut read) = ws.split();
        let write = Arc::new(Mutex::new(write));

        // Hello → 鉴权或恢复
        let hello = timeout(Duration::from_secs(10), read.next())
            .await
            .context("hello timeout")?
            .ok_or_else(|| anyhow!("ws closed"))??;
        let hello: Value = serde_json::from_str(hello.to_text()?)?;
        let interval_ms = hello["d"]["heartbeat_interval"].as_u64().unwrap_or(41250);
        let hb_ms = (interval_ms as f64 * 0.9) as u64;

        let sid = self.session_id.lock().await.clone();
        let payload = if sid.is_empty() {
            json!({"op": 2, "d": {
                "token": format!("QQBot {token}"),
                "intents": self.intents,
                "shard": [0, 1],
                "properties": {},
            }})
        } else {
            json!({"op": 6, "d": {
                "token": format!("QQBot {token}"),
                "session_id": sid,
                "seq": self.seq.load(Ordering::Relaxed),
            }})
        };
        write
            .lock()
            .await
            .send(Message::Text(payload.to_string().into()))
            .await?;

        // 心跳任务
        let (hb_stop, write2, seq2) =
            (self.stop.clone(), write.clone(), self.seq.clone());
        let hb = tokio::spawn(async move {
            let mut tick = interval(Duration::from_millis(hb_ms.max(5000)));
            loop {
                tokio::select! {
                    _ = hb_stop.notified() => break,
                    _ = tick.tick() => {
                        let s = seq2.load(Ordering::Relaxed);
                        let msg = Message::Text(json!({"op":1,"d":s}).to_string().into());
                        if write2.lock().await.send(msg).await.is_err() { break; }
                    }
                }
            }
        });

        // 读循环
        let r = self.read_loop(&mut read).await;
        hb.abort();
        let _ = write;
        r
    }

    async fn read_loop(&self, read: &mut WsRead) -> Result<()> {
        loop {
            tokio::select! {
                _ = self.stop.notified() => return Ok(()),
                msg = read.next() => {
                    let msg = match msg {
                        Some(Ok(m)) => m,
                        Some(Err(e)) => return Err(e.into()),
                        None => return Err(anyhow!("ws closed")),
                    };
                    let text = match msg {
                        Message::Text(t) => t,
                        Message::Close(_) => return Err(anyhow!("ws close frame")),
                        _ => continue,
                    };
                    let v: Value = serde_json::from_str(&text)?;
                    let op = v["op"].as_i64().unwrap_or(-1);
                    match op {
                        0 => {
                            // dispatch
                            if let Some(s) = v["s"].as_i64() {
                                self.seq.store(s, Ordering::Relaxed);
                            }
                            if v["t"].as_str() == Some("READY") {
                                *self.session_id.lock().await =
                                    v["d"]["session_id"].as_str().unwrap_or_default().to_string();
                            }
                            let _ = self.event_tx.send(v);
                        }
                        7 => return Err(anyhow!("server requested reconnect")),
                        9 => {
                            // invalid session：清 session，外层重连走重新鉴权
                            *self.session_id.lock().await = String::new();
                            return Err(anyhow!("invalid session"));
                        }
                        11 => {} // heartbeat ack
                        _ => {}
                    }
                }
            }
        }
    }
}

/// GET /gateway → wss url（沙盒走腾讯沙盒环境域名）
async fn get_ws_url(http: &reqwest::Client, token: &str, sandbox: bool) -> Result<String> {
    let v: Value = http
        .get(format!("{}/gateway", crate::bot::api::api_base(sandbox)))
        .header("Authorization", format!("QQBot {token}"))
        .send()
        .await?
        .json()
        .await?;
    v["url"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow!("no gateway url: {v}"))
}
