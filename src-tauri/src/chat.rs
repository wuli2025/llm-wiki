//! 板块 ① 对话核心 — MVP v0.2 (stderr 透传 + 项目/对话历史)
//!
//! 设计依据: PRD-v6 §7
//! - chat_send: 组装 prompt(KB 注入) -> spawn claude CLI -> emit chat:stream
//! - 同时读 stdout + stderr (单独线程), stderr 转 error 事件
//! - child.wait 完成后, 检查 exit code, 非 0 时 emit error
//! - 沙箱模式预检容器是否在运行, 不在时直接返回错误
//! - 整合 conv 模块, 自动写 user/assistant 消息

use crate::claude_md;
use crate::conv;
use crate::kb;
use crate::skills;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

pub fn init(_app: &AppHandle) -> Result<(), anyhow::Error> {
    Ok(())
}

// ───────────────────────── Types ─────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Manual,
    AutoCurrent,
    AutoAll,
    Deny,
}

impl PermissionMode {
    fn cli_value(&self) -> &'static str {
        match self {
            PermissionMode::Manual => "default",
            PermissionMode::AutoCurrent => "acceptEdits",
            // AutoAll 不再 bypass permissions，与 AutoCurrent 一致
            PermissionMode::AutoAll => "acceptEdits",
            PermissionMode::Deny => "plan",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSendArgs {
    pub prompt: String,
    pub permission_mode: PermissionMode,
    #[serde(default)]
    pub use_sandbox: bool,
    #[serde(default)]
    pub skill_id: Option<String>,
    #[serde(default)]
    pub conversation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatStreamEvent {
    pub req_id: String,
    pub kind: String, // delta | tool | error | done
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
}

// ───────────────────────── State ─────────────────────────

static CHILDREN: once_cell::sync::Lazy<Arc<Mutex<HashMap<String, Child>>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));
static REQ_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_req_id() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let c = REQ_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("req-{:x}-{:x}", ts, c)
}

// ───────────────────────── Commands ──────────────────────

