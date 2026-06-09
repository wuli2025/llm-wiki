//! Codex(ChatGPT) 翻译代理 —— 让 `claude` 用上 ChatGPT 订阅。
//!
//! 思路同 cc-switch 的 `proxy/`, 但**不背 axum/hyper/reqwest/tokio 全家桶**: 入站用
//! `std::net` 手写一条路由的 HTTP/1.1 + SSE 本地服务, 出站继续用现成的 `ureq`(流式读响应体)。
//!
//!   claude  ──Anthropic /v1/messages(SSE)──▶  本代理  ──Responses(SSE)──▶  chatgpt.com/backend-api/codex
//!           ◀────── Anthropic SSE ──────────         ◀──── OpenAI SSE ────────
//!
//! - 鉴权: 读 `~/.codex/auth.json`(坞里 Codex 授权已写好的 ChatGPT OAuth token), access_token
//!   将过期或上游 401 时用 refresh_token 静默刷新并回写。
//! - 翻译: system→instructions、messages→input、tools→function、tool_use/tool_result ↔
//!   function_call/function_call_output; 流式把 Responses 的 output_text/function_call_arguments
//!   增量翻成 Anthropic 的 content_block_delta。reasoning(思维链)在 v1 直接不透传。
//! - 模型: 默认 gpt-5.5(最新 ChatGPT 模型); 收到 gpt-*/o* 透传, 其余(claude-*)回落默认。
//!
//! 注: ChatGPT 后端的请求契约(必需头/字段)可能随官方调整, 出错文案会经 `last_error()` 暴露到坞里。

use crate::provider::{
    codex_auth_path, codex_b64url_decode, codex_rfc3339_now, CODEX_CLIENT_ID, CODEX_OAUTH_TOKEN_URL,
};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const BACKEND_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
const DEFAULT_MODEL: &str = "gpt-5.5";
const USER_AGENT: &str = "polaris-codex-proxy";
const BASE_PORT: u16 = 8765;
const PORT_TRIES: u16 = 25;
/// 入站 socket 读/写超时: 半发请求或谎报 Content-Length 的慢连接不会让连接线程永久阻塞泄漏。
const IO_TIMEOUT: Duration = Duration::from_secs(60);
/// 上游连接建立超时。
const UPSTREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// 上游 SSE 流「两次读之间」的最大静默 (超过即判上游半死) —— **不是整条流的总时长上限**,
/// 故不会误杀真的在持续吐 token 的长回复; 只在上游接受连接后彻底不发数据时兜底解除阻塞。
const UPSTREAM_READ_TIMEOUT: Duration = Duration::from_secs(120);
/// 请求体大小硬上限: 挡 `Content-Length` 谎报超大值导致 `vec![0u8; N]` 瞬时巨量分配的内存放大 DoS。
const MAX_BODY: usize = 64 * 1024 * 1024;

static PROXY_PORT: Lazy<RwLock<Option<u16>>> = Lazy::new(|| RwLock::new(None));
static LAST_ERROR: Lazy<RwLock<String>> = Lazy::new(|| RwLock::new(String::new()));
static COUNTER: AtomicU64 = AtomicU64::new(1);

fn set_error(s: String) {
    *LAST_ERROR.write() = s;
}

/// 当前代理端口(未启动则 None)
pub fn port() -> Option<u16> {
    *PROXY_PORT.read()
}

/// 最近一次上游/鉴权错误(供坞里展示)
pub fn last_error() -> String {
    LAST_ERROR.read().clone()
}

/// 确保代理已启动, 返回监听端口。已在跑则直接返回端口(幂等)。
pub fn ensure_running() -> Result<u16, String> {
    if let Some(p) = *PROXY_PORT.read() {
        return Ok(p);
    }
    let mut guard = PROXY_PORT.write();
    if let Some(p) = *guard {
        return Ok(p); // 另一线程刚起好
    }
    let listener = bind_port()?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("读取代理端口失败: {e}"))?
        .port();
    std::thread::spawn(move || accept_loop(listener));
    *guard = Some(port);
    Ok(port)
}

/// 从 BASE_PORT 起逐个尝试绑定, 避开被占用端口
fn bind_port() -> Result<TcpListener, String> {
    for off in 0..PORT_TRIES {
        let p = BASE_PORT + off;
        if let Ok(l) = TcpListener::bind(("127.0.0.1", p)) {
            return Ok(l);
        }
    }
    Err(format!(
        "无法在 {}–{} 间绑定本地端口(都被占用)",
        BASE_PORT,
        BASE_PORT + PORT_TRIES - 1
    ))
}

