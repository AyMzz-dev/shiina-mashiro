use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{interval, timeout};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

/// 默认订阅：GROUP_MEMBER(1<<24) + GROUP_AND_C2C(1<<25) + INTERACTION(1<<26)
/// + DIRECT_MESSAGE(1<<12) + PUBLIC_GUILD_MESSAGES(1<<30)。
/// 账号 ws_intents=0 时用它；频道消息等大intent 默认不开。
pub const DEFAULT_INTENTS: i64 = (1 << 30) | (1 << 12) | (1 << 24) | (1 << 25) | (1 << 26);

/// 网关主循环：断线 → 指数退避重连；有 session_id 时走 op6 Resume 续传。
pub struct Gateway {
    pub appid: i64,
    pub intents: i64,
    pub sandbox: bool,
    /// 自定义网关地址（webhook→websocket 中转/自建网关）；
    /// Some 时跳过 /gateway 取地址，且不要求对端先发 Hello
    pub ws_url: Option<String>,
    pub event_tx: mpsc::UnboundedSender<Value>,
    pub seq: Arc<AtomicI64>,
    pub session_id: Arc<Mutex<String>>,
    /// 最近一次收到 op11 Heartbeat ACK 的时刻（epoch ms）；
    /// 心跳任务据此检测「发得出但收不到应答」的假死连接。
    pub last_ack: Arc<AtomicI64>,
}

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
type WsRead = futures_util::stream::SplitStream<WsStream>;

/// 致命错误：继续重连没有意义（机器人被下架/封禁）。run_gateway 见到它应直接停止。
#[derive(Debug)]
pub struct GatewayFatal(pub String);
impl std::fmt::Display for GatewayFatal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for GatewayFatal {}

/// 服务端主动要求重连（OpCode 7）：官方网关约每小时例行下发一次，
/// 属正常协议行为而非故障，上层按 Info 级别记录即可。
#[derive(Debug)]
pub struct GatewayReconnect;
impl std::fmt::Display for GatewayReconnect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("server requested reconnect")
    }
}
impl std::error::Error for GatewayReconnect {}

/// 鉴权失败（关闭码 4004）：缓存的 access_token 已失效，
/// 上层应作废 token 缓存强制刷新后再重连。
#[derive(Debug)]
pub struct GatewayAuthFail(pub String);
impl std::fmt::Display for GatewayAuthFail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for GatewayAuthFail {}

/// 服务端关闭码 → 人类可读原因 + 处置建议（官方错误码表）。
fn close_hint(code: u16) -> &'static str {
    match code {
        4001 => "无效的 opcode",
        4002 => "无效的 payload",
        4006 => "无效的 session id，将重新鉴权",
        4007 => "seq 错误，将重新鉴权",
        4008 => "发送过快被限流，稍后重连",
        4009 => "连接过期，重连后恢复会话",
        4010 => "无效的 shard",
        4011 => "guild 过多需要分片",
        4012 => "无效的 version",
        4013 => "无效的 intent",
        4014 => "intent 无权限",
        4914 => "机器人已下架，仅允许连接沙箱环境",
        4915 => "机器人已封禁，请申请解封后再连接",
        4900..=4913 => "服务端内部错误",
        4004 => "token 无效，将刷新 token 后重连",
        9001 | 9005 => "token 校验失败，将重新鉴权",
        _ => "",
    }
}

impl Gateway {
    /// 取 wss 地址 → 连接 → 鉴权/恢复 → 心跳 → 读循环。
    /// 返回 Ok 表示收到了 stop 信号。
    pub async fn run(&self, http: &reqwest::Client, token: &str) -> Result<()> {
        let custom = self
            .ws_url
            .as_deref()
            .map(str::trim)
            .filter(|u| !u.is_empty());
        let url = match custom {
            Some(u) => u.to_string(),
            None => get_ws_url(http, token, self.sandbox).await?,
        };
        let (ws, _) = connect_async(&url).await.context("ws connect")?;
        let (write, mut read) = ws.split();
        let write = Arc::new(Mutex::new(write));
        // 新连接的 ACK 基线：防止连接刚建立、尚未发心跳就被误判假死
        self.last_ack
            .store(chrono::Utc::now().timestamp_millis(), Ordering::Relaxed);
        // 心跳任务检测到假死时置位，读循环在 15s 切片边界检查并退出
        let zombie = Arc::new(AtomicBool::new(false));

        let hb_ms = if custom.is_some() {
            // 中转网关：dumb relay 不下发 Hello，直接鉴权；协议仿真的 relay 会回 READY。
            // 心跳按官方默认周期发（对端不需要也无害）。
            tracing::info!(appid = self.appid, "custom ws gateway {url}");
            (41250.0 * 0.9) as u64
        } else {
            // Hello → 鉴权或恢复
            let hello = timeout(Duration::from_secs(10), read.next())
                .await
                .context("hello timeout")?
                .ok_or_else(|| anyhow!("ws closed"))??;
            let hello: Value = serde_json::from_str(hello.to_text()?)?;
            let interval_ms = hello["d"]["heartbeat_interval"].as_u64().unwrap_or(41250);
            (interval_ms as f64 * 0.9) as u64
        };

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

        // 心跳任务：d = 最近收到的事件 s；首次连接未收到任何消息时按文档传 null
        let (write2, seq2, ack2, z2) = (
            write.clone(),
            self.seq.clone(),
            self.last_ack.clone(),
            zombie.clone(),
        );
        // 官方网关才有 op11 ACK 协议；中转 relay 不发，不做假死检测
        let check_ack = custom.is_none();
        let hb = tokio::spawn(async move {
            let mut tick = interval(Duration::from_millis(hb_ms.max(5000)));
            // tokio interval 首 tick 立即触发——吞掉，按文档「鉴权成功后按周期」发送
            tick.tick().await;
            let mut sent = 0u32;
            loop {
                tick.tick().await;
                // 假死检测：发过心跳但连续 ~2 个周期无 op11 → TCP 半开，
                // 标记 zombie 让读循环尽快断开走 Resume
                if check_ack && sent > 0 {
                    let silence =
                        chrono::Utc::now().timestamp_millis() - ack2.load(Ordering::Relaxed);
                    if silence > (hb_ms as i64) * 2 {
                        z2.store(true, Ordering::Relaxed);
                        break;
                    }
                }
                let s = seq2.load(Ordering::Relaxed);
                let d = if s == 0 { Value::Null } else { json!(s) };
                let msg = Message::Text(json!({"op":1,"d":d}).to_string().into());
                if write2.lock().await.send(msg).await.is_err() {
                    break;
                }
                sent += 1;
            }
        });

        // 读循环
        let r = self.read_loop(&mut read, &zombie).await;
        hb.abort();
        let _ = write;
        r
    }