#[tauri::command]
pub async fn chat_send(app: AppHandle, args: ChatSendArgs) -> Result<String, String> {
    let req_id = next_req_id();

    // 把 user 消息写入对话历史 (若提供 conversation_id)
    if let Some(cid) = &args.conversation_id {
        let _ = conv::append_message(cid, "user", &args.prompt);
    }

    // 一体注入: Skill prompt → KB CLAUDE.md + kb_search 召回 → 用户问题
    let current_project_id = args
        .conversation_id
        .as_deref()
        .and_then(conv::project_id_of_conversation);
    let cm_ctx = claude_md::render_for_project(current_project_id.as_deref(), &args.prompt);

    let mut final_prompt = String::new();

    // 1. Skill system prompt（显式激活或意图自动检测）
    let active_skill = args
        .skill_id
        .as_deref()
        .and_then(skills::find)
        .or_else(|| skills::default_skill_for_intent(&args.prompt));
    if let Some(skill) = active_skill {
        final_prompt.push_str(&skill.system_prompt);
        final_prompt.push_str("\n\n---\n\n");
    }

    // 2. CLAUDE.md 上下文
    if !cm_ctx.is_empty() {
        final_prompt.push_str(&cm_ctx);
        final_prompt.push_str("\n\n## 用户问题\n\n");
    }

    // 3. 用户原始问题
    final_prompt.push_str(&args.prompt);

    let perm = args.permission_mode.cli_value();
    let conv_id_opt = args.conversation_id.clone();

    // 默认走宿主机执行（沙箱可选，但默认关闭）
    let mut child = spawn_on_host(&final_prompt, perm)?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "claude 子进程没有 stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "claude 子进程没有 stderr".to_string())?;

    CHILDREN.lock().insert(req_id.clone(), child);

    // stderr 读线程: 任何 stderr 行都 emit 为 error 事件; 累积起来给 wait 用
    let app_err = app.clone();
    let req_err = req_id.clone();
    let stderr_buf = Arc::new(Mutex::new(String::new()));
    let stderr_buf_clone = stderr_buf.clone();
    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            let Ok(line) = line else { continue };
            if line.trim().is_empty() {
                continue;
            }
            stderr_buf_clone.lock().push_str(&line);
            stderr_buf_clone.lock().push('\n');
            emit_event(
                &app_err,
                ChatStreamEvent {
                    req_id: req_err.clone(),
                    kind: "error".into(),
                    text: Some(format!("[stderr] {}", line)),
                    tool: None,
                    conversation_id: None,
                },
            );
        }
    });

    // stdout 读线程: stream-json -> 事件; 累积 assistant 文本
    let app_out = app.clone();
    let req_out = req_id.clone();
    let conv_id_thread = conv_id_opt.clone();
    let stderr_buf_for_done = stderr_buf.clone();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        let mut assistant_text = String::new();
        for line in reader.lines() {
            let Ok(line) = line else { continue };
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Value>(&line) {
                Ok(v) => handle_stream_event(&app_out, &req_out, &v, &mut assistant_text),
                Err(_) => {
                    // 非 JSON 行: 当作 delta 直接显示 (调试友好)
                    assistant_text.push_str(&line);
                    assistant_text.push('\n');
                    emit_event(
                        &app_out,
                        ChatStreamEvent {
                            req_id: req_out.clone(),
                            kind: "delta".into(),
                            text: Some(line),
                            tool: None,
                            conversation_id: None,
                        },
                    );
                }
            }
        }

        // 等子进程退出, 检查 exit code (不能持锁 wait, 否则 chat_cancel 死锁)
        let child_opt = CHILDREN.lock().remove(&req_out);
        let exit_msg: Option<String> = if let Some(mut child) = child_opt {
            match child.wait() {
                Ok(status) => {
                    if !status.success() {
                        let stderr_txt = stderr_buf_for_done.lock().clone();
                        Some(format!(
                            "claude 进程异常退出 (exit code={:?})\n--- stderr ---\n{}",
                            status.code(),
                            if stderr_txt.is_empty() {
                                "(stderr 为空)".to_string()
                            } else {
                                stderr_txt
                            }
                        ))
                    } else {
                        None
                    }
                }
                Err(e) => Some(format!("等待 claude 进程失败: {}", e)),
            }
        } else {
            None
        };

        if let Some(msg) = exit_msg {
            emit_event(
                &app_out,
                ChatStreamEvent {
                    req_id: req_out.clone(),
                    kind: "error".into(),
                    text: Some(msg),
                    tool: None,
                    conversation_id: None,
                },
            );
        }

        // 持久化 assistant 消息
        if let Some(cid) = &conv_id_thread {
            if !assistant_text.trim().is_empty() {
                let _ = conv::append_message(cid, "assistant", assistant_text.trim());
            }
        }

        emit_event(
            &app_out,
            ChatStreamEvent {
                req_id: req_out.clone(),
                kind: "done".into(),
                text: None,
                tool: None,
                conversation_id: conv_id_thread.clone(),
            },
        );
    });

    Ok(req_id)
}

#[tauri::command]
pub fn chat_cancel(req_id: String) -> Result<(), String> {
    if let Some(mut child) = CHILDREN.lock().remove(&req_id) {
        let _ = child.kill();
    }
    Ok(())
}

// ───────────────────────── Internals ─────────────────────