fn accept_loop(listener: TcpListener) {
    for stream in listener.incoming().flatten() {
        std::thread::spawn(move || {
            let _ = handle_conn(stream);
        });
    }
}

// ───────────────────────── HTTP/1.1 入站 ─────────────────────────

fn handle_conn(mut stream: TcpStream) -> std::io::Result<()> {
    // 入站读/写都设超时, 防慢连接/半发请求把连接线程永久阻塞 → 线程只增不减泄漏。
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let clone = stream.try_clone()?;
    let _ = clone.set_read_timeout(Some(IO_TIMEOUT));
    let mut reader = BufReader::new(clone);

    // 请求行: METHOD PATH HTTP/1.1
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();

    // 头: 只关心 Content-Length, 读到空行为止
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let t = line.trim_end();
        if t.is_empty() {
            break;
        }
        if let Some((k, v)) = t.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse().unwrap_or(0);
            }
        }
    }

    if content_length > MAX_BODY {
        anthropic_error(&mut stream, 413, "请求体过大");
        return Ok(());
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }

    if method == "POST" && path.starts_with("/v1/messages/count_tokens") {
        // claude 偶尔预估上下文 token; 给个粗略估算(字符数/4)即可, 别 404 把它卡住。
        let est = (body.len() / 4).max(1) as u64;
        let out = serde_json::to_vec(&json!({ "input_tokens": est })).unwrap_or_default();
        write_simple(&mut stream, 200, "application/json", &out);
    } else if method == "POST" && path.starts_with("/v1/messages") {
        handle_messages(&mut stream, &body);
    } else if method == "GET" {
        write_simple(
            &mut stream,
            200,
            "application/json",
            b"{\"ok\":true,\"service\":\"polaris-codex-proxy\"}",
        );
    } else {
        anthropic_error(&mut stream, 404, "未知路由");
    }
    Ok(())
}

fn write_simple(stream: &mut TcpStream, code: u16, ctype: &str, body: &[u8]) {
    let status = match code {
        200 => "200 OK",
        400 => "400 Bad Request",
        401 => "401 Unauthorized",
        404 => "404 Not Found",
        413 => "413 Payload Too Large",
        502 => "502 Bad Gateway",
        _ => "500 Internal Server Error",
    };
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

/// 以 Anthropic 错误格式(顶层 JSON)回吐, claude 会把 message 显示给用户
fn anthropic_error(stream: &mut TcpStream, code: u16, message: &str) {
    let etype = match code {
        400 => "invalid_request_error",
        401 => "authentication_error",
        404 => "not_found_error",
        _ => "api_error",
    };
    let body = json!({ "type": "error", "error": { "type": etype, "message": message } });
    let bytes = serde_json::to_vec(&body).unwrap_or_default();
    write_simple(stream, code, "application/json", &bytes);
}

fn write_event(stream: &mut TcpStream, event: &str, data: &Value) -> std::io::Result<()> {
    let payload = serde_json::to_string(data).unwrap_or_else(|_| "{}".into());
    let frame = format!("event: {event}\ndata: {payload}\n\n");
    stream.write_all(frame.as_bytes())?;
    stream.flush()
}

// ───────────────────────── /v1/messages 主流程 ─────────────────────────

fn handle_messages(stream: &mut TcpStream, body: &[u8]) {
    let req: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => return anthropic_error(stream, 400, &format!("请求体不是合法 JSON: {e}")),
    };
    let want_stream = req.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);

    let auth = match load_auth() {
        Ok(a) => a,
        Err(e) => {
            set_error(e.clone());
            return anthropic_error(stream, 401, &e);
        }
    };

    let upstream = match build_responses_body(&req) {
        Ok(b) => b,
        Err(e) => return anthropic_error(stream, 400, &e),
    };

    // 调上游: 401/403 先刷新 token 重试一次
    let resp = match call_upstream(&auth, &upstream) {
        Ok(r) => r,
        Err(UpstreamErr::Unauthorized) => match refresh_auth(&auth) {
            Ok(a2) => match call_upstream(&a2, &upstream) {
                Ok(r) => r,
                Err(e) => {
                    let m = e.message();
                    set_error(m.clone());
                    return anthropic_error(stream, 502, &m);
                }
            },
            Err(e) => {
                set_error(e.clone());
                return anthropic_error(stream, 401, &e);
            }
        },
        Err(e) => {
            let m = e.message();
            set_error(m.clone());
            return anthropic_error(stream, 502, &m);
        }
    };

    if want_stream {
        stream_translate(stream, resp);
    } else {
        buffer_translate(stream, resp);
    }
}

