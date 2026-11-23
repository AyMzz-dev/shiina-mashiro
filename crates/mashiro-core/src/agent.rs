//! Agent 插件生成：LLM（OpenAI / Anthropic 双协议）按 PLUGIN_SPEC 产出插件。
//! 消息用统一的 Msg 内部表示，发送时翻译成各家格式——工具调用走
//! tool_calls / tool_use 原生协议；不支持工具的接口可用 <<<FILE:>>> 块兜底。

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

const SPEC: &str = include_str!("../../../PLUGIN_SPEC.md");
const SDK_JS: &str = include_str!("../../../sdk/js/mashiro.js");
const SDK_PY: &str = include_str!("../../../sdk/python/mashiro.py");

/// 一次工具调用（模型侧产生）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// arguments：OpenAI 是 JSON 字符串、Anthropic 是对象——统一存 Value。
    pub args: serde_json::Value,
}

/// 会话消息统一表示；to_openai_msgs / to_anthropic_msgs 负责翻译。
/// serde 用于会话持久化：adjacent tag（role/content），兼容 tuple variant。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", content = "content", rename_all = "lowercase")]
pub enum Msg {
    User(String),
    Assistant {
        text: String,
        calls: Vec<ToolCall>,
    },
    /// 工具执行结果（喂回模型）
    Tool {
        id: String,
        name: String,
        content: String,
    },
}

/// 一轮回复：纯文本 or 带工具调用（text 可能为空串）。
pub enum ChatResp {
    Text(String),
    Calls { text: String, calls: Vec<ToolCall> },
}

#[derive(Debug, Clone)]
pub struct ProviderCfg {
    /// "openai" | "anthropic"
    pub provider: String,
    pub base: String,
    pub key: String,
    pub model: String,
}

impl ProviderCfg {
    /// 从 settings 表读 agent_* 配置；缺 key/model 直接报错（前端引导去设置页）。
    pub fn from_db(db: &crate::storage::db::Database) -> Result<Self> {
        let g = |k: &str| db.get_setting(k).unwrap_or_default();
        let provider = {
            let p = g("agent_provider");
            if p.is_empty() {
                "openai".to_string()
            } else {
                p
            }
        };
        let base = default_base(&provider, g("agent_base"));
        let cfg = Self {
            provider,
            base,
            key: g("agent_key"),
            model: g("agent_model"),
        };
        if cfg.key.is_empty() {
            bail!("未配置 API Key，先到「设置 → Agent 模型」填写");
        }
        if cfg.model.is_empty() {
            bail!("未配置模型名，先到「设置 → Agent 模型」填写");
        }
        Ok(cfg)
    }
}

pub fn default_base(provider: &str, base: String) -> String {
    if !base.is_empty() {
        return base;
    }
    if provider == "anthropic" {
        "https://api.anthropic.com".into()
    } else {
        "https://api.openai.com/v1".into()
    }
}

// ---------- URL 归一 ----------

fn openai_url(base: &str) -> String {
    let b = base.trim_end_matches('/');
    if b.ends_with("/chat/completions") {
        b.to_string()
    } else {
        format!("{b}/chat/completions")
    }
}

fn openai_models_url(base: &str) -> String {
    let b = base.trim_end_matches('/');
    let b = b.strip_suffix("/chat/completions").unwrap_or(b);
    format!("{b}/models")
}

fn anthropic_url(base: &str) -> String {
    let b = base.trim_end_matches('/');
    if b.ends_with("/messages") {
        b.to_string()
    } else if b.ends_with("/v1") {
        format!("{b}/messages")
    } else {
        format!("{b}/v1/messages")
    }
}

fn anthropic_models_url(base: &str) -> String {
    let b = base.trim_end_matches('/');
    if b.ends_with("/models") {
        b.to_string()
    } else if b.ends_with("/v1") {
        format!("{b}/models")
    } else {
        format!("{b}/v1/models")
    }
}

// ---------- 消息翻译 ----------

