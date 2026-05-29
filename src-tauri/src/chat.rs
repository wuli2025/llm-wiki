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
use directories::UserDirs;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use walkdir::WalkDir;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 给从 GUI 进程拉起的子进程加 `CREATE_NO_WINDOW`：宿主是窗口子系统、本身没有控制台，
/// 直接 spawn 控制台子系统的 claude.exe / docker.exe 会被分配一个新控制台 → 每次发消息
/// 都弹一个黑色终端窗口。加这个标志让它隐藏式运行，用户看不到终端。
#[cfg_attr(not(windows), allow(unused_variables))]
fn no_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

pub fn init(_app: &AppHandle) -> Result<(), anyhow::Error> {
    Ok(())
}

/// 默认预授权的联网工具 (逗号分隔, 传给 `--allowedTools`)。
/// 把内置 WebSearch / WebFetch 设为「联网搜索默认打开」: 任何权限模式都不再拦截,
/// 深度搜索 / 联网搜索因此能真正联网检索, 而不是退回内置知识。
const DEFAULT_WEB_TOOLS: &str = "WebSearch,WebFetch";

/// 非「拒绝授权」档位下额外放行的本地工具。
/// 缘由: headless (`--print`, stdin=null) 模式下没有人能逐个点「同意」, `acceptEdits`
/// 只自动批准文件编辑而 **不含执行**, 于是 claude 能写出 `create_pptx.py` 却跑不了
/// `python create_pptx.py` → .pptx / .xlsx / 图表这类「要执行脚本才能产出」的成品全部卡死
/// (实测 permission_denials 五连拒, 工具名是 Windows 的 `PowerShell`)。
/// 这里显式放行本地读写 + 执行 (Windows shell 工具叫 `PowerShell`, 跨平台再带上 `Bash`),
/// 让成品能真正落地。危险兜底仍由「拒绝授权(plan, 只读)」档位提供。
const LOCAL_WORK_TOOLS: &str = "Read,Write,Edit,Glob,Grep,Bash,PowerShell";