enum UpstreamErr {
    Unauthorized,
    Http(u16, String),
    Transport(String),
}
impl UpstreamErr {
    fn message(&self) -> String {
        match self {
            UpstreamErr::Unauthorized => "ChatGPT 授权已失效, 请在坞里重新授权 Codex".into(),
            UpstreamErr::Http(c, b) => format!("ChatGPT 后端 HTTP {c}: {b}"),
            UpstreamErr::Transport(t) => format!("连接 ChatGPT 后端失败: {t}"),
        }
    }
}

/// 带超时的上游 agent: connect/read/write 都设上限。read 用 per-read 超时 (见 UPSTREAM_READ_TIMEOUT
/// 注释), 不设整条 call 的全局 deadline, 以免误杀正在持续吐 token 的长 SSE 回复。
fn upstream_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(UPSTREAM_CONNECT_TIMEOUT)
        .timeout_read(UPSTREAM_READ_TIMEOUT)
        .timeout_write(IO_TIMEOUT)
        .build()
}

fn call_upstream(auth: &Auth, body: &Value) -> Result<ureq::Response, UpstreamErr> {
    let session = gen_uuid();
    let r = upstream_agent()
        .post(BACKEND_URL)
        .set("Authorization", &format!("Bearer {}", auth.access_token))
        .set("chatgpt-account-id", &auth.account_id)
        .set("OpenAI-Beta", "responses=experimental")
        .set("originator", "codex_cli_rs")
        .set("session_id", &session)
        .set("Accept", "text/event-stream")
        .set("Content-Type", "application/json")
        .set("User-Agent", USER_AGENT)
        .send_json(body.clone());
    match r {
        Ok(resp) => Ok(resp),
        Err(ureq::Error::Status(401, _)) | Err(ureq::Error::Status(403, _)) => {
            Err(UpstreamErr::Unauthorized)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let b = resp.into_string().unwrap_or_default();
            Err(UpstreamErr::Http(code, b.chars().take(400).collect()))
        }
        Err(ureq::Error::Transport(t)) => Err(UpstreamErr::Transport(t.to_string())),
    }
}

// ───────────────────────── Anthropic → Responses 请求翻译 ─────────────────────────

fn build_responses_body(req: &Value) -> Result<Value, String> {
    let model = map_model(req.get("model").and_then(|v| v.as_str()).unwrap_or(""));
    let mut instructions = extract_system(req);
    if instructions.trim().is_empty() {
        instructions = "You are a helpful coding assistant.".to_string();
    }
    let input = build_input(req);
    if input.is_empty() {
        return Err("messages 为空".into());
    }
    let tools = build_tools(req);

    let mut body = json!({
        "model": model,
        "instructions": instructions,
        "input": input,
        "store": false,
        "stream": true,
        // gpt-5.5 是最新 ChatGPT 模型; 推理模型(o1/o3)用 reasoning 字段
        "reasoning": { "effort": "medium" },
    });
    let obj = body.as_object_mut().unwrap();
    if !tools.is_empty() {
        obj.insert("tools".into(), Value::Array(tools));
        obj.insert("tool_choice".into(), json!("auto"));
        obj.insert("parallel_tool_calls".into(), json!(false));
    }
    if let Some(mt) = req.get("max_tokens").and_then(|v| v.as_u64()) {
        obj.insert("max_output_tokens".into(), json!(mt));
    }
    if let Some(t) = req.get("temperature").and_then(|v| v.as_f64()) {
        obj.insert("temperature".into(), json!(t));
    }
    Ok(body)
}

/// 模型映射: 空→默认; gpt-*/o*/codex 透传; 其余(claude-* 等)→默认 codex 模型
fn map_model(m: &str) -> String {
    let low = m.to_ascii_lowercase();
    if low.is_empty() {
        return DEFAULT_MODEL.into();
    }
    if low.contains("codex")
        || low.starts_with("gpt")
        || low.starts_with("o1")
        || low.starts_with("o3")
        || low.starts_with("o4")
    {
        return m.to_string();
    }
    DEFAULT_MODEL.into()
}