fn to_openai_msgs(system: &str, msgs: &[Msg]) -> Vec<serde_json::Value> {
    let mut out = vec![json!({ "role": "system", "content": system })];
    for m in msgs {
        match m {
            Msg::User(s) => out.push(json!({ "role": "user", "content": s })),
            Msg::Assistant { text, calls } => {
                if calls.is_empty() {
                    out.push(json!({ "role": "assistant", "content": text }));
                } else {
                    let tcs: Vec<_> = calls
                        .iter()
                        .map(|c| {
                            json!({
                                "id": c.id,
                                "type": "function",
                                "function": { "name": c.name, "arguments": c.args.to_string() },
                            })
                        })
                        .collect();
                    out.push(json!({
                        "role": "assistant",
                        "content": if text.is_empty() { serde_json::Value::Null } else { json!(text) },
                        "tool_calls": tcs,
                    }));
                }
            }
            Msg::Tool { id, name, content } => out.push(json!({
                "role": "tool",
                "tool_call_id": id,
                "name": name,
                "content": content,
            })),
        }
    }
    out
}

fn to_anthropic_msgs(msgs: &[Msg]) -> Vec<serde_json::Value> {
    let mut out: Vec<serde_json::Value> = Vec::new();
    for m in msgs {
        match m {
            Msg::User(s) => out.push(json!({ "role": "user", "content": s })),
            Msg::Assistant { text, calls } => {
                if calls.is_empty() {
                    out.push(json!({ "role": "assistant", "content": text }));
                } else {
                    let mut blocks: Vec<serde_json::Value> = Vec::new();
                    if !text.is_empty() {
                        blocks.push(json!({ "type": "text", "text": text }));
                    }
                    for c in calls {
                        blocks.push(json!({
                            "type": "tool_use",
                            "id": c.id,
                            "name": c.name,
                            "input": c.args,
                        }));
                    }
                    out.push(json!({ "role": "assistant", "content": blocks }));
                }
            }
            // Anthropic 要求 tool_result 在 user 消息里；连续的 Tool 合并成一条
            Msg::Tool { id, content, .. } => {
                let block = json!({ "type": "tool_result", "tool_use_id": id, "content": content });
                let merged = out.last_mut().and_then(|last| {
                    if last["role"].as_str() == Some("user") && last["content"].is_array() {
                        let arr = last["content"].as_array()?;
                        if arr
                            .iter()
                            .all(|b| b["type"].as_str() == Some("tool_result"))
                        {
                            last["content"].as_array_mut()?.push(block.clone());
                            return Some(());
                        }
                    }
                    None
                });
                if merged.is_none() {
                    out.push(json!({ "role": "user", "content": [block] }));
                }
            }
        }
    }
    out
}

// ---------- 工具 schema ----------

fn tool_props(props: serde_json::Value, required: &[&str]) -> serde_json::Value {
    json!({
        "type": "object",
        "properties": props,
        "required": required,
    })
}

fn tool_descs() -> Vec<(&'static str, &'static str, serde_json::Value)> {
    vec![
        ("write_file", "写入或覆盖插件文件（相对插件目录）", json!({"path":{"type":"string"},"content":{"type":"string"}})),
        ("read_file", "读取插件文件内容", json!({"path":{"type":"string"}})),
        ("delete_file", "删除插件文件", json!({"path":{"type":"string"}})),
        ("list_files", "列出当前插件文件集", json!({})),
        ("install_plugin", "把当前文件集安装并真实加载校验（manifest 解析 + 宿主 load + describe）。成功返回 handlers 列表；失败返回 error——修复后重试", json!({})),
        ("mcp_call", "调用框架内置工具（清单见系统提示，args 为对象可省略）", json!({"name":{"type":"string"},"args":{"type":"object"}})),
    ]
}

fn tools_openai() -> serde_json::Value {
    let mut v: Vec<serde_json::Value> = tool_descs()
        .iter()
        .map(|(n, d, p)| {
            let required: Vec<&str> = p
                .as_object()
                .map(|o| o.keys().map(String::as_str).collect())
                .unwrap_or_default();
            json!({
                "type": "function",
                "function": {
                    "name": n,
                    "description": d,
                    "parameters": tool_props(p.clone(), &required),
                },
            })
        })
        .collect();
    v.push(json!({
        "type": "function",
        "function": {
            "name": "ask_user",
            "description": "需求不明确或有多个合理实现方向时向用户提问确认（每题给 2~4 个选项让用户点选）",
            "parameters": ask_user_schema(),
        },
    }));
    serde_json::Value::Array(v)
}

fn tools_anthropic() -> serde_json::Value {
    let mut v: Vec<serde_json::Value> = tool_descs()
        .iter()
        .map(|(n, d, p)| {
            let required: Vec<&str> = p
                .as_object()
                .map(|o| o.keys().map(String::as_str).collect())
                .unwrap_or_default();
            json!({
                "name": n,
                "description": d,
                "input_schema": tool_props(p.clone(), &required),
            })
        })
        .collect();
    v.push(json!({
        "name": "ask_user",
        "description": "需求不明确或有多个合理实现方向时向用户提问确认（每题给 2~4 个选项让用户点选）",
        "input_schema": ask_user_schema(),
    }));
    serde_json::Value::Array(v)
}