/// 按权限档位 (cli_value: default | acceptEdits | plan) 组装 `--allowedTools`。
/// - plan (拒绝授权 / 只读): 仅联网工具, 不放行任何本地执行;
/// - default / acceptEdits (手动 / 自动): 联网 + 本地读写执行, 成品能真正产出。
fn allowed_tools_for(perm: &str) -> String {
    if perm == "plan" {
        DEFAULT_WEB_TOOLS.to_string()
    } else {
        format!("{},{}", DEFAULT_WEB_TOOLS, LOCAL_WORK_TOOLS)
    }
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
    pub skill_ids: Option<Vec<String>>,
    #[serde(default)]
    pub conversation_id: Option<String>,
    /// 目标模式：完成条件。设置后注入「持续推进直到达成」指令。
    #[serde(default)]
    pub goal: Option<String>,
    /// 「请教毛主席」：注入毛选式客观分析指令，调用毛主席资料库，生成标注来源的 HTML。
    #[serde(default)]
    pub consult_mao: bool,
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

    // 产物目录 (每个会话一份): claude 把成品文件写到这里 → 侧边栏可预览
    let art_dir = artifacts_dir(args.conversation_id.as_deref());
    let _ = std::fs::create_dir_all(&art_dir);
    let art_before = dir_snapshot(&art_dir);

    // 一体注入: Skill prompt → KB CLAUDE.md + kb_search 召回 → 用户问题
    let current_project_id = args
        .conversation_id
        .as_deref()
        .and_then(conv::project_id_of_conversation);
    let cm_ctx = claude_md::render_for_project(current_project_id.as_deref(), &args.prompt);

    let mut final_prompt = String::new();

    // 1. Skill system prompts —— 显式点选 + 按任务意图自动激活（去重）
    let mut injected: Vec<String> = Vec::new();
    // 1a. 用户在对话框显式激活的 skill
    if let Some(ids) = &args.skill_ids {
        for id in ids {
            if injected.iter().any(|x| x == id) {
                continue;
            }
            if let Some((meta, system_prompt)) = skills::find(id) {
                final_prompt.push_str(&system_prompt);
                final_prompt.push('\n');
                injected.push(meta.id);
            }
        }
    }
    // 1b. 按任务意图自动激活（即使对话框没点选）：
    //     创建技能 → skill-creator；网页/浏览器自动化 → cloak-browser
    for (meta, system_prompt) in skills::auto_skills_for_intent(&args.prompt) {
        if injected.iter().any(|x| *x == meta.id) {
            continue;
        }
        final_prompt.push_str(&system_prompt);
        final_prompt.push('\n');
        injected.push(meta.id);
    }
    if !final_prompt.is_empty() {
        final_prompt.push_str("\n---\n\n");
    }

    // 2. 输出文件约定 (Polaris) — 让成品文件落到产物目录, 侧边栏即可预览
    final_prompt.push_str(&output_convention(&art_dir));
    final_prompt.push_str("\n\n---\n\n");

    // 2.5 目标模式: 用户设了完成条件时, 注入「持续推进直到达成」指令
    if let Some(goal) = args
        .goal
        .as_deref()
        .map(str::trim)
        .filter(|g| !g.is_empty())
    {
        final_prompt.push_str(&goal_directive(goal));
        final_prompt.push_str("\n\n---\n\n");
    }

    // 2.6 请教毛主席: 注入毛选式客观分析指令(调资料库 + 生成标来源 HTML)
    if args.consult_mao {
        final_prompt.push_str(&mao_consult_directive(&art_dir));
        final_prompt.push_str("\n\n---\n\n");
    }

    // 2.7 生图能力检测: 用户想生成图片, 但供应商坞里全是文本/代码大模型, 没有一个能真生图。
    //     注入「当前供应商 + 能否真生图」的事实, 让 image-gen 技能据此决定:
    //     不支持 → 用中文说清楚, 并改用「很有图片质感的 HTML」兜底。
    //     模型有时不遵守「开头摊牌」指令(会先说「已生成」), 所以由后端在回复最前面
    //     **确定性地**插入这句中文说明(见下方 image_notice), 保证用户一上来就看到。
    let image_notice: Option<String> = if skills::detect_image_intent(&args.prompt) {
        let (provider_name, supported) = crate::provider::image_gen_capability();
        final_prompt.push_str(&image_capability_directive(&provider_name, supported, &art_dir));
        final_prompt.push_str("\n\n---\n\n");
        if supported {
            None
        } else {
            Some(format!(
                "> ⚠️ **说明**：你当前使用的「{}」是文本大模型，**不支持生成真实图片**。下面用一张「HTML 模拟的画面」来替代；如需真实 AI 生图，请在「API 供应商」里配置支持文生图的图像接口。\n\n",
                provider_name
            ))
        }
    } else {
        None
    };

    // 3. CLAUDE.md 上下文
    if !cm_ctx.is_empty() {
        final_prompt.push_str(&cm_ctx);
        final_prompt.push_str("\n\n## 用户问题\n\n");
    }

    // 4. 用户原始问题
    final_prompt.push_str(&args.prompt);

    let perm = args.permission_mode.cli_value();
    let conv_id_opt = args.conversation_id.clone();

    // 默认走宿主机执行（沙箱可选，但默认关闭）
    let mut child = spawn_on_host(&final_prompt, perm, &art_dir)?;

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
    let conv_id_err = conv_id_opt.clone();
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
                    conversation_id: conv_id_err.clone(),
                },
            );
        }
    });

    // stdout 读线程: stream-json -> 事件; 累积 assistant 文本 + 产物路径
    let app_out = app.clone();
    let req_out = req_id.clone();
    let conv_id_thread = conv_id_opt.clone();
    let stderr_buf_for_done = stderr_buf.clone();
    let art_dir_thread = art_dir.clone();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        let mut assistant_text = String::new();
        // 生图不支持时: 后端确定性地把中文说明作为**第一段**发出去并计入正文,
        // 不依赖模型遵守「开头摊牌」指令 → 用户一定先看到「当前模型不支持生图」。
        if let Some(notice) = image_notice {
            assistant_text.push_str(&notice);
            emit_event(
                &app_out,
                ChatStreamEvent {
                    req_id: req_out.clone(),
                    kind: "delta".into(),
                    text: Some(notice),
                    tool: None,
                    conversation_id: conv_id_thread.clone(),
                },
            );
        }
        // 本轮生成的成品文件 (绝对路径, 正斜杠), 既来自 Write/Edit 工具调用,
        // 也来自产物目录的前后快照 diff (覆盖 Bash/脚本生成的文件)
        let mut artifacts: Vec<String> = Vec::new();
        for line in reader.lines() {
            let Ok(line) = line else { continue };
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Value>(&line) {
                Ok(v) => handle_stream_event(
                    &app_out,
                    &req_out,
                    conv_id_thread.as_deref(),
                    &v,
                    &mut assistant_text,
                    &mut artifacts,
                ),
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
                            conversation_id: conv_id_thread.clone(),
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
                    conversation_id: conv_id_thread.clone(),
                },
            );
        }

        // 产物目录前后快照 diff: 捕获 Bash / 脚本 / Skill 生成的新增或改动文件
        let art_after = dir_snapshot(&art_dir_thread);
        for (path, mtime) in art_after.iter() {
            let changed = match art_before.get(path) {
                None => true,
                Some(old) => mtime > old,
            };
            if !changed {
                continue;
            }
            let s = path.to_string_lossy().replace('\\', "/");
            if !artifacts.contains(&s) {
                artifacts.push(s.clone());
                emit_event(
                    &app_out,
                    ChatStreamEvent {
                        req_id: req_out.clone(),
                        kind: "artifact".into(),
                        text: Some(s),
                        tool: None,
                        conversation_id: conv_id_thread.clone(),
                    },
                );
            }
        }

        // 持久化 assistant 消息 (产物清单以注释 marker 形式存入正文, 重载历史时解析)
        if let Some(cid) = &conv_id_thread {
            let mut content = assistant_text.trim().to_string();
            if !artifacts.is_empty() {
                if let Ok(json) = serde_json::to_string(&artifacts) {
                    content.push_str(&format!("\n\n{}{}-->", ARTIFACT_MARKER_PREFIX, json));
                }
            }
            if !content.trim().is_empty() {
                let _ = conv::append_message(cid, "assistant", &content);
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

fn handle_stream_event(
    app: &AppHandle,
    req_id: &str,
    conv_id: Option<&str>,
    v: &Value,
    accum: &mut String,
    artifacts: &mut Vec<String>,
) {
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
                                        conversation_id: conv_id.map(|s| s.to_string()),
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
                                    conversation_id: conv_id.map(|s| s.to_string()),
                                },
                            );
                            // 写文件类工具 → 记一个成品文件 (实时反馈)
                            if matches!(name, "Write" | "Edit" | "MultiEdit" | "NotebookEdit") {
                                let fp = block
                                    .get("input")
                                    .and_then(|i| {
                                        i.get("file_path").or_else(|| i.get("notebook_path"))
                                    })
                                    .and_then(|x| x.as_str());
                                if let Some(fp) = fp {
                                    let norm = fp.replace('\\', "/");
                                    if !artifacts.contains(&norm) {
                                        artifacts.push(norm.clone());
                                        emit_event(
                                            app,
                                            ChatStreamEvent {
                                                req_id: req_id.into(),
                                                kind: "artifact".into(),
                                                text: Some(norm),
                                                tool: None,
                                                conversation_id: conv_id.map(|s| s.to_string()),
                                            },
                                        );
                                    }
                                }
                            }
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
                            conversation_id: conv_id.map(|s| s.to_string()),
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
                            conversation_id: conv_id.map(|s| s.to_string()),
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
    // 联网 + (非只读档位)本地读写执行, 让成品能真正产出
    let allowed = allowed_tools_for(perm);
    // 沙箱内 KB 永远挂在 /kb (sandbox_start 时挂载),
    // 这里让 claude 把 /kb 也加进可读目录,并以 /workspace 为 cwd
    let mut cmd = Command::new("docker");
    cmd.args([
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
        "--allowedTools",
        &allowed,
        &perm_flag,
        prompt,
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    no_window(&mut cmd); // 隐藏式: 不弹控制台窗口
    let child = cmd
        .spawn()
        .map_err(|e| format!("在沙箱内调起 claude 失败: {}", e))?;
    Ok(child)
}

fn spawn_on_host(prompt: &str, perm: &str, art_dir: &Path) -> Result<Child, String> {
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
    // 产物目录在 ~/Polaris 下, 不在 cwd 子树, 显式放行 claude 可写入
    if art_dir.exists() && !art_dir.starts_with(&cwd) {
        extra_dirs.push("--add-dir".into());
        extra_dirs.push(art_dir.to_string_lossy().to_string());
    }

    let mut args: Vec<String> = vec![
        "--print".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
    ];
    args.extend(extra_dirs);
    // 联网工具默认放行; 非「拒绝授权」档位再叠加本地读写执行 (Bash/PowerShell/文件),
    // 否则 headless 下连 `python xxx.py` 都被拒, .pptx/.xlsx 这类成品根本产不出来。
    args.push("--allowedTools".into());
    args.push(allowed_tools_for(perm));
    args.push(perm_flag);
    args.push(prompt.to_string());

    // 解析 claude 可执行文件的全路径再 spawn, 而非裸名 "claude":
    // npm 装只在 PATH 放 `claude.cmd`, 而 Windows CreateProcessW 解析裸名只补 `.exe`、不查 PATHEXT
    // → 裸名找不到 npm 装的 claude。resolve_claude_exe 会挖出真·原生 exe (原生装 / npm 装通吃);
    // 解析不到再回退裸名靠 PATH (兼容用户自行配好的环境)。
    let claude_bin: std::ffi::OsString = crate::doctor::resolve_claude_exe()
        .map(|p| p.into_os_string())
        .unwrap_or_else(|| "claude".into());
    let mut cmd = Command::new(&claude_bin);
    cmd.args(&args)
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    no_window(&mut cmd); // 隐藏式: 每次发消息不再弹出黑色终端窗口
    let child = cmd
        .spawn()
        .map_err(|e| format!("调起宿主机 claude CLI 失败: {}", e))?;
    Ok(child)
}

// ───────────────────────── Artifacts (产物预览) ─────────────────────────

/// assistant 正文里夹带的产物清单 marker 前缀; 完整形如
/// `<!--POLARIS_ARTIFACTS:["C:/a/b.html"]-->`, 重载历史时由前端解析并隐藏。
pub const ARTIFACT_MARKER_PREFIX: &str = "<!--POLARIS_ARTIFACTS:";

/// 每个会话一个目录。优先落到「工作文件夹」(KB root) 下，让产物与用户的知识库
/// 同处一地、可见可备份：`<kb_root>/conversations/<id>/`。
/// KB root 不可用时回退到 `~/Polaris/data/artifacts/<id>`。
fn conversation_dir(conv_id: Option<&str>) -> PathBuf {
    let id = conv_id.unwrap_or("scratch");
    let kb_root = PathBuf::from(kb::kb_root());
    if !kb_root.as_os_str().is_empty() && kb_root.exists() {
        kb_root.join("conversations").join(id)
    } else {
        UserDirs::new()
            .map(|u| u.home_dir().join("Polaris").join("data").join("artifacts"))
            .unwrap_or_else(|| PathBuf::from("artifacts"))
            .join(id)
    }
}

/// 产物(成品)目录: 会话目录下的 `outputs/`。claude 把成品写到这里 → 侧边栏可预览。
fn artifacts_dir(conv_id: Option<&str>) -> PathBuf {
    conversation_dir(conv_id).join("outputs")
}

/// 递归快照目录里的文件 → mtime, 用于前后 diff 找新增/改动文件
fn dir_snapshot(dir: &Path) -> HashMap<PathBuf, SystemTime> {
    let mut m = HashMap::new();
    if !dir.exists() {
        return m;
    }
    for entry in WalkDir::new(dir).into_iter().flatten() {
        if entry.file_type().is_file() {
            if let Ok(meta) = entry.metadata() {
                if let Ok(mt) = meta.modified() {
                    m.insert(entry.path().to_path_buf(), mt);
                }
            }
        }
    }
    m
}

/// 注入给 claude 的「输出文件约定」, 引导成品落到产物目录
fn output_convention(art_dir: &Path) -> String {
    let dir = art_dir.to_string_lossy().replace('\\', "/");
    format!(
        "## 输出文件约定 (Polaris)\n\n\
当你生成任何可供用户**查看或下载的成品文件**(HTML 网页 / 数据可视化 / 报告 / Markdown / 图片 / CSV / PDF 等)时,请遵守:\n\n\
1. 把成品文件保存到这个已授权可写的目录(用绝对路径):\n   `{dir}`\n\
2. 网页类成品请优先生成**单文件、自包含的 HTML**(把 CSS/JS 内联进去),以便在侧边栏直接预览。\n\
3. 在回答末尾用一句话点明你生成了哪些文件(文件名即可)。\n\n\
普通问答无需创建文件。",
        dir = dir
    )
}

/// 目标模式指令: 把用户设定的「完成条件」当作直接指令, 引导 claude 持续推进直到达成,
/// 对应 Claude Code 的 goal 模式 —— 条件未满足前不收尾、不反问, 自行规划下一步。
fn goal_directive(goal: &str) -> String {
    format!(
        "## 目标模式 (Goal Mode)\n\n\
本轮已开启**目标模式**。用户设定的完成条件是:\n\n\
> {goal}\n\n\
把这个条件本身当作你的指令, 持续推进直到它真正达成:\n\
1. 条件未满足时不要收尾, 也不要反问用户「接下来做什么」—— 自行规划并执行下一步。\n\
2. 每完成一步, 对照条件自检是否已达成; 未达成就继续做, 直到满足为止。\n\
3. 条件达成后, 明确说明它已达成, 并简述你是如何确认的。\n\
4. 仅当遇到无法自行解决的硬阻塞(如缺少凭据 / 权限 / 外部依赖)时, 才停下来向用户说明原因。",
        goal = goal
    )
}

/// 生图能力指令: 把「当前供应商 + 能否真生图」作为事实交给模型。
/// supported=false(绝大多数情况)时, 要求一开始就用中文讲清「当前模型不支持生成真实图片」,
/// 再用「很有图片质感的自包含 HTML」兜底; supported=true 才允许走真实图像 API。
fn image_capability_directive(provider_name: &str, supported: bool, art_dir: &Path) -> String {
    let dir = art_dir.to_string_lossy().replace('\\', "/");
    if supported {
        format!(
            "## 生图能力检测 (Image Capability)\n\n\
本轮检测到用户想**生成图片**, 且环境里配置了独立的图像 API 密钥(`OPENAI_API_KEY`)。\n\
- 可以走真实文生图: 按 image-gen 技能的说明调用图像 API 生成位图, 存到产物目录(绝对路径): `{dir}`。\n\
- 若调用过程中报错(额度 / 网络 / 该 key 无图像权限), **立即用中文如实告知用户**, 再用下面的 HTML 兜底, 不要假装已生成。",
            dir = dir
        )
    } else {
        format!(
            "## 生图能力检测 (Image Capability) — 关键\n\n\
本轮检测到用户想**生成图片(写实照片 / AI 绘画类位图)**。但用户当前用的供应商是 **「{provider}」**, \
它(以及供应商坞里其它走 Anthropic 协议的文本 / 代码大模型)**并不具备文生图能力**, 环境里也没有配置独立的图像生成 API 密钥。\n\n\
因此请**严格**这样做:\n\
1. 本应用**已经在你这条回复的最前面自动插入了一句中文说明**(「你当前使用的「{provider}」不支持生成真实图片…」), 用户一定会先看到它。所以**你不要再重复这句开头、也不要说「已生成」**, 直接从下面第 2 步动手。\n\
2. **用「很有图片质感」的自包含 HTML 兜底**: 按 image-gen 技能的要求, 用 CSS 渐变 / SVG / 几何构图 / 排版做出一张**看起来就像那张图**的单文件 HTML(海报 / 插画 / 场景感), 存到产物目录(绝对路径): `{dir}`, 让用户在侧边栏直接看到。\n\
3. 末尾用一句中文点明: 这是用 HTML 模拟的图片效果, 如需**真实 AI 生图**, 可在「API 供应商」里配置支持文生图的图像 API(如 OpenAI 图像接口 `OPENAI_API_KEY`)。\n\
4. 例外: 如果用户其实要的是**图表 / 流程图 / 示意图 / 图标 / SVG**, 这些能用代码(SVG / HTML / matplotlib)直接画出来, **不受上面限制** —— 正常生成即可, 无需声明「不支持」。",
            provider = provider_name,
            dir = dir
        )
    }
}

/// 「请教毛主席」指令: 让 claude 以毛主席(毛选)的口吻和思想方法, 沿毛主席资料库
/// 客观分析用户的问题, 并生成一份标注来源的自包含 HTML。资料库(结构化 wiki)已由
/// `claude_md::render_for_project` 以长上下文 + 双链地图注入, 用 Read/Glob/Grep 沿双链自取。
fn mao_consult_directive(art_dir: &Path) -> String {
    let dir = art_dir.to_string_lossy().replace('\\', "/");
    format!(
        "## 请教毛主席 (Consult Mode)\n\n\
本轮用户开启了「请教毛主席」模式。请你 **化身毛主席(毛泽东同志)**, 就同志提出的问题给出回答, \
**核心交付物是一份 HTML 文件**, 对话里只作简要铺垫。\n\n\
**口吻与文风(毛选风格 · 大白话第一)**\n\
- 写成《毛泽东选集》那种调子: 平实有力、口语化, 多用大白话、多打比方, 庄稼汉、工人都听得懂; \
不要堆术语、不要掉书袋、不要写论文腔。\n\
- 称呼用户为「同志」或「小同志」, 自称用「我」。\n\
- 句子要短, 段落要短, 一段讲一个道理; 道理要落到「该怎么办」上, 不空谈。\n\n\
**分析方法**(自觉运用毛主席的思想方法, 客观地分析)\n\
- 实事求是, 一切从实际出发; 调查研究, 没有调查就没有发言权。\n\
- 矛盾分析法: 抓主要矛盾和矛盾的主要方面, 一分为二地看问题(两点论)。\n\
- 具体问题具体分析, 反对教条主义; 从群众中来, 到群众中去。\n\n\
**面向未来的视角(必须)**\n\
- 不能只就事论事停在当下, 要 **站在未来的眼光回头看今天这件事**: 长远走向是什么? \
五年、十年、再远些会怎么演变? 现在的小苗头, 将来会长成什么样的大势?\n\
- 善于看「萌芽中的新东西」: 哪些事情现在看着不起眼, 将来要成主流; 哪些事情现在风光, \
将来要被时代抛掉。把这个判断讲清楚, 才算给同志一个真有用的答复。\n\n\
**引用要克制(重要)**\n\
- **正文一开头不要堆原文/语录**, 不要拿《矛盾论》《实践论》整段往上糊。先用大白话把问题讲透, \
该点出处就一句话带过(例如「这就是《矛盾论》里讲的抓主要矛盾」)。\n\
- 详尽的篇目引用、原文摘抄, **统一放到 HTML 文件末尾的「来源」一节**, 不要散在正文上头。\n\n\
**立场底线**\n\
- 立场坚定地拥护社会主义、共产主义事业和人民利益; **回答不得违背共产主义、社会主义的基本原则**, \
不传播违背社会主义核心价值观的内容。\n\n\
**取证 (KB-first)**\n\
- 毛主席资料库(结构化 wiki)已随本轮以长上下文 + 双链地图注入, 就在你工作目录的 `PolarisKB/` 下。\n\
- 下笔前先用 Read/Glob/Grep 翻 `raw/毛主席/` 下相关篇目(如《矛盾论》《实践论》《论持久战》《关于正确处理人民内部矛盾的问题》等)取证, \
不要凭空发挥; 引用就标明篇目名。\n\n\
**输出步骤**\n\
1. 对话里 **只写简短铺垫**: 一两段毛主席口吻的大白话, 点出抓哪个主要矛盾、看到什么未来走向。\
不要在对话里铺长篇, 详细的分析交给 HTML。\n\
2. 生成一份 **单文件、自包含的 HTML**(CSS 内联, 字体可读、排版清爽)保存到这个可写目录(用绝对路径):\n   `{dir}`\n\
   HTML 内容结构建议:\n\
     - 标题 (问题概括)\n\
     - 「实事求是」: 把问题摆平, 大白话讲清楚现状\n\
     - 「主要矛盾」: 抓住主要矛盾和矛盾的主要方面, 一分为二地看\n\
     - 「该怎么办」: 给同志几条具体的、能落地的办法\n\
     - 「站在未来看今天」: 长远走向、未来五年十年的演变、现在该种什么苗\n\
     - 「来源」: 列出引用的篇目, 必要的原文摘抄集中放这里\n\
   **正文开头不要罗列原文**, 把原文压到「来源」一节去。\n\
3. 对话末尾用一句话点明生成了哪个 HTML 文件(绝对路径), 方便同志打开。\n\n\
结尾可以用一句鼓励的话, 例如「为人民服务」「为建设共产主义事业而奋斗」。",
        dir = dir
    )
}

/// 标准 Base64 编码 (无外部依赖) — 给图片产物拼 data URL 用
fn base64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn classify_ext(ext: &str) -> &'static str {
    match ext {
        "html" | "htm" => "html",
        "svg" => "svg",
        "md" | "markdown" => "markdown",
        "png" | "apng" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "avif" => "image",
        "txt" | "json" | "csv" | "tsv" | "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx" | "css"
        | "scss" | "less" | "py" | "rs" | "go" | "java" | "c" | "cpp" | "h" | "hpp" | "toml"
        | "yaml" | "yml" | "xml" | "log" | "sh" | "bat" | "ps1" | "sql" | "ini" | "conf"
        | "env" | "vue" | "php" | "rb" | "kt" | "swift" | "" => "text",
        _ => "binary",
    }
}

fn mime_for(ext: &str) -> &'static str {
    match ext {
        "png" | "apng" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactPayload {
    pub path: String,
    pub name: String,
    pub ext: String,
    /// html | svg | image | markdown | text | binary
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_url: Option<String>,
    pub size: u64,
}

#[tauri::command]
pub fn artifact_read(path: String) -> Result<ArtifactPayload, String> {
    let p = PathBuf::from(&path);
    let meta = std::fs::metadata(&p).map_err(|_| format!("文件不存在或无法访问: {}", path))?;
    if !meta.is_file() {
        return Err("目标不是文件".into());
    }
    let size = meta.len();
    let name = p
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.clone());
    let ext = p
        .extension()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let kind = classify_ext(&ext);

    match kind {
        "image" => {
            const MAX: u64 = 25 * 1024 * 1024;
            if size > MAX {
                return Err("图片过大, 无法预览 (>25MB)".into());
            }
            let bytes = std::fs::read(&p).map_err(|e| e.to_string())?;
            let data_url = format!("data:{};base64,{}", mime_for(&ext), base64_encode(&bytes));
            Ok(ArtifactPayload {
                path,
                name,
                ext,
                kind: kind.into(),
                text: None,
                data_url: Some(data_url),
                size,
            })
        }
        "binary" => Ok(ArtifactPayload {
            path,
            name,
            ext,
            kind: kind.into(),
            text: None,
            data_url: None,
            size,
        }),
        _ => {
            // html / svg / markdown / text
            const MAX: u64 = 8 * 1024 * 1024;
            if size > MAX {
                return Err("文件过大, 无法预览 (>8MB)".into());
            }
            let text = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
            Ok(ArtifactPayload {
                path,
                name,
                ext,
                kind: kind.into(),
                text: Some(text),
                data_url: None,
                size,
            })
        }
    }
}

/// 用系统默认程序打开产物文件 (浏览器开 HTML / 看图器开图片等)
#[tauri::command]
pub fn artifact_open_external(path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/C", "start", "", &path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 在系统文件管理器中定位并选中该产物文件 (Windows 资源管理器 / macOS Finder)。
/// Linux 无统一「选中文件」语义, 退化为打开其所在目录。
#[tauri::command]
pub fn artifact_reveal(path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // explorer /select 需要反斜杠路径; 用 raw_arg 让路径被正确引号包裹
        let win_path = path.replace('/', "\\");
        Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", win_path))
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .args(["-R", &path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let parent = std::path::Path::new(&path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        Command::new("xdg-open")
            .arg(&parent)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 「参考资料」文件夹视图的一条文件记录。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactEntry {
    /// 绝对路径 (正斜杠), 供 artifact_read / openExternal 用
    pub path: String,
    pub name: String,
    pub ext: String,
    /// html | svg | image | markdown | text | binary —— 前端选图标 / 预览方式
    pub kind: String,
    pub size: u64,
    /// 修改时间 (Unix 秒), 前端按此倒序 + 显示
    pub modified: u64,
}

/// 列出某会话产物目录下的全部成品文件, 按修改时间倒序 (最新在前)。
/// 供右侧抽屉「参考资料」以文件夹视图按时间排列、点开即预览。
#[tauri::command]
pub fn artifact_list(conversation_id: Option<String>) -> Vec<ArtifactEntry> {
    let dir = artifacts_dir(conversation_id.as_deref());
    let mut entries: Vec<ArtifactEntry> = Vec::new();
    if !dir.exists() {
        return entries;
    }
    for w in WalkDir::new(&dir).into_iter().flatten() {
        if !w.file_type().is_file() {
            continue;
        }
        let p = w.path();
        let meta = match w.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let name = p
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        // 跳过隐藏 / 临时文件
        if name.starts_with('.') {
            continue;
        }
        let ext = p
            .extension()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        entries.push(ArtifactEntry {
            path: p.to_string_lossy().replace('\\', "/"),
            name,
            ext: ext.clone(),
            kind: classify_ext(&ext).to_string(),
            size: meta.len(),
            modified,
        });
    }
    entries.sort_by(|a, b| b.modified.cmp(&a.modified));
    entries
}

/// 跨「所有对话」产物的搜索命中。供历史对话记忆检索把过往输出文件也算入。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactSearchHit {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub conversation_id: String,
    pub snippet: String,
    pub modified: u64,
    pub score: i32,
}

/// 所有「会话根目录」候选: 工作文件夹(KB root)/conversations 与回退目录。
fn conversation_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let kb_root = PathBuf::from(kb::kb_root());
    if !kb_root.as_os_str().is_empty() && kb_root.exists() {
        roots.push(kb_root.join("conversations"));
    }
    if let Some(u) = UserDirs::new() {
        roots.push(u.home_dir().join("Polaris").join("data").join("artifacts"));
    }
    roots
}

/// 在所有对话的 outputs 里检索: 文件名命中 +10, 正文命中 +2/次(上限), 按分数+时间排序。
/// 让「搜索以前的对话记忆」把之前输出的文件也算入。
#[tauri::command]
pub fn artifact_search(query: String) -> Vec<ArtifactSearchHit> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<ArtifactSearchHit> = Vec::new();
    for root in conversation_roots() {
        if !root.exists() {
            continue;
        }
        for w in WalkDir::new(&root).into_iter().flatten() {
            if !w.file_type().is_file() {
                continue;
            }
            let p = w.path();
            // 仅 conversations/<id>/outputs/** 下的文件
            let rel = match p.strip_prefix(&root) {
                Ok(r) => r,
                Err(_) => continue,
            };
            let comps: Vec<String> = rel
                .components()
                .filter_map(|c| c.as_os_str().to_str().map(|s| s.to_string()))
                .collect();
            // 期望 [<id>, "outputs", ...]
            if comps.len() < 3 || comps[1] != "outputs" {
                continue;
            }
            let conversation_id = comps[0].clone();
            let name = p
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if name.starts_with('.') {
                continue;
            }
            let ext = p
                .extension()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let kind = classify_ext(&ext);
            let meta = match w.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);

            let mut score = 0;
            let mut snippet = String::new();
            if name.to_lowercase().contains(&q) {
                score += 10;
            }
            // 文本类才读正文匹配 (限大小, 防卡)
            if matches!(kind, "text" | "markdown" | "html" | "svg") && meta.len() < 512 * 1024 {
                if let Ok(body) = std::fs::read_to_string(p) {
                    let lower = body.to_lowercase();
                    if let Some(pos) = lower.find(&q) {
                        score += 2;
                        let start = body[..pos].char_indices().rev().take(40).last().map(|(i, _)| i).unwrap_or(0);
                        let end = (pos + q.len() + 60).min(body.len());
                        let mut e = end;
                        while e < body.len() && !body.is_char_boundary(e) {
                            e += 1;
                        }
                        snippet = body[start..e].replace('\n', " ").trim().to_string();
                    }
                }
            }
            if score > 0 {
                hits.push(ArtifactSearchHit {
                    path: p.to_string_lossy().replace('\\', "/"),
                    name,
                    kind: kind.to_string(),
                    conversation_id,
                    snippet,
                    modified,
                    score,
                });
            }
        }
    }
    hits.sort_by(|a, b| b.score.cmp(&a.score).then(b.modified.cmp(&a.modified)));
    hits.truncate(50);
    hits
}

// ───────────────────────── 对话附件 (拖拽上传) ─────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachedFile {
    pub name: String,
    /// 复制后在会话 uploads 目录里的绝对路径 (正斜杠)
    pub path: String,
    /// text | image | pdf | office | binary —— 前端选图标用
    pub kind: String,
    pub size: u64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 对话拖拽上传:把文件复制进「会话 uploads 目录」,返回附件清单。
/// 与「知识库上传」是两条不同的路径 —— 这里只把文件挂到当前对话,
/// 前端发送时把这些绝对路径写进 prompt,claude 用 Read 工具按需读取。
#[tauri::command]
pub fn chat_attach_files(
    conversation_id: Option<String>,
    paths: Vec<String>,
) -> Vec<AttachedFile> {
    const MAX: usize = 50;
    let dir = conversation_dir(conversation_id.as_deref()).join("uploads");
    let _ = std::fs::create_dir_all(&dir);

    let mut out = Vec::new();
    for p in paths.iter().take(MAX) {
        let src = PathBuf::from(p);
        if src.is_dir() {
            // 目录:浅层展开其中的文件
            if let Ok(rd) = std::fs::read_dir(&src) {
                for e in rd.flatten() {
                    let ep = e.path();
                    if ep.is_file() && out.len() < MAX {
                        push_attach(&dir, &ep, &mut out);
                    }
                }
            }
            continue;
        }
        if !src.is_file() {
            out.push(AttachedFile {
                name: file_name_of(&src),
                path: String::new(),
                kind: "binary".into(),
                size: 0,
                ok: false,
                error: Some("文件不存在".into()),
            });
            continue;
        }
        push_attach(&dir, &src, &mut out);
    }
    out
}

fn file_name_of(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| p.to_string_lossy().to_string())
}

fn push_attach(dir: &Path, src: &Path, out: &mut Vec<AttachedFile>) {
    let name = file_name_of(src);
    let size = std::fs::metadata(src).map(|m| m.len()).unwrap_or(0);
    let dst = unique_upload_path(dir, &name);
    match std::fs::copy(src, &dst) {
        Ok(_) => out.push(AttachedFile {
            name,
            path: dst.to_string_lossy().replace('\\', "/"),
            kind: attach_kind(src).into(),
            size,
            ok: true,
            error: None,
        }),
        Err(e) => out.push(AttachedFile {
            name,
            path: String::new(),
            kind: "binary".into(),
            size,
            ok: false,
            error: Some(e.to_string()),
        }),
    }
}

fn unique_upload_path(dir: &Path, fname: &str) -> PathBuf {
    let first = dir.join(fname);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match fname.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (fname.to_string(), String::new()),
    };
    for n in 2..10_000 {
        let cand = dir.join(format!("{stem} ({n}){ext}"));
        if !cand.exists() {
            return cand;
        }
    }
    first
}

fn attach_kind(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "avif" | "svg" => "image",
        "pdf" => "pdf",
        "docx" | "doc" | "pptx" | "ppt" | "xlsx" | "xls" | "ods" | "odt" | "odp" => "office",
        "txt" | "md" | "markdown" | "csv" | "tsv" | "json" | "yaml" | "yml" | "xml" | "html"
        | "htm" | "log" | "rs" | "js" | "ts" | "py" | "go" | "java" | "c" | "cpp" | "css"
        | "vue" | "sh" | "toml" | "ini" => "text",
        _ => "binary",
    }
}