/// system: 字符串或 block 数组 → 拼成 instructions
fn extract_system(req: &Value) -> String {
    match req.get("system") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

/// messages → Responses input 项数组。文本/图片合成 message 项; tool_use/tool_result
/// 单独成 function_call / function_call_output 项(遇到时先把已累积的 message parts 落盘)。
fn build_input(req: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    let Some(msgs) = req.get("messages").and_then(|m| m.as_array()) else {
        return out;
    };
    for msg in msgs {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
        match msg.get("content") {
            Some(Value::String(s)) => out.push(message_item(role, s)),
            Some(Value::Array(blocks)) => {
                let mut parts: Vec<Value> = Vec::new();
                let flush = |out: &mut Vec<Value>, parts: &mut Vec<Value>| {
                    if !parts.is_empty() {
                        out.push(wrap_message(role, std::mem::take(parts)));
                    }
                };
                for b in blocks {
                    match b.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                        "text" => {
                            let text = b.get("text").and_then(|t| t.as_str()).unwrap_or("");
                            let kind = if role == "assistant" {
                                "output_text"
                            } else {
                                "input_text"
                            };
                            parts.push(json!({ "type": kind, "text": text }));
                        }
                        "image" => {
                            if let Some(url) = image_data_url(b) {
                                parts.push(json!({ "type": "input_image", "image_url": url }));
                            }
                        }
                        "tool_use" => {
                            flush(&mut out, &mut parts);
                            let id = b.get("id").and_then(|v| v.as_str()).unwrap_or("");
                            let name = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
                            let args = b.get("input").cloned().unwrap_or_else(|| json!({}));
                            let args_str =
                                serde_json::to_string(&args).unwrap_or_else(|_| "{}".into());
                            out.push(json!({
                                "type": "function_call",
                                "call_id": id,
                                "name": name,
                                "arguments": args_str,
                            }));
                        }
                        "tool_result" => {
                            flush(&mut out, &mut parts);
                            let id = b.get("tool_use_id").and_then(|v| v.as_str()).unwrap_or("");
                            out.push(json!({
                                "type": "function_call_output",
                                "call_id": id,
                                "output": tool_result_text(b),
                            }));
                        }
                        _ => {}
                    }
                }
                if !parts.is_empty() {
                    out.push(wrap_message(role, parts));
                }
            }
            _ => {}
        }
    }
    out
}

fn message_item(role: &str, text: &str) -> Value {
    let kind = if role == "assistant" {
        "output_text"
    } else {
        "input_text"
    };
    json!({ "type": "message", "role": role, "content": [{ "type": kind, "text": text }] })
}
fn wrap_message(role: &str, parts: Vec<Value>) -> Value {
    json!({ "type": "message", "role": role, "content": parts })
}