/// ask_user 工具参数：结构化问题列表，UI 弹窗收集答案回填。
fn ask_user_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "questions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "q": {"type": "string", "description": "问题正文"},
                        "options": {"type": "array", "items": {"type": "string"}, "description": "可选答案"}
                    },
                    "required": ["q", "options"]
                }
            }
        },
        "required": ["questions"]
    })
}

// ---------- 请求 ----------

/// 带工具的一轮对话。
pub async fn chat_tools(
    http: &reqwest::Client,
    cfg: &ProviderCfg,
    system: &str,
    msgs: &[Msg],
) -> Result<ChatResp> {
    if cfg.provider == "anthropic" {
        chat_anthropic(http, cfg, system, msgs).await
    } else {
        chat_openai(http, cfg, system, msgs).await
    }
}

/// 不带工具的简单对话（测试连接用）。
pub async fn chat(
    http: &reqwest::Client,
    cfg: &ProviderCfg,
    system: &str,
    text: &str,
) -> Result<String> {
    match chat_tools(http, cfg, system, &[Msg::User(text.to_string())]).await? {
        ChatResp::Text(t) => Ok(t),
        ChatResp::Calls { .. } => Ok("(模型请求了工具调用)".into()),
    }
}

// ---------- 流式请求（SSE）----------