fn handle_stream_event(app: &AppHandle, req_id: &str, v: &Value, accum: &mut String) {
    let t = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
    match t {
        "assistant" => {
            if let Some(content) = v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_array())
            {
                for block in content {
                    let bt = block.get("type").and_then(|x| x.as_str()).unwrap_or("");
                    match bt {
                        "text" => {
                            if let Some(txt) = block.get("text").and_then(|x| x.as_str()) {
                                accum.push_str(txt);
                                emit_event(
                                    app,
                                    ChatStreamEvent {
                                        req_id: req_id.into(),
                                        kind: "delta".into(),
                                        text: Some(txt.to_string()),
                                        tool: None,
                                        conversation_id: None,
                                    },
                                );
                            }
                        }
                        "tool_use" => {
                            let name = block
                                .get("name")
                                .and_then(|x| x.as_str())
                                .unwrap_or("unknown");
                            emit_event(
                                app,
                                ChatStreamEvent {
                                    req_id: req_id.into(),
                                    kind: "tool".into(),
                                    text: None,
                                    tool: Some(name.to_string()),
                                    conversation_id: None,
                                },
                            );
                        }
                        _ => {}
                    }
                }
            }
        }
        "result" => {
            // result 事件: claude --print 模式收尾, result 字段是最终文本
            if let Some(txt) = v.get("result").and_then(|x| x.as_str()) {
                // 若前面已经有 assistant text, result 通常是同一内容的最终版, 不重复显示
                if accum.is_empty() {
                    accum.push_str(txt);
                    emit_event(
                        app,
                        ChatStreamEvent {
                            req_id: req_id.into(),
                            kind: "delta".into(),
                            text: Some(txt.to_string()),
                            tool: None,
                            conversation_id: None,
                        },
                    );
                }
            }
            // error subtype
            if let Some(subtype) = v.get("subtype").and_then(|x| x.as_str()) {
                if subtype.starts_with("error") {
                    let msg = v
                        .get("result")
                        .and_then(|x| x.as_str())
                        .unwrap_or("(unknown error)")
                        .to_string();
                    emit_event(
                        app,
                        ChatStreamEvent {
                            req_id: req_id.into(),
                            kind: "error".into(),
                            text: Some(format!("[result error: {}] {}", subtype, msg)),
                            tool: None,
                            conversation_id: None,
                        },
                    );
                }
            }
        }
        _ => {}
    }
}

fn emit_event(app: &AppHandle, ev: ChatStreamEvent) {
    let _ = app.emit("chat:stream", ev);
}

fn spawn_in_sandbox(prompt: &str, perm: &str) -> Result<Child, String> {
    let perm_flag = format!("--permission-mode={}", perm);
    // 沙箱内 KB 永远挂在 /kb (sandbox_start 时挂载),
    // 这里让 claude 把 /kb 也加进可读目录,并以 /workspace 为 cwd
    let child = Command::new("docker")
        .args([
            "exec",
            "-i",
            "-w",
            "/workspace",
            polaris_sandbox::CONTAINER_NAME,
            "claude",
            "--print",
            "--output-format",
            "stream-json",
            "--verbose",
            "--add-dir",
            "/kb",
            &perm_flag,
            prompt,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("在沙箱内调起 claude 失败: {}", e))?;
    Ok(child)
}

fn spawn_on_host(prompt: &str, perm: &str) -> Result<Child, String> {
    let perm_flag = format!("--permission-mode={}", perm);
    // cwd = polaris-app 根 (env!("CARGO_MANIFEST_DIR") 的父级),
    // 这样 claude CLI 自动信任整棵 polaris-app/ 子树, 包括 PolarisKB/
    let cwd = claude_md::project_root().unwrap_or_else(|| {
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
    });

    // 如果 KB root 不在 cwd 子树下(用户可能把 KB 移到别处), 用 --add-dir 显式放行
    let kb_root = std::path::PathBuf::from(kb::kb_root());
    let mut extra_dirs: Vec<String> = Vec::new();
    if !kb_root.as_os_str().is_empty() && kb_root.exists() && !kb_root.starts_with(&cwd) {
        extra_dirs.push("--add-dir".into());
        extra_dirs.push(kb_root.to_string_lossy().to_string());
    }

    let mut args: Vec<String> = vec![
        "--print".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
    ];
    args.extend(extra_dirs);
    args.push(perm_flag);
    args.push(prompt.to_string());

    let child = Command::new("claude")
        .args(&args)
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("调起宿主机 claude CLI 失败: {}", e))?;
    Ok(child)
}