fn tool_result_text(b: &Value) -> String {
    match b.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|x| x.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn image_data_url(b: &Value) -> Option<String> {
    let src = b.get("source")?;
    match src.get("type").and_then(|t| t.as_str()).unwrap_or("") {
        "base64" => {
            let media = src
                .get("media_type")
                .and_then(|t| t.as_str())
                .unwrap_or("image/png");
            let data = src.get("data").and_then(|t| t.as_str()).unwrap_or("");
            Some(format!("data:{media};base64,{data}"))
        }
        "url" => src.get("url").and_then(|t| t.as_str()).map(String::from),
        _ => None,
    }
}

fn build_tools(req: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    let Some(tools) = req.get("tools").and_then(|t| t.as_array()) else {
        return out;
    };
    for t in tools {
        let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let desc = t.get("description").and_then(|v| v.as_str()).unwrap_or("");
        let schema = t
            .get("input_schema")
            .cloned()
            .unwrap_or_else(|| json!({ "type": "object" }));
        out.push(json!({
            "type": "function",
            "name": name,
            "description": desc,
            "parameters": schema,
        }));
    }
    out
}

// ───────────────────────── Responses SSE → Anthropic SSE 响应翻译 ─────────────────────────

/// 从上游 Responses 流里解析出的规范化事件
enum Norm {
    TextDelta(String),
    ToolStart { id: String, name: String },
    ToolArgs(String),
    ToolStop,
    Done {
        stop: String,
        input_tokens: u64,
        output_tokens: u64,
    },
    Failed(String),
}

/// 逐行读上游 SSE, 把 Responses 事件归一成 Norm 回调给 emit。
fn drive_upstream<R: Read>(resp_reader: R, mut emit: impl FnMut(Norm)) {
    let mut reader = BufReader::new(resp_reader);
    let mut saw_tool = false;
    let mut in_tool = false;
    let mut tool_args_streamed = false;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        let line = line.trim_end();
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        match v.get("type").and_then(|x| x.as_str()).unwrap_or("") {
            "response.output_text.delta" => {
                if let Some(d) = v.get("delta").and_then(|x| x.as_str()) {
                    emit(Norm::TextDelta(d.to_string()));
                }
            }
            "response.output_item.added" => {
                let item = v.get("item");
                if item.and_then(|i| i.get("type")).and_then(|x| x.as_str()) == Some("function_call")
                {
                    let item = item.unwrap();
                    let id = item
                        .get("call_id")
                        .and_then(|x| x.as_str())
                        .or_else(|| item.get("id").and_then(|x| x.as_str()))
                        .unwrap_or("")
                        .to_string();
                    let name = item.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    saw_tool = true;
                    in_tool = true;
                    tool_args_streamed = false;
                    emit(Norm::ToolStart { id, name });
                }
            }
            "response.function_call_arguments.delta" => {
                if let Some(d) = v.get("delta").and_then(|x| x.as_str()) {
                    tool_args_streamed = true;
                    emit(Norm::ToolArgs(d.to_string()));
                }
            }
            "response.function_call_arguments.done" => {
                // 后端只给完整 arguments 不给增量时, 在此补发一次
                if in_tool && !tool_args_streamed {
                    if let Some(a) = v.get("arguments").and_then(|x| x.as_str()) {
                        emit(Norm::ToolArgs(a.to_string()));
                    }
                }
            }
            "response.output_item.done" => {
                if in_tool
                    && v.get("item").and_then(|i| i.get("type")).and_then(|x| x.as_str())
                        == Some("function_call")
                {
                    in_tool = false;
                    emit(Norm::ToolStop);
                }
            }
            "response.completed" => {
                let (mut it, mut ot) = (0u64, 0u64);
                if let Some(u) = v.get("response").and_then(|r| r.get("usage")) {
                    it = u.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
                    ot = u.get("output_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
                }
                emit(Norm::Done {
                    stop: if saw_tool { "tool_use" } else { "end_turn" }.into(),
                    input_tokens: it,
                    output_tokens: ot,
                });
            }
            "response.failed" | "error" => {
                let msg = v
                    .get("response")
                    .and_then(|r| r.get("error"))
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .or_else(|| {
                        v.get("error")
                            .and_then(|e| e.get("message"))
                            .and_then(|m| m.as_str())
                    })
                    .or_else(|| v.get("message").and_then(|m| m.as_str()))
                    .unwrap_or("上游返回错误")
                    .to_string();
                emit(Norm::Failed(msg));
            }
            _ => {}
        }
    }
}

/// 流式: 把 Norm 事件实时翻成 Anthropic SSE 写回 claude
fn stream_translate(client: &mut TcpStream, resp: ureq::Response) {
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
    if client.write_all(head.as_bytes()).is_err() {
        return;
    }

    let msg_id = format!("msg_{}", gen_id());
    let _ = write_event(
        client,
        "message_start",
        &json!({
            "type": "message_start",
            "message": {
                "id": msg_id,
                "type": "message",
                "role": "assistant",
                "model": DEFAULT_MODEL,
                "content": [],
                "stop_reason": Value::Null,
                "stop_sequence": Value::Null,
                "usage": { "input_tokens": 0, "output_tokens": 0 },
            }
        }),
    );

    let mut index: i64 = -1;
    let mut text_open = false;
    let mut tool_open = false;
    let mut done_sent = false;

    drive_upstream(resp.into_reader(), |ev| match ev {
        Norm::TextDelta(t) => {
            if tool_open {
                let _ = write_event(client, "content_block_stop", &json!({ "type": "content_block_stop", "index": index }));
                tool_open = false;
            }
            if !text_open {
                index += 1;
                let _ = write_event(client, "content_block_start", &json!({
                    "type": "content_block_start", "index": index,
                    "content_block": { "type": "text", "text": "" }
                }));
                text_open = true;
            }
            let _ = write_event(client, "content_block_delta", &json!({
                "type": "content_block_delta", "index": index,
                "delta": { "type": "text_delta", "text": t }
            }));
        }
        Norm::ToolStart { id, name } => {
            if text_open {
                let _ = write_event(client, "content_block_stop", &json!({ "type": "content_block_stop", "index": index }));
                text_open = false;
            }
            index += 1;
            tool_open = true;
            let _ = write_event(client, "content_block_start", &json!({
                "type": "content_block_start", "index": index,
                "content_block": { "type": "tool_use", "id": id, "name": name, "input": {} }
            }));
        }
        Norm::ToolArgs(a) => {
            if tool_open {
                let _ = write_event(client, "content_block_delta", &json!({
                    "type": "content_block_delta", "index": index,
                    "delta": { "type": "input_json_delta", "partial_json": a }
                }));
            }
        }
        Norm::ToolStop => {
            if tool_open {
                let _ = write_event(client, "content_block_stop", &json!({ "type": "content_block_stop", "index": index }));
                tool_open = false;
            }
        }
        Norm::Done { stop, input_tokens, output_tokens } => {
            if text_open {
                let _ = write_event(client, "content_block_stop", &json!({ "type": "content_block_stop", "index": index }));
                text_open = false;
            }
            if tool_open {
                let _ = write_event(client, "content_block_stop", &json!({ "type": "content_block_stop", "index": index }));
                tool_open = false;
            }
            let _ = write_event(client, "message_delta", &json!({
                "type": "message_delta",
                "delta": { "stop_reason": stop, "stop_sequence": Value::Null },
                "usage": { "input_tokens": input_tokens, "output_tokens": output_tokens }
            }));
            let _ = write_event(client, "message_stop", &json!({ "type": "message_stop" }));
            done_sent = true;
        }
        Norm::Failed(msg) => {
            set_error(msg.clone());
            let _ = write_event(client, "error", &json!({
                "type": "error", "error": { "type": "api_error", "message": msg }
            }));
            done_sent = true;
        }
    });

    if !done_sent {
        // 上游中途断流: 优雅收尾, 别让 claude 一直挂着
        if text_open {
            let _ = write_event(client, "content_block_stop", &json!({ "type": "content_block_stop", "index": index }));
        }
        if tool_open {
            let _ = write_event(client, "content_block_stop", &json!({ "type": "content_block_stop", "index": index }));
        }
        let _ = write_event(client, "message_delta", &json!({
            "type": "message_delta",
            "delta": { "stop_reason": "end_turn", "stop_sequence": Value::Null },
            "usage": { "input_tokens": 0, "output_tokens": 0 }
        }));
        let _ = write_event(client, "message_stop", &json!({ "type": "message_stop" }));
    }
    let _ = client.flush();
}

/// 非流式(stream:false): 累积成完整 Anthropic message JSON 一次性返回
fn buffer_translate(client: &mut TcpStream, resp: ureq::Response) {
    let mut blocks: Vec<Value> = Vec::new();
    let mut cur_text = String::new();
    let mut cur_tool: Option<(String, String, String)> = None; // id, name, args
    let mut stop_reason = "end_turn".to_string();
    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;
    let mut err: Option<String> = None;

    drive_upstream(resp.into_reader(), |ev| match ev {
        Norm::TextDelta(t) => cur_text.push_str(&t),
        Norm::ToolStart { id, name } => {
            if !cur_text.is_empty() {
                blocks.push(json!({ "type": "text", "text": cur_text.clone() }));
                cur_text.clear();
            }
            cur_tool = Some((id, name, String::new()));
        }
        Norm::ToolArgs(a) => {
            if let Some(t) = cur_tool.as_mut() {
                t.2.push_str(&a);
            }
        }
        Norm::ToolStop => {
            if let Some((id, name, args)) = cur_tool.take() {
                let input: Value = serde_json::from_str(&args).unwrap_or_else(|_| json!({}));
                blocks.push(json!({ "type": "tool_use", "id": id, "name": name, "input": input }));
            }
        }
        Norm::Done { stop, input_tokens: it, output_tokens: ot } => {
            stop_reason = stop;
            input_tokens = it;
            output_tokens = ot;
        }
        Norm::Failed(m) => err = Some(m),
    });
    if !cur_text.is_empty() {
        blocks.push(json!({ "type": "text", "text": cur_text }));
    }

    if let Some(m) = err {
        set_error(m.clone());
        return anthropic_error(client, 502, &m);
    }
    let body = json!({
        "id": format!("msg_{}", gen_id()),
        "type": "message",
        "role": "assistant",
        "model": DEFAULT_MODEL,
        "content": blocks,
        "stop_reason": stop_reason,
        "stop_sequence": Value::Null,
        "usage": { "input_tokens": input_tokens, "output_tokens": output_tokens },
    });
    let bytes = serde_json::to_vec(&body).unwrap_or_default();
    write_simple(client, 200, "application/json", &bytes);
}

// ───────────────────────── 鉴权: 读 ~/.codex/auth.json + 刷新 ─────────────────────────

struct Auth {
    access_token: String,
    refresh_token: String,
    account_id: String,
}

fn load_auth() -> Result<Auth, String> {
    let path = codex_auth_path().ok_or_else(|| "无法定位 ~/.codex/auth.json".to_string())?;
    let text = std::fs::read_to_string(&path)
        .map_err(|_| "未找到 ChatGPT 授权, 请先在坞里授权 Codex".to_string())?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("auth.json 解析失败: {e}"))?;
    let tokens = v
        .get("tokens")
        .ok_or_else(|| "auth.json 缺少 tokens, 请重新授权".to_string())?;
    let access = tokens.get("access_token").and_then(|x| x.as_str()).unwrap_or("").to_string();
    if access.is_empty() {
        return Err("ChatGPT 授权无效(缺 access_token), 请重新授权".into());
    }
    let mut auth = Auth {
        access_token: access,
        refresh_token: tokens.get("refresh_token").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        account_id: tokens.get("account_id").and_then(|x| x.as_str()).unwrap_or("").to_string(),
    };
    // access_token 将过期则主动刷新(失败不致命: 还可能在上游 401 时再刷一次)
    if token_expiring(&auth.access_token) {
        if let Ok(a2) = refresh_auth(&auth) {
            auth = a2;
        }
    }
    Ok(auth)
}

