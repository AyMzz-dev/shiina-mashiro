use serde_json::{json, Value};

/// NDJSON 帧协议：stdin/stdout 一行一个 JSON。
///
///   宿主 → 插件: {"id":N,"method":"invoke","params":{...}}      请求（要回包）
///                {"type":"shutdown"}                            通知
///   插件 → 宿主: {"id":N,"result":{...},"error":null}            回包
///                {"id":N,"method":"send_message","params":{}}   反向调框架 API
///                {"type":"register","data":{...}}               通知（注册/日志）

/// 编码一条宿主侧请求（返回值不含换行，调用方补 "\n"）。
pub fn request(id: i64, method: &str, params: Value) -> String {
    json!({ "id": id, "method": method, "params": params }).to_string()
}

/// 编码一条对插件反向请求的应答。
pub fn respond(id: i64, result: Result<Value, String>) -> String {
    match result {
        Ok(v) => json!({ "id": id, "result": v, "error": null }),
        Err(e) => json!({ "id": id, "result": null, "error": e }),
    }
    .to_string()
}

/// 宿主读到的上行消息。
#[derive(Debug)]
pub enum Inbound {
    /// 插件回包
    Response {
        id: i64,
        result: Value,
        error: Option<String>,
    },
    /// 插件调用框架 API（plugin/token = 宿主标注的来源身份凭据）
    ApiCall {
        id: i64,
        method: String,
        params: Value,
        plugin: Option<String>,
        token: Option<String>,
    },
    /// 通知：register / log / event-ack ...
    Notify { kind: String, data: Value },
}

/// 解码一行 NDJSON；字段结构不合法返回 None。
pub fn decode(line: &str) -> Option<Inbound> {
    let v: Value = serde_json::from_str(line).ok()?;
    let id = v.get("id").and_then(Value::as_i64);

    if let (Some(id), Some(method)) = (id, v.get("method").and_then(Value::as_str)) {
        return Some(Inbound::ApiCall {
            id,
            method: method.to_string(),
            params: v.get("params").cloned().unwrap_or(Value::Null),
            plugin: v.get("plugin").and_then(Value::as_str).map(str::to_string),
            token: v.get("token").and_then(Value::as_str).map(str::to_string),
        });
    }
    if let Some(id) = id {
        return Some(Inbound::Response {
            id,
            result: v.get("result").cloned().unwrap_or(Value::Null),
            error: v.get("error").and_then(Value::as_str).map(str::to_string),
        });
    }
    if let Some(kind) = v.get("type").and_then(Value::as_str) {
        return Some(Inbound::Notify {
            kind: kind.to_string(),
            data: v.get("data").cloned().unwrap_or(Value::Null),
        });
    }
    None
}
