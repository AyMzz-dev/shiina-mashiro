use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

use super::manifest::Runtime;
use super::proto::{self, Inbound};

const CALL_TIMEOUT: Duration = Duration::from_secs(15);

/// 插件发起的框架 API 调用：host 读循环投递到这条通道，由 sdk_api 消费并 respond。
pub struct HostApiCall {
    pub host: Arc<PluginHost>,
    pub plugin: Option<String>,
    pub token: Option<String>,
    pub id: i64,
    pub method: String,
    pub params: Value,
}

/// 一个语言宿主进程：管多个同 runtime 插件，协议见 proto.rs。
/// 进程退出后所有 pending 请求失败，由 manager 决定是否拉起新宿主。
pub struct PluginHost {
    pub runtime: Runtime,
    stdin: Mutex<tokio::process::ChildStdin>,
    pending: DashMap<i64, oneshot::Sender<Result<Value, String>>>,
    api_tx: mpsc::UnboundedSender<HostApiCall>,
    next_id: AtomicI64,
    /// stdout 读循环还活着 = 宿主进程可用；死后 manager 会重建
    alive: AtomicBool,
    _child: Mutex<Child>,
}

impl PluginHost {
    /// hosts_dir 指到 `hosts/` 目录（内含 node/host.js、python/host.py）。
    /// program：解释器可执行名/路径（node、python，或设置页配置的绝对路径）。
    pub async fn spawn(
        runtime: Runtime,
        hosts_dir: PathBuf,
        api_tx: mpsc::UnboundedSender<HostApiCall>,
        program: &str,
    ) -> anyhow::Result<Arc<Self>> {
        let script = match runtime {
            Runtime::Node => hosts_dir.join("node").join("host.js"),
            Runtime::Python => hosts_dir.join("python").join("host.py"),
        };
        let mut cmd = Command::new(program);
        cmd.arg(&script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        // Windows：node/python 是控制台子进程，GUI 程序 spawn 会自带黑色 console 窗口
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");

        let host = Arc::new(Self {
            runtime,
            stdin: Mutex::new(stdin),
            pending: DashMap::new(),
            api_tx,
            next_id: AtomicI64::new(1),
            alive: AtomicBool::new(true),
            _child: Mutex::new(child),
        });
        let h = Arc::clone(&host);
        tokio::spawn(async move { h.read_loop(stdout).await });
        Ok(host)
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    async fn read_loop(self: Arc<Self>, stdout: ChildStdout) {
        let mut lines = BufReader::new(stdout).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => self.handle_line(&line).await,
                Ok(None) | Err(_) => break,
            }
        }
        self.alive.store(false, Ordering::Relaxed);
        tracing::warn!(runtime = ?self.runtime, "plugin host stdout closed");
        // 唤醒全部 pending，避免请求方傻等
        let ids: Vec<i64> = self.pending.iter().map(|e| *e.key()).collect();
        for id in ids {
            if let Some((_, tx)) = self.pending.remove(&id) {
                let _ = tx.send(Err("host exited".to_string()));
            }
        }
    }

    async fn handle_line(self: &Arc<Self>, line: &str) {
        match proto::decode(line) {
            Some(Inbound::Response { id, result, error }) => {
                if let Some((_, tx)) = self.pending.remove(&id) {
                    let _ = tx.send(match error {
                        Some(e) if !e.is_empty() => Err(e),
                        _ => Ok(result),
                    });
                }
            }
            Some(Inbound::ApiCall {
                id,
                method,
                params,
                plugin,
                token,
            }) => {
                let _ = self.api_tx.send(HostApiCall {
                    host: Arc::clone(self),
                    plugin,
                    token,
                    id,
                    method,
                    params,
                });
            }
            Some(Inbound::Notify { kind, data }) => match kind.as_str() {
                "log" => tracing::info!("plugin: {}", data),
                "register" => tracing::info!("plugin register: {}", data),
                _ => tracing::debug!("host notify {kind}: {data}"),
            },
            None => tracing::debug!("bad frame: {line}"),
        }
    }

    /// 宿主 → 插件 请求/响应。
    pub async fn call(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.insert(id, tx);
        let mut line = proto::request(id, method, params);
        line.push('\n');
        {
            let mut stdin = self.stdin.lock().await;
            if let Err(e) = async {
                stdin.write_all(line.as_bytes()).await?;
                stdin.flush().await
            }
            .await
            {
                self.pending.remove(&id); // 写失败立刻清 pending，别留到宿主死才释放
                return Err(e.into());
            }
        }
        match tokio::time::timeout(CALL_TIMEOUT, rx).await {
            Ok(Ok(r)) => r.map_err(anyhow::Error::msg),
            Ok(Err(_)) => anyhow::bail!("host dropped response"),
            Err(_) => anyhow::bail!("{method} timeout"),
        }
    }

    /// 宿主 → 插件 应答（回插件的 API 调用）。
    pub async fn respond(&self, id: i64, result: Result<Value, String>) -> anyhow::Result<()> {
        let mut line = proto::respond(id, result);
        line.push('\n');
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }
}