/// 解 JWT 的 exp, 距现在不足 60s 即视为将过期
fn token_expiring(access: &str) -> bool {
    let Some(payload) = access.split('.').nth(1) else {
        return false;
    };
    let Some(bytes) = codex_b64url_decode(payload) else {
        return false;
    };
    let Ok(claims) = serde_json::from_slice::<Value>(&bytes) else {
        return false;
    };
    let Some(exp) = claims.get("exp").and_then(|x| x.as_i64()) else {
        return false;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    now + 60 >= exp
}

fn refresh_auth(auth: &Auth) -> Result<Auth, String> {
    if auth.refresh_token.is_empty() {
        return Err("缺少 refresh_token, 请重新授权 ChatGPT".into());
    }
    let resp = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .build()
        .post(CODEX_OAUTH_TOKEN_URL)
        .set("User-Agent", USER_AGENT)
        .send_form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &auth.refresh_token),
            ("client_id", CODEX_CLIENT_ID),
            ("scope", "openid profile email"),
        ])
        .map_err(|e| format!("刷新 ChatGPT token 失败: {}", short_err(e)))?;
    let v: Value = resp.into_json().map_err(|e| format!("解析刷新响应失败: {e}"))?;
    let access = v.get("access_token").and_then(|x| x.as_str()).unwrap_or("").to_string();
    if access.is_empty() {
        return Err("刷新响应缺少 access_token".into());
    }
    let refresh = v
        .get("refresh_token")
        .and_then(|x| x.as_str())
        .map(String::from)
        .unwrap_or_else(|| auth.refresh_token.clone());
    let id_token = v.get("id_token").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let new = Auth {
        access_token: access,
        refresh_token: refresh,
        account_id: auth.account_id.clone(),
    };
    persist_auth(&new, &id_token);
    Ok(new)
}