/// SSE 分帧读取：body 按 \n\n 切块，逐个 data: 负载交给 f（[DONE] 自动跳过）。
/// 每个 chunk 都带 180s 空闲闸：连接假活但服务端不吐字时会无限转圈，超时判死。
async fn read_sse(
    mut resp: reqwest::Response,
    f: &mut impl FnMut(&serde_json::Value),
) -> Result<()> {
    let mut buf = String::new();
    loop {
        let chunk = tokio::time::timeout(std::time::Duration::from_secs(180), resp.chunk())
            .await
            .context("SSE 流空闲超时（180s 无数据）")?
            .context("读取 SSE 流失败")?;
        let Some(chunk) = chunk else { break };
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = buf.find("\n\n") {
            let ev = buf[..pos].to_string();
            buf.drain(..pos + 2);
            for line in ev.lines() {
                if let Some(d) = line.strip_prefix("data:") {
                    let d = d.trim();
                    if d != "[DONE]" {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(d) {
                            f(&v);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// 带工具的流式对话：text 增量经 on_delta 实时回调给 UI。
pub async fn chat_tools_stream(
    http: &reqwest::Client,
    cfg: &ProviderCfg,
    system: &str,
    msgs: &[Msg],
    on_delta: impl FnMut(&str),
) -> Result<ChatResp> {
    if cfg.provider == "anthropic" {
        chat_anthropic_stream(http, cfg, system, msgs, on_delta).await
    } else {
        chat_openai_stream(http, cfg, system, msgs, on_delta).await
    }
}

async fn chat_openai_stream(
    http: &reqwest::Client,
    cfg: &ProviderCfg,
    system: &str,
    msgs: &[Msg],
    mut on_delta: impl FnMut(&str),
) -> Result<ChatResp> {
    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        http.post(openai_url(&cfg.base))
            .bearer_auth(&cfg.key)
            .json(&json!({
                "model": cfg.model,
                "messages": to_openai_msgs(system, msgs),
                "tools": tools_openai(),
                "stream": true,
            }))
            .send(),
    )
    .await
    .context("openai 请求超时（60s 未返回响应头）")?
    .context("openai 请求失败")?;
    let status = resp.status();
    if !status.is_success() {
        let v: serde_json::Value = resp.json().await.unwrap_or_default();
        bail!(
            "openai {status}: {}",
            v["error"]["message"].as_str().unwrap_or("unknown")
        );
    }
    let mut text = String::new();
    // 流式 tool_calls：按 index 拼接 (id, name, arguments 分片)
    let mut calls: Vec<(String, String, String)> = Vec::new();
    read_sse(resp, &mut |v| {
        let d = &v["choices"][0]["delta"];
        if let Some(t) = d["content"].as_str() {
            text.push_str(t);
            on_delta(t);
        }
        if let Some(tcs) = d["tool_calls"].as_array() {
            for tc in tcs {
                let i = tc["index"].as_u64().unwrap_or(0) as usize;
                while calls.len() <= i {
                    calls.push((String::new(), String::new(), String::new()));
                }
                if let Some(id) = tc["id"].as_str() {
                    calls[i].0 = id.to_string();
                }
                if let Some(n) = tc["function"]["name"].as_str() {
                    calls[i].1.push_str(n);
                }
                if let Some(a) = tc["function"]["arguments"].as_str() {
                    calls[i].2.push_str(a);
                }
            }
        }
    })
    .await?;
    let calls: Vec<ToolCall> = calls
        .into_iter()
        .map(|(id, name, args)| ToolCall {
            id,
            name,
            args: serde_json::from_str(&args).unwrap_or(json!({})),
        })
        .collect();
    if calls.is_empty() {
        Ok(ChatResp::Text(text))
    } else {
        Ok(ChatResp::Calls { text, calls })
    }
}

async fn chat_anthropic_stream(
    http: &reqwest::Client,
    cfg: &ProviderCfg,
    system: &str,
    msgs: &[Msg],
    mut on_delta: impl FnMut(&str),
) -> Result<ChatResp> {
    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        http.post(anthropic_url(&cfg.base))
            .header("x-api-key", &cfg.key)
            .header("anthropic-version", "2023-06-01")
            .json(&json!({
                "model": cfg.model,
                "max_tokens": 8192,
                "system": system,
                "messages": to_anthropic_msgs(msgs),
                "tools": tools_anthropic(),
                "stream": true,
            }))
            .send(),
    )
    .await
    .context("anthropic 请求超时（60s 未返回响应头）")?
    .context("anthropic 请求失败")?;
    let status = resp.status();
    if !status.is_success() {
        let v: serde_json::Value = resp.json().await.unwrap_or_default();
        bail!(
            "anthropic {status}: {}",
            v["error"]["message"].as_str().unwrap_or("unknown")
        );
    }
    let mut text = String::new();
    // content_block 按 index 对齐：tool_use 块的 input 以 JSON 分片增量到达
    let mut calls: Vec<Option<ToolCall>> = Vec::new();
    read_sse(resp, &mut |v| {
        let i = v["index"].as_u64().unwrap_or(0) as usize;
        match v["type"].as_str().unwrap_or("") {
            "content_block_start" => {
                while calls.len() <= i {
                    calls.push(None);
                }
                if v["content_block"]["type"].as_str() == Some("tool_use") {
                    calls[i] = Some(ToolCall {
                        id: v["content_block"]["id"].as_str().unwrap_or("").to_string(),
                        name: v["content_block"]["name"]
                            .as_str()
                            .unwrap_or("")
                            .to_string(),
                        args: json!(""),
                    });
                }
            }
            "content_block_delta" => match v["delta"]["type"].as_str() {
                Some("text_delta") => {
                    let t = v["delta"]["text"].as_str().unwrap_or("");
                    text.push_str(t);
                    on_delta(t);
                }
                Some("input_json_delta") => {
                    if let Some(Some(c)) = calls.get_mut(i) {
                        let cur = c.args.as_str().unwrap_or("").to_string()
                            + v["delta"]["partial_json"].as_str().unwrap_or("");
                        c.args = json!(cur);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    })
    .await?;
    let calls: Vec<ToolCall> = calls
        .into_iter()
        .flatten()
        .map(|mut c| {
            c.args = serde_json::from_str(c.args.as_str().unwrap_or("{}")).unwrap_or(json!({}));
            c
        })
        .collect();
    if calls.is_empty() {
        Ok(ChatResp::Text(text))
    } else {
        Ok(ChatResp::Calls { text, calls })
    }
}

async fn chat_openai(
    http: &reqwest::Client,
    cfg: &ProviderCfg,
    system: &str,
    msgs: &[Msg],
) -> Result<ChatResp> {
    let resp = http
        .post(openai_url(&cfg.base))
        .bearer_auth(&cfg.key)
        .json(&json!({
            "model": cfg.model,
            "messages": to_openai_msgs(system, msgs),
            "tools": tools_openai(),
        }))
        .send()
        .await
        .context("openai 请求失败")?;
    let status = resp.status();
    let v: serde_json::Value = resp.json().await.context("openai 返回非 JSON")?;
    if !status.is_success() {
        let msg = v["error"]["message"].as_str().unwrap_or("unknown");
        bail!("openai {status}: {msg}");
    }
    let m = &v["choices"][0]["message"];
    let text = m["content"].as_str().unwrap_or("").to_string();
    let calls: Vec<ToolCall> = m["tool_calls"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|c| {
                    Some(ToolCall {
                        id: c["id"].as_str()?.to_string(),
                        name: c["function"]["name"].as_str()?.to_string(),
                        args: serde_json::from_str(
                            c["function"]["arguments"].as_str().unwrap_or("{}"),
                        )
                        .unwrap_or(json!({})),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if calls.is_empty() {
        Ok(ChatResp::Text(text))
    } else {
        Ok(ChatResp::Calls { text, calls })
    }
}

async fn chat_anthropic(
    http: &reqwest::Client,
    cfg: &ProviderCfg,
    system: &str,
    msgs: &[Msg],
) -> Result<ChatResp> {
    let resp = http
        .post(anthropic_url(&cfg.base))
        .header("x-api-key", &cfg.key)
        .header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": cfg.model,
            "max_tokens": 8192,
            "system": system,
            "messages": to_anthropic_msgs(msgs),
            "tools": tools_anthropic(),
        }))
        .send()
        .await
        .context("anthropic 请求失败")?;
    let status = resp.status();
    let v: serde_json::Value = resp.json().await.context("anthropic 返回非 JSON")?;
    if !status.is_success() {
        let msg = v["error"]["message"].as_str().unwrap_or("unknown");
        bail!("anthropic {status}: {msg}");
    }
    let mut text = String::new();
    let mut calls = Vec::new();
    for b in v["content"].as_array().cloned().unwrap_or_default() {
        match b["type"].as_str() {
            Some("text") => text.push_str(b["text"].as_str().unwrap_or("")),
            Some("tool_use") => {
                if let (Some(id), Some(name)) = (b["id"].as_str(), b["name"].as_str()) {
                    calls.push(ToolCall {
                        id: id.to_string(),
                        name: name.to_string(),
                        args: b["input"].clone(),
                    });
                }
            }
            _ => {}
        }
    }
    if calls.is_empty() {
        Ok(ChatResp::Text(text))
    } else {
        Ok(ChatResp::Calls { text, calls })
    }
}

/// 拉取模型列表：OpenAI GET {base}/models / Anthropic GET {base}/v1/models。
pub async fn models(
    http: &reqwest::Client,
    provider: &str,
    base: &str,
    key: &str,
) -> Result<Vec<String>> {
    let base = default_base(provider, base.to_string());
    let req = if provider == "anthropic" {
        http.get(anthropic_models_url(&base))
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
    } else {
        http.get(openai_models_url(&base)).bearer_auth(key)
    };
    let resp = req.send().await.context("models 请求失败")?;
    let status = resp.status();
    let v: serde_json::Value = resp.json().await.context("models 返回非 JSON")?;
    if !status.is_success() {
        let msg = v["error"]["message"].as_str().unwrap_or("unknown");
        bail!("models {status}: {msg}");
    }
    let mut ids: Vec<String> = v["data"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| m["id"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    Ok(ids)
}

// ---------- 文件块兜底解析 ----------

/// 解析 <<<FILE:路径>>> 文件块（不支持工具的接口的兜底输出格式）。
pub fn parse_files(text: &str) -> Vec<(String, String)> {
    const MARK: &str = "<<<FILE:";
    let mut out = Vec::new();
    let mut pos = 0usize;
    while let Some(rel) = text[pos..].find(MARK) {
        let name_start = pos + rel + MARK.len();
        let name_end = text[name_start..]
            .find(">>>")
            .map(|n| name_start + n)
            .unwrap_or(text.len());
        let name = text[name_start..name_end].trim().to_string();
        let body_start = (name_end + 3).min(text.len());
        let body_end = text[body_start..]
            .find(MARK)
            .map(|n| body_start + n)
            .unwrap_or(text.len());
        let body = text[body_start..body_end].trim().to_string();
        if !name.is_empty() {
            out.push((name, body));
        }
        pos = body_end;
    }
    out
}

/// 过滤危险路径（../、绝对路径），统一分隔符。
pub fn sanitize_path(name: &str) -> Option<String> {
    let n = name.trim_start_matches('/').replace('\\', "/");
    if n.is_empty() || n.contains("..") {
        None
    } else {
        Some(n)
    }
}

/// 把 <<<FILE:>>> 块从展示文本里剔除，留下纯说明部分。
pub fn strip_file_blocks(text: &str) -> String {
    const MARK: &str = "<<<FILE:";
    let mut out = String::new();
    let mut pos = 0usize;
    while let Some(rel) = text[pos..].find(MARK) {
        out.push_str(&text[pos..pos + rel]);
        let name_start = pos + rel + MARK.len();
        let name_end = text[name_start..]
            .find(">>>")
            .map(|n| name_start + n)
            .unwrap_or(text.len());
        let body_start = (name_end + 3).min(text.len());
        pos = text[body_start..]
            .find(MARK)
            .map(|n| body_start + n)
            .unwrap_or(text.len());
    }
    out.push_str(&text[pos.min(text.len())..]);
    out.trim().to_string()
}

/// 校验生成物：plugin.json 能 parse、id 合法、入口文件存在。
pub fn validate_files(files: &[(String, String)], entry: &str) -> Result<()> {
    let manifest_text = files
        .iter()
        .find(|(n, _)| n == "plugin.json")
        .map(|(_, c)| c.clone())
        .context("缺少 plugin.json")?;
    let m: serde_json::Value =
        serde_json::from_str(&manifest_text).context("plugin.json 不是合法 JSON")?;
    let id = m["id"].as_str().unwrap_or("");
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        bail!("plugin.json 的 id「{id}」只允许小写字母/数字/中划线");
    }
    if !files.iter().any(|(n, _)| n == entry) {
        bail!("缺少入口文件 {entry}");
    }
    Ok(())
}

pub fn system_prompt(runtime: &str) -> String {
    let (sdk, entry, lang) = match runtime {
        "python" => (SDK_PY, "main.py", "Python"),
        _ => (SDK_JS, "main.js", "JavaScript(Node.js)"),
    };
    format!(
        "你是「椎名真白」QQ 机器人框架的插件生成器，根据用户需求用 {lang} 生成插件。\n\n\
        # 插件规范\n{SPEC}\n\n\
        # 插件 SDK 源码（{lang}）\n```\n{sdk}\n```\n\n\
        # 工作方式（重要）\n\
        - 用 write_file 工具逐个写入插件文件；plugin.json 和 {entry} 必须写\n\
        - 代码量大时支持多文件布局：拆成入口 + 辅助文件（JS 用 require(\"./x.js\")，Python 用 import x），路径写相对插件目录\n\
        - 【接口纪律】只用规范里列出的 bot./ctx. 接口；严禁访问 __mashiro_transport、callApi 等 SDK 内部对象；\
          规范没有的方法不要发明，拿不准先查规范\n\
        - 若用户已「载入工作区」：文件集就是真实插件目录（写/删直接改盘上真文件），改完 install_plugin 即热重载\n\
        - 写完**必须**调用 install_plugin 做真实加载校验；返回 error 时修复后再次调用，直到成功\n\
        - read_file / list_files / delete_file 可随时自查当前文件集\n\
        - 用户要求「修改」时：read_file 看现状 → write_file 重写 → install_plugin 复验\n\
        - 需求不明确或有多个合理实现方向时，先用 ask_user 工具向用户提问确认（给选项），拿到答案再写文件\n\
        - 全部完成后**只用文字**总结改了什么，不要在回复里贴文件内容\n\n\
        # 规则\n\
        - plugin.json 的 id 只允许小写字母/数字/中划线；entry 填 {entry}\n\
        - permissions 只声明用到的能力（send/recall/kv/timer/http/group_admin/media）\n\
        - 插件有用户可调参数时（密钥/开关/阈值），在 plugin.json 加 config_schema 声明（见规范），代码里用 bot.config(key) 读\n\
        - cmd:<指令> 注册即生效：消息文本以指令开头即触发插件；event:<事件名> 订阅事件流\n\
        - JS 插件只能 require(\"mashiro\") 和 Node 内置模块；Python 插件只用标准库和 mashiro\n\
        - 若接口不支持工具调用，可退化为输出 <<<FILE:相对路径>>> 文件块（每块到下一个 <<<FILE 或结束）\n\n\
        # 内置工具（用 mcp_call 调用，name + args）\n{mcp_cat}\n\
        - mcp 工具用于调试/检查：plugin_invoke 可模拟触发验证插件行为，logs_query 看插件报错；会真实发消息/卸载的工具谨慎用\n",
        lang = lang,
        SPEC = SPEC,
        sdk = sdk,
        entry = entry,
        mcp_cat = crate::mcp::catalog_text(),
    )
}