    async fn read_loop(&self, read: &mut WsRead, zombie: &AtomicBool) -> Result<()> {
        // 无帧看门狗：官方链路 120s 没有任何帧视为僵尸连接；
        // 自定义中转可能长时间无事件，放宽到 600s
        let limit = Duration::from_secs(if self.ws_url.is_some() { 600 } else { 120 });
        let mut last_frame = std::time::Instant::now();
        loop {
            // 15s 切片轮询：正常帧照常处理；超时切片检查 zombie 标记和总看门狗，
            // 比单次 120s 大超时更快感知心跳假死
            let msg = timeout(Duration::from_secs(15), read.next()).await;
            let msg = match msg {
                Ok(Some(Ok(m))) => m,
                Ok(Some(Err(e))) => return Err(e.into()),
                Ok(None) => return Err(anyhow!("ws closed")),
                Err(_) => {
                    if zombie.load(Ordering::Relaxed) {
                        return Err(anyhow!("心跳连续无应答（连接假死），重连恢复"));
                    }
                    if last_frame.elapsed() > limit {
                        return Err(anyhow!("ws read watchdog timeout"));
                    }
                    continue;
                }
            };
            last_frame = std::time::Instant::now();
            let text = match msg {
                Message::Text(t) => t,
                Message::Close(f) => {
                    let (code, reason) = f
                        .map(|c| (u16::from(c.code), c.reason.to_string()))
                        .unwrap_or((0, String::new()));
                    let hint = close_hint(code);
                    let desc = format!(
                        "服务端关闭 {code}{} {reason}",
                        if hint.is_empty() {
                            String::new()
                        } else {
                            format!("({hint})")
                        }
                    );
                    return Err(match code {
                        // 已下架/已封禁：文档明确不可重连，交给上层熔断
                        4914 | 4915 => GatewayFatal(desc).into(),
                        // token 失效：作废缓存，上层刷新 token 后重连
                        4004 => GatewayAuthFail(desc).into(),
                        // session/seq/token 无效：清会话归零，下轮走 op2 重新鉴权
                        4006 | 4007 | 9001 | 9005 => {
                            *self.session_id.lock().await = String::new();
                            self.seq.store(0, Ordering::Relaxed);
                            anyhow!(desc)
                        }
                        _ => anyhow!(desc),
                    });
                }
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
                        *self.session_id.lock().await = v["d"]["session_id"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string();
                    }
                    let _ = self.event_tx.send(v);
                }
                7 => return Err(GatewayReconnect.into()),
                9 => {
                    // invalid session：清会话归零，外层重连走重新鉴权
                    *self.session_id.lock().await = String::new();
                    self.seq.store(0, Ordering::Relaxed);
                    return Err(anyhow!("invalid session"));
                }
                11 => {
                    self.last_ack
                        .store(chrono::Utc::now().timestamp_millis(), Ordering::Relaxed);
                }
                _ => {}
            }
        }
    }
}

/// GET /gateway/bot → wss url（沙盒走腾讯沙盒环境域名）。
/// 用带分片的接入点：/gateway 不返回 session_start_limit，拿不到限额信息。
async fn get_ws_url(http: &reqwest::Client, token: &str, sandbox: bool) -> Result<String> {
    let v: Value = http
        .get(format!(
            "{}/gateway/bot",
            crate::bot::api::api_base(sandbox)
        ))
        .header("Authorization", format!("QQBot {token}"))
        .send()
        .await?
        .json()
        .await?;
    // session_start_limit.remaining=0 时硬连只会被服务端立刻踢掉，直接报清楚
    if v["session_start_limit"]["remaining"].as_i64() == Some(0) {
        let reset_after = v["session_start_limit"]["reset_after"]
            .as_i64()
            .unwrap_or(0);
        anyhow::bail!("会话并发限额已耗尽，约 {}h 后重置", reset_after / 3_600_000);
    }
    // 官方建议分片数 >1 说明该 bot 的 guild 规模已需要分片——[0,1] 仍能收全
    // 事件（分片只是负载分流），但值得在日志里留个提醒
    if let Some(shards) = v["shards"].as_i64() {
        if shards > 1 {
            tracing::warn!(
                "官方建议分片数 {shards}：当前单连接 [0,1] 可收全事件，事件量大时可考虑分片"
            );
        }
    }
    v["url"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow!("no gateway url: {v}"))
}