/// 刷新后按官方格式回写 auth.json(id_token 若未返回则保留旧值), 外部 codex CLI 也能续用
fn persist_auth(auth: &Auth, id_token: &str) {
    let Some(path) = codex_auth_path() else {
        return;
    };
    let existing_id = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| {
            v.get("tokens")
                .and_then(|t| t.get("id_token"))
                .and_then(|x| x.as_str())
                .map(String::from)
        })
        .unwrap_or_default();
    let id = if id_token.is_empty() {
        existing_id
    } else {
        id_token.to_string()
    };
    let body = json!({
        "OPENAI_API_KEY": Value::Null,
        "tokens": {
            "id_token": id,
            "access_token": auth.access_token,
            "refresh_token": auth.refresh_token,
            "account_id": auth.account_id,
        },
        "last_refresh": codex_rfc3339_now(),
    });
    if let Ok(txt) = serde_json::to_string_pretty(&body) {
        let _ = std::fs::write(&path, txt);
    }
}

fn short_err(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, resp) => {
            let b = resp.into_string().unwrap_or_default();
            format!("HTTP {code} {}", b.chars().take(200).collect::<String>())
        }
        ureq::Error::Transport(t) => format!("网络错误: {t}"),
    }
}

// ───────────────────────── 杂项 ─────────────────────────

fn gen_id() -> String {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let c = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{n:x}{c:x}")
}

/// 伪 uuid v4(够后端当 session_id 用), 不引 uuid crate
fn gen_uuid() -> String {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let c = COUNTER.fetch_add(1, Ordering::Relaxed);
    let a = (n & 0xffff_ffff) as u32;
    let b = ((n >> 32) & 0xffff) as u16;
    let cc = (((n >> 48) & 0x0fff) as u16) | 0x4000;
    let d = ((c & 0x3fff) as u16) | 0x8000;
    let e = (n >> 16) & 0xffff_ffff_ffff;
    format!("{a:08x}-{b:04x}-{cc:04x}-{d:04x}-{e:012x}")
}

// ───────────────────────── Command: 代理状态(供坞展示) ─────────────────────────

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexProxyInfo {
    pub running: bool,
    pub port: u16,
    pub last_error: String,
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn codex_proxy_info() -> CodexProxyInfo {
    let p = port();
    CodexProxyInfo {
        running: p.is_some(),
        port: p.unwrap_or(0),
        last_error: last_error(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn map_model_passthrough_and_default() {
        assert_eq!(map_model("gpt-5-codex"), "gpt-5-codex");
        assert_eq!(map_model("o3-mini"), "o3-mini");
        assert_eq!(map_model("codex-foo"), "codex-foo");
        // 空 / claude-* / 未知 → 回落默认 codex 模型
        assert_eq!(map_model("").as_str(), DEFAULT_MODEL);
        assert_eq!(map_model("claude-opus-4").as_str(), DEFAULT_MODEL);
    }

    #[test]
    fn extract_system_handles_all_shapes() {
        assert_eq!(extract_system(&json!({"system": "hi"})), "hi");
        assert_eq!(
            extract_system(&json!({"system": [{"text":"a"},{"text":"b"}]})),
            "a\n\nb"
        );
        assert_eq!(extract_system(&json!({})), "");
        // 畸形: system 是数字 / block 缺 text → 不 panic, 返回空串
        assert_eq!(extract_system(&json!({"system": 42})), "");
        assert_eq!(extract_system(&json!({"system": [{"nope": 1}]})), "");
    }

    #[test]
    fn build_responses_body_rejects_empty_messages() {
        // 空 / 缺 messages → Err (而非 panic)
        assert!(build_responses_body(&json!({"model":"gpt-5","messages":[]})).is_err());
        assert!(build_responses_body(&json!({})).is_err());
    }

    #[test]
    fn build_responses_body_minimal_ok() {
        let req = json!({
            "model": "gpt-5-codex",
            "messages": [{"role":"user","content":"hello"}],
            "max_tokens": 100,
            "temperature": 0.5,
        });
        let body = build_responses_body(&req).expect("应翻译成功");
        assert_eq!(body["model"], "gpt-5-codex");
        assert_eq!(body["max_output_tokens"], 100);
        assert_eq!(body["stream"], true);
        assert!(body["input"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false));
    }

    #[test]
    fn build_responses_body_tolerates_malformed_messages() {
        // content/结构各种畸形: 只要求不 panic (健壮性回归保护)
        let weird = json!({
            "model": "gpt-5",
            "messages": [
                {"role":"user","content": 12345},
                {"role":"assistant"},
                {"content": [{"type":"text"}]},
                {"role":"user","content":[{"type":"image","source":{}}]},
                {"role":"user","content":[{"type":"tool_use"}]},
                {"role":"user","content":[{"type":"tool_result"}]},
                "not-an-object",
                42
            ]
        });
        let _ = build_responses_body(&weird);
    }
}
