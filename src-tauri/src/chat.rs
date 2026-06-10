//! 板块 ① 对话核心 — MVP v0.2 (stderr 透传 + 项目/对话历史)
//!
//! 设计依据: PRD-v6 §7
//! - chat_send: 组装 prompt(KB 注入) -> spawn claude CLI -> emit chat:stream
//! - 同时读 stdout + stderr (单独线程), stderr 转 error 事件
//! - child.wait 完成后, 检查 exit code, 非 0 时 emit error
//! - 沙箱模式预检容器是否在运行, 不在时直接返回错误
//! - 整合 conv 模块, 自动写 user/assistant 消息

use crate::claude_md;
use crate::convert;
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
#[cfg(feature = "desktop")]
use tauri::{AppHandle, Emitter};
#[cfg(not(feature = "desktop"))]
use crate::host::AppHandle;
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
/// - with_task=true (动态编排): 额外放行 `Task` —— 否则 headless(stdin=null)下编排器
///   想扇出子代理会卡在权限确认上, 多智能体并行就跑不起来。
fn allowed_tools_for(perm: &str, with_task: bool) -> String {
    let mut tools = if perm == "plan" {
        DEFAULT_WEB_TOOLS.to_string()
    } else {
        format!("{},{}", DEFAULT_WEB_TOOLS, LOCAL_WORK_TOOLS)
    };
    if with_task {
        tools.push_str(",Task");
    }
    tools
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
    /// 「动态编排」：把本轮当成多智能体编排——编排器拆成 N 个独立子任务，
    /// 用 Task 子代理并行扇出，每条流水线 实现→对抗式校验→修复，最后汇总。
    #[serde(default)]
    pub dynamic_workflow: bool,
    /// 「知识库严格搜索」：打开时才把 KB 结构化 wiki + 双链地图注入上下文。
    /// 默认 false 以节省 token，日常任务不注入大段 KB 导航。
    #[serde(default)]
    pub use_kb: bool,
    /// 「分批长任务」：把一次超长生成(如 60 页 PPT)拆成多轮有界批次。
    /// 注入分批构建协议——先产 `polaris.build.json` 计划清单, 每轮只建 ≤batch_size 个
    /// pending 单元并回写状态; 由前端编排循环驱动多轮、断线从清单下一个 pending 续跑。
    /// 缘由: 单轮把 60 页全吐完会让流式连接跑太久被掐(socket closed → exit 1), 分批让
    /// 每轮输出有界、context 不随页数膨胀、崩了也不丢已落盘的批次。
    #[serde(default)]
    pub batch_build: bool,
    /// 每批最多构建几个单元(页/章/文件)。None 时用默认值。
    #[serde(default)]
    pub batch_size: Option<usize>,
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

/// 单轮 assistant 文本落库缓冲上限 (字节): 防 claude 异常死循环狂打输出把内存撑爆。
/// 超限后实时 delta 仍照常 emit 给前端, 只是不再增长落库缓冲, 末尾加一次截断标记。
const MAX_ASSISTANT_BYTES: usize = 8 * 1024 * 1024;
/// 单轮 stderr 累积上限 (字节)。
const MAX_STDERR_BYTES: usize = 1024 * 1024;

fn next_req_id() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let c = REQ_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("req-{:x}-{:x}", ts, c)
}

// ───────────────────────── Commands ──────────────────────

#[cfg_attr(feature = "desktop", tauri::command)]
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
    let cm_ctx = claude_md::render_for_project(current_project_id.as_deref(), &args.prompt, args.use_kb);

    let mut final_prompt = String::new();

    // 0. KB-first 顶层指令 (写死, 优先级最高)
    // 任何后续指令(目标模式 / 请教毛主席 / 动态编排 / 风格约定)都不能凌驾它之上;
    // 它是产品立场: 知识库是真相源, 模型必须先用 KB 取证再作答。
    // 这条指令会出现在 prompt 最前面, 离用户问题最远——但因 Claude 的"system 指令优先"特性,
    // 它仍然约束着整轮回复。配合 `claude_md::render_for_project` 注入的结构化 wiki,
    // 模型就能沿 Read/Glob/Grep + [[双链]] 自主取证。
    final_prompt.push_str(&kb_first_directive());
    final_prompt.push_str("\n\n---\n\n");

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

    // 1.5 回答风格约定 (Codex 式扁平) — 框定所有对话回复: 扁平/结构化/砍废话
    final_prompt.push_str(&reply_style_directive());
    final_prompt.push_str("\n\n---\n\n");

    // 2. 输出文件约定 (Polaris) — 让成品文件落到产物目录, 侧边栏即可预览
    final_prompt.push_str(&output_convention(&art_dir));
    final_prompt.push_str("\n\n---\n\n");

    // 2.1 可运行项目约定 (板块⑮) — 要跑起来的应用(尤其前后端)打包成带运行清单的项目文件夹,
    //     用户在右侧点「运行」即一键启动前后端并内嵌预览, 不必再拖文件、再说「打开项目」。
    final_prompt.push_str(&project_convention(&art_dir));
    final_prompt.push_str("\n\n---\n\n");

    // 2.15 分批长任务: 超长生成(60 页 PPT 这类)拆成有界批次, 每轮只建 ≤K 个 pending 单元,
    //      用 polaris.build.json 清单做 checkpoint, 断线从下一个 pending 续跑 ——
    //      规避单轮输出过长把流式连接拖死(socket closed → 进程坏死)。
    if args.batch_build {
        let bs = args.batch_size.unwrap_or(8).clamp(1, 50);
        final_prompt.push_str(&batch_build_directive(&art_dir, bs));
        final_prompt.push_str("\n\n---\n\n");
    }

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

    // 2.65 动态编排: 把本轮当成多智能体编排, 用 Task 子代理并行扇出, 每条流水线
    //      实现 -> 对抗式校验 -> 修复, 最后汇总(详见 dynamic_workflow_directive)。
    if args.dynamic_workflow {
        final_prompt.push_str(&dynamic_workflow_directive());
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

    // 3. CLAUDE.md 上下文 (KB 地图 + 项目人格)
    if !cm_ctx.is_empty() {
        final_prompt.push_str(&cm_ctx);
        final_prompt.push_str("\n\n---\n\n");
    }

    // 3.5 跨对话产物地图: 本项目其它对话生成过、仍在磁盘上的文件(绝对路径)。
    //     让模型可直接 Read「上次那个文件」, 用户不用重新拖拽。当前对话排除(它的文件
    //     已在下面的对话历史里出现)。
    if let Some(pid) = current_project_id.as_deref() {
        let amap = project_artifacts_block(pid, args.conversation_id.as_deref(), ARTIFACT_MAP_BUDGET);
        if !amap.is_empty() {
            final_prompt.push_str(&amap);
            final_prompt.push_str("\n\n---\n\n");
        }
    }

    // 3.6 对话历史: 本对话最近若干轮原文(预算封顶), 让同一对话能接上文 ——
    //     此前每轮都是无状态新进程, claude 看不到上一句, 这里补上。
    if let Some(cid) = args.conversation_id.as_deref() {
        let hist = history_block(cid, HISTORY_CTX_BUDGET);
        if !hist.is_empty() {
            final_prompt.push_str(&hist);
            final_prompt.push_str("\n\n---\n\n");
        }
    }

    // 4. 用户原始问题
    final_prompt.push_str("## 用户问题\n\n");
    final_prompt.push_str(&args.prompt);

    let perm = args.permission_mode.cli_value();
    let conv_id_opt = args.conversation_id.clone();

    // 上下文预算自检: 估算本轮注入的总 token 并 emit 给前端(kind=meta) —— 分批编排据此
    // 自适应批量大小(input 越大则每批越小), 也让「自动检测上下文优化」有据可依。
    let est_tokens = estimate_tokens(&final_prompt);
    emit_event(
        &app,
        ChatStreamEvent {
            req_id: req_id.clone(),
            kind: "meta".into(),
            text: Some(est_tokens.to_string()),
            tool: None,
            conversation_id: conv_id_opt.clone(),
        },
    );

    // 默认走宿主机执行（沙箱可选，但默认关闭）；动态编排时放行 Task 子代理
    let mut child = spawn_on_host(&final_prompt, perm, &art_dir, args.dynamic_workflow)?;

    // prompt 经 stdin 喂给 claude (而非命令行参数): 大 prompt 不会撞 Windows 命令行
    // 长度上限, 也不会因 prompt 以 `-` 开头被当成 flag。spawn 后立刻写 + drop, claude 读到 EOF 就开始处理。
    // stdin 写放独立线程: 大 prompt 超过 OS 管道缓冲(~64KB)且 claude 尚未开始读时,
    // write_all 会阻塞 —— 放后台线程就不会卡住本 async 命令的执行线程(影响其它并发命令)。
    // 写完线程结束时 drop(stdin) 关管道 → claude 读到 EOF 开工。失败不致命(claude 有 fallback)。
    if let Some(mut stdin) = child.stdin.take() {
        let payload = std::mem::take(&mut final_prompt);
        std::thread::spawn(move || {
            use std::io::Write;
            let _ = stdin.write_all(payload.as_bytes());
        });
    }

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "claude 子进程没有 stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "claude 子进程没有 stderr".to_string())?;

    CHILDREN.lock().insert(req_id.clone(), child);

    // 「最近一次活动」时间戳: stdout/stderr 每产出一行就刷新(见下面两个 reader 线程)。
    // 看门狗据此判「空闲挂死」而非「绝对超时」—— 正在活跃流式输出的长任务(批量 PPT/
    // 长脚本等)不会被误杀, 只有真的长时间零输出(claude 子代理对 `/` 无界扫描卡住)才判挂死。
    let last_activity = Arc::new(Mutex::new(std::time::Instant::now()));

    // 看门狗(容器/服务端稳健性): 个别 prompt 会让 claude 触发子代理(`claude --print`,
    // 容器内其 cwd 落在 `/`)对文件系统做无界扫描而长时间不返回 —— 既拖死本轮, 又占住
    // OAuth 订阅的并发槽拖垮后续消息。**连续空闲**超过阈值(而非一启动就倒计时)才杀掉整个
    // 进程组(claude + 子代理), claude stdout 随之关闭 → 下面 reader 线程照常 emit error+done,
    // 系统自愈、释放并发槽。由 POLARIS_CHAT_TIMEOUT_SECS 控制: 桌面默认 0=不启用, 容器 180。
    let watchdog_timeout = std::env::var("POLARIS_CHAT_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    if watchdog_timeout > 0 {
        let wd_req = req_id.clone();
        let wd_activity = last_activity.clone();
        std::thread::spawn(move || {
            let timeout = std::time::Duration::from_secs(watchdog_timeout);
            // 检查节拍: 每 tick 醒来看一次是否空闲超时; tick 不超过 5s, 也不超过 timeout 本身。
            let tick = std::cmp::min(timeout, std::time::Duration::from_secs(5));
            loop {
                std::thread::sleep(tick);
                // 先读空闲时长(不与 CHILDREN 锁同时持有, 避免锁序问题), 再持锁取 child:
                // 取到 Some 才证明仍是本 req 的活进程(防 PID 复用误杀); 取到 None = 已正常
                // 结束被 stdout 线程 remove → 退出看门狗。
                let idle = wd_activity.lock().elapsed();
                let g = CHILDREN.lock();
                let Some(c) = g.get(&wd_req) else { break };
                if idle >= timeout {
                    kill_tree(c.id()); // 持锁内杀进程组: 一并带走 cwd=/ 的子代理
                    break;
                }
                // 否则仍在活跃推进, 不误杀, 继续看门(锁随作用域结束释放)。
            }
        });
    }

    // stderr 读线程: 任何 stderr 行都 emit 为 error 事件; 累积起来给 wait 用
    let app_err = app.clone();
    let req_err = req_id.clone();
    let conv_id_err = conv_id_opt.clone();
    let stderr_buf = Arc::new(Mutex::new(String::new()));
    let stderr_buf_clone = stderr_buf.clone();
    let act_err = last_activity.clone();
    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            let Ok(line) = line else { continue };
            if line.trim().is_empty() {
                continue;
            }
            *act_err.lock() = std::time::Instant::now(); // 刷新活动: 有产出就不算挂死
            {
                // 单次加锁 + 封顶: 异常时 stderr 也可能狂刷, 不让它无界累积。
                let mut buf = stderr_buf_clone.lock();
                if buf.len() < MAX_STDERR_BYTES {
                    buf.push_str(&line);
                    buf.push('\n');
                }
            }
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
    let act_out = last_activity.clone();
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
        // 落库缓冲封顶: claude 若异常死循环狂打输出, 不让 assistant_text 无界增长撑爆内存。
        // 超限后改写入可丢弃的 scrap (实时 delta 仍照常 emit, 前端实时可见), 不再增长落库缓冲。
        let mut scrap = String::new();
        let mut capped = false;
        for line in reader.lines() {
            let Ok(line) = line else { continue };
            if line.trim().is_empty() {
                continue;
            }
            *act_out.lock() = std::time::Instant::now(); // 刷新活动: 流式产出即视为推进, 防误杀
            let target = if capped { &mut scrap } else { &mut assistant_text };
            match serde_json::from_str::<Value>(&line) {
                Ok(v) => handle_stream_event(
                    &app_out,
                    &req_out,
                    conv_id_thread.as_deref(),
                    &v,
                    target,
                    &mut artifacts,
                ),
                Err(_) => {
                    // 非 JSON 行: 当作 delta 直接显示 (调试友好)
                    target.push_str(&line);
                    target.push('\n');
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
            if capped {
                scrap.clear(); // scrap 只为让上面 emit 继续工作, 不能自己变成无界
            } else if assistant_text.len() > MAX_ASSISTANT_BYTES {
                assistant_text.push_str("\n\n[⚠️ 输出过长，后续内容已省略]");
                capped = true;
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

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn chat_cancel(req_id: String) -> Result<(), String> {
    if let Some(mut child) = CHILDREN.lock().remove(&req_id) {
        kill_tree(child.id()); // 先杀整树: claude 扇出的 python/node/dev server 等子孙
        let _ = child.kill(); // 再杀 claude 本体 (taskkill /T 通常已带走它, 这步兜底)
        let _ = child.wait(); // reap, 防 Unix 僵尸进程泄漏
    }
    Ok(())
}

/// 读取某会话的分批构建清单 `polaris.build.json`(分批长任务的断点/进度凭据)。
/// 前端编排循环每轮结束后读它, 算还剩几个 pending 来决定续不续、断了从哪接。
/// 不存在或解析失败返回 None(前端据此判定「还没规划」或「读不到, 当作未完成重试」)。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn chat_build_manifest(conversation_id: Option<String>) -> Option<Value> {
    let path = artifacts_dir(conversation_id.as_deref()).join("polaris.build.json");
    let txt = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str::<Value>(&txt).ok()
}

/// App 退出 (关窗 / 主动退出) 时回收所有在飞的 claude 子进程, 连同它们扇出的整棵进程树。
/// 否则用户在 claude 跑长任务 (起 dev server / Task 扇出) 时直接关 App, claude 及其子孙
/// 会变孤儿继续在后台占端口/CPU/写文件。由 lib.rs 的 RunEvent 钩子调用。
pub fn kill_all_children() {
    let mut map = CHILDREN.lock();
    for (_id, mut child) in map.drain() {
        kill_tree(child.id());
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// 按 PID kill 整个进程树。claude 在 Bash/PowerShell/Task 工具下会拉起 python/node/dev server
/// 等子进程, 只 kill claude 本体会留孤儿占着端口。与 project.rs::kill_tree 同策略。
fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("taskkill");
        cmd.args(["/PID", &pid.to_string(), "/T", "/F"]);
        no_window(&mut cmd);
        let _ = cmd.output();
    }
    #[cfg(not(windows))]
    {
        // 杀进程组 (shell -c 起的子孙); 失败再退化为 kill 单进程。
        let _ = Command::new("kill")
            .args(["-TERM", &format!("-{}", pid)])
            .output()
            .or_else(|_| Command::new("kill").arg(pid.to_string()).output());
    }
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

// Docker-in-Docker 沙箱仅桌面构建可用 (依赖 polaris_sandbox crate)；
// server(容器内)本期降级，不编译此路径。
#[cfg(feature = "desktop")]
#[allow(dead_code)]
fn spawn_in_sandbox(prompt: &str, perm: &str) -> Result<Child, String> {
    let perm_flag = format!("--permission-mode={}", perm);
    // 联网 + (非只读档位)本地读写执行, 让成品能真正产出
    let allowed = allowed_tools_for(perm, false);
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

fn spawn_on_host(prompt: &str, perm: &str, art_dir: &Path, with_task: bool) -> Result<Child, String> {
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
    args.push(allowed_tools_for(perm, with_task));
    args.push(perm_flag);
    // ⚠️ prompt 不再塞 argv 末尾 —— 走 stdin。
    // Windows CreateProcessW 的 lpCommandLine 上限 32767 字符, 你 KB 全文 + 多轮对话历史
    // 拼一起轻松爆, 直接抛 206 ERROR_FILENAME_TOO_LONG 拒 spawn (实测 33k 字符就 100% 复现)。
    // 改 stdin 后 prompt 长度无限制。kb.rs 的 spawn_in_sandbox 早就这么干了 (注释在那)。
    let _ = prompt; // 函数签名仍保留 prompt 参数, 调用方写 stdin

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
        .stdin(Stdio::piped()) // 接 prompt 用, 调用方 spawn 后 write + drop
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Windows: claude 跑 Bash 工具要靠 Git Bash。启动期 prime 通常已设好 CLAUDE_CODE_GIT_BASH_PATH,
    // 但若 Git Bash 是 app 启动后才装的, 这里兜底显式喂给子进程 —— 免得 claude 扫不到 shell。
    #[cfg(windows)]
    if std::env::var_os("CLAUDE_CODE_GIT_BASH_PATH").is_none() {
        if let Some(bash) = crate::doctor::detect_git_bash() {
            cmd.env("CLAUDE_CODE_GIT_BASH_PATH", bash);
        }
    }
    // 子进程环境净化: loopback 强制 NO_PROXY (切 Codex 时 claude 走 127.0.0.1 本地代理,
    // 系统代理会劫持回环 → 连不上) + 清 DEBUG/LD_PRELOAD。见 doctor::harden_child_env。
    crate::doctor::harden_child_env(&mut cmd);
    no_window(&mut cmd); // 隐藏式: 每次发消息不再弹出黑色终端窗口

    // Linux/容器: 让 claude 成为新进程组的组长 (setpgid)。这样 kill_tree 的
    // `kill -TERM -<pid>` 能一次带走 claude 扇出的 python/node/dev-server 整棵子孙树,
    // 不留孤儿占端口/CPU —— 对容器内长稳运行(>3h, 反复发消息)至关重要。
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    cmd.spawn().map_err(|e| {
        // 错误只在 spawn 本身失败 (e.g. exe 找不到), 不再是 prompt 太长
        format!("调起宿主机 claude CLI 失败: {}", e)
    })
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
/// `pub(crate)`: 板块⑮「可运行项目」(project.rs) 也要按同一规则定位产物目录, 去扫项目清单。
pub(crate) fn artifacts_dir(conv_id: Option<&str>) -> PathBuf {
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

// ───────────────────────── 对话记忆 (历史 + 跨对话产物地图) ─────────────────────────
//
// 设计: 此前每轮 chat_send 都是无状态新进程, claude 看不到上一句、也读不到别的对话生成的
// 文件。这里补两块, 都顺着 llmwiki「注地图不注全文」的哲学:
//   ① history_block          —— 本对话最近若干轮原文(预算封顶) → 同一对话能接上文
//   ② project_artifacts_block —— 本项目其它对话生成过、仍在磁盘上的文件(绝对路径+描述)
//                                 → 用户说「上次那个文件」时模型直接 Read, 不用重新拖拽
// 两块都从已持久化的消息派生(产物路径早已存在 assistant 正文的 ARTIFACT marker 里), 零新存储。

/// 单块历史预算(字符): 超了就丢最旧的几轮。stdin 喂 prompt, 不受命令行 32k 限制,
/// 但仍要控总 context, 故封顶。
const HISTORY_CTX_BUDGET: usize = 8000;
/// 单条消息正文上限(字符): 太长的回答只留开头, 避免一条吃掉整个预算。
const HISTORY_MSG_CAP: usize = 1500;
/// 跨对话产物地图预算(字符)。
const ARTIFACT_MAP_BUDGET: usize = 4000;

/// 按字符(非字节)截断, 中文安全; 超长加省略标记。
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{}…(略)", head)
    }
}

/// 从 assistant 正文里剥出产物清单 marker: 返回 (去掉 marker 的正文, 产物绝对路径列表)。
/// marker 形如 `<!--POLARIS_ARTIFACTS:["C:/a.html","C:/b.md"]-->`(见 ARTIFACT_MARKER_PREFIX)。
fn split_artifacts(content: &str) -> (String, Vec<String>) {
    if let Some(idx) = content.find(ARTIFACT_MARKER_PREFIX) {
        let after = &content[idx + ARTIFACT_MARKER_PREFIX.len()..];
        if let Some(end) = after.find("-->") {
            let paths: Vec<String> = serde_json::from_str(&after[..end]).unwrap_or_default();
            let clean = content[..idx].trim_end().to_string();
            return (clean, paths);
        }
    }
    (content.trim().to_string(), Vec::new())
}

/// epoch 毫秒 → "YYYY-MM-DD"(UTC, 仅供模型粗略排序「上次/之前」参考)。
/// 无依赖实现 (Howard Hinnant civil_from_days)。
fn ymd(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// ① 对话历史块: 本对话最近若干轮原文, 从最新往回累计到预算上限, 再翻回时间正序。
/// 末尾那条 user 消息是「本轮问题」(chat_send 进来时刚 append), 已单独注入 → 去掉避免重复。
fn history_block(conv_id: &str, budget: usize) -> String {
    let mut msgs = conv::get_messages(conv_id);
    if matches!(msgs.last(), Some(m) if m.role == "user") {
        msgs.pop();
    }
    if msgs.is_empty() {
        return String::new();
    }

    let mut picked: Vec<String> = Vec::new();
    let mut used = 0usize;
    for m in msgs.iter().rev() {
        let line = match m.role.as_str() {
            "user" => format!("**用户**：{}", truncate_chars(m.content.trim(), HISTORY_MSG_CAP)),
            "assistant" => {
                let (clean, files) = split_artifacts(&m.content);
                let body = truncate_chars(clean.trim(), HISTORY_MSG_CAP);
                if files.is_empty() {
                    format!("**助手**：{}", body)
                } else {
                    format!("**助手**：{}\n〔本轮生成文件：{}〕", body, files.join(" · "))
                }
            }
            _ => continue, // tool 等其它角色不进历史
        };
        let cost = line.chars().count() + 2;
        if used + cost > budget && !picked.is_empty() {
            break;
        }
        used += cost;
        picked.push(line);
    }
    if picked.is_empty() {
        return String::new();
    }
    picked.reverse();
    format!(
        "## 对话历史 (本对话最近若干轮, 供你接上文)\n\n\
下面是本对话之前的往返。继续作答时**默认用户在接着上文聊**, 别把已经聊过的当成全新问题重头解释。\n\n{}",
        picked.join("\n\n")
    )
}

/// ② 跨对话产物地图: 遍历本项目其它对话, 把每条带产物的 assistant 消息的文件路径,
/// 配上「前一条 user 问题」当描述, 列成一张地图。只列仍存在于磁盘的文件(去悬空), 去重, 预算封顶。
/// 排除当前对话(它的文件已在 history_block 里出现, 避免重复)。
fn project_artifacts_block(project_id: &str, exclude_conv: Option<&str>, budget: usize) -> String {
    let convs = conv::conversations_of_project(project_id); // 最近在前
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut lines: Vec<String> = Vec::new();
    let mut used = 0usize;

    'outer: for c in &convs {
        if Some(c.id.as_str()) == exclude_conv {
            continue;
        }
        // 正序遍历记住「最近的 user 问题」, 给随后的产物当描述
        let mut last_user: Option<String> = None;
        let mut entries: Vec<(String, String)> = Vec::new();
        for m in conv::get_messages(&c.id) {
            match m.role.as_str() {
                "user" => last_user = Some(m.content.trim().to_string()),
                "assistant" => {
                    let (_clean, files) = split_artifacts(&m.content);
                    let desc = last_user.clone().unwrap_or_default();
                    for f in files {
                        entries.push((f, desc.clone()));
                    }
                }
                _ => {}
            }
        }
        // 该对话内新产物在前
        for (path, desc) in entries.into_iter().rev() {
            if seen.contains(&path) || !Path::new(&path).exists() {
                continue;
            }
            seen.insert(path.clone());
            let desc_short = truncate_chars(desc.trim(), 60);
            let date = ymd(c.updated_at);
            let line = if desc_short.is_empty() {
                format!("- `{}` — 来自对话「{}」· {}", path, c.title, date)
            } else {
                format!("- `{}` — 来自对话「{}」({}) · 当时请求: {}", path, c.title, date, desc_short)
            };
            let cost = line.chars().count() + 1;
            if used + cost > budget && !lines.is_empty() {
                break 'outer;
            }
            used += cost;
            lines.push(line);
        }
    }
    if lines.is_empty() {
        return String::new();
    }
    format!(
        "## 本项目已生成的文件 (产物地图)\n\n\
下面是**本项目其它对话**里生成过、现在仍在磁盘上的成品文件(绝对路径)。\n\
当用户说「上次那个 / 之前生成的 X / 接着改那个文件」时, **直接用 `Read` 打开对应路径即可, \
不需要用户重新拖拽文件**; 路径对不上再问用户。\n\n{}",
        lines.join("\n")
    )
}

/// KB-first 顶层指令 (写死) —— 这一条优先级最高, 任何后续指令都不能凌驾。
///
/// 设计: 模型每一轮回答前, 必须先按本指令 4 步沿双链在知识库里「调查取证」;
/// 取不到证据(且问题属于事实/可考证领域)时, 显式说「资料不足」, 不准凭预训练兜底。
/// 配合 `claude_md::render_for_project` 注入的结构化 wiki + 双链地图使用。
///
/// 结构遵循通用 llmwiki (Karpathy 式): 三层 `raw/ output/ wiki/`, 扁平 `wiki/*.md`,
/// 入口 `wiki/index.md`, 双链写 wiki 根相对名/title, 引用走脚注 —— 不含任何
/// 项目特定结构 (无 SQL/位次工具、无 概念/实体 子目录约定)。
///
/// 适用场景: 所有对话(包括普通问答、请教毛主席、目标模式、动态编排、偶像对话)——
/// 这是产品立场, 不让用户开关。
fn kb_first_directive() -> String {
    "## ⚡ 知识库优先 (KB-First · 写死, 不可关闭)\n\n\
你的工作目录下挂着一棵**结构化维基知识库** (PolarisKB), 信奉 Karpathy \
「结构化 wiki + 长上下文 > 平铺文档 + 向量检索」, 分三层: `raw/`(只读原始层)、\
`output/`(生成的文章/Lint 报告)、`wiki/`(知识层, 扁平 `wiki/*.md`)。导航入口是 \
`wiki/index.md`。它就在你的工作目录下, 已随本轮以长上下文方式预先注入。\n\n\
**本轮回答问题之前, 必须按下述 4 步沿双链在知识库里调查取证, 不准凭空作答:**\n\n\
1. **定位 (Locate)** —— 先用 `Glob` 找出与问题最相关的页面 (如 `wiki/*.md`、`raw/**`), \
别一上来就 `Read` 全库。\n\
2. **命中 (Grep)** —— 用 `Grep` 在定位到的范围里搜关键词, 拿到候选页的精确列表 \
(标题/正文里出现过目标概念)。\n\
3. **取证 (Read)** —— 对每个候选页 `Read` 完整正文, **不要切片, 整页读**。\n\
4. **沿双链 (Trace)** —— 顺着页面里的 `[[双链]]` 续读 (双链只写 wiki 根相对名或 \
frontmatter 的 title, 如 `[[index]]`、`[[CLAUDE]]`), 把相关页面串成证据链。\n\n\
**反幻想护栏 (强制, 不可省):**\n\n\
- 命中库内容时**必须以脚注标注来源**: 正文处 `[^1]`, 文末 `[^1]: [[file-name]]`; \
**模型自己脑补出来的话术不算证据**。\n\
- 知识库查不到、且问题属于事实/可考证领域 → 用 `💡` 标明这是推断/仿写, \
**明确说缺什么**, 严禁用预训练知识冒充检索结果, 也不要伪造引文; 通用闲聊/生活常识类除外。\n\n\
**与其它指令的优先级 (重要):**\n\n\
- 本指令的优先级**高于**后续所有指令 (回答风格、目标模式、请教毛主席、动态编排、偶像对话)。\
任何指令与本条冲突时, 以本条为准。\n\
- 本指令**不限制**你的判断与表达自由, 只约束你「事实必须可溯源、不能凭印象胡诌」。\n\n\
> 入口: 知识库根目录在工作目录下的 `PolarisKB/`。先看 `wiki/index.md` 找到主导航, \
再按上面 4 步沿 `[[双链]]` 用 Read/Glob/Grep 取证 —— **不要等别人把答案喂你**。\
这里不存在也不需要 kb_search 之类的召回工具。"
        .to_string()
}

/// 注入给 claude 的「回答风格约定」—— Codex 式扁平回答, 砍废话, 只留信号。
/// 框定所有对话回复(普通问答 / 分析 / 计划), 不影响成品文件本身的丰富度。
fn reply_style_directive() -> String {
    "## 回答风格约定 (Polaris · Codex 式扁平)\n\n\
你的对话回复必须扁平、结构化、切中要点 —— 学卡帕西/「山顶洞人」式只留信号:\n\n\
1. **先给结论**。第一句就是答案或要做的事, 不要开场白、铺垫、寒暄。\n\
2. **砍掉废话**。不写「让我来…」「总的来说…」「希望这能帮到你」这类过渡和总结句。\n\
3. **能结构化就结构化**。用短列表、表格、代码块承载信息; 避免大段散文。\n\
4. **短**。同样的信息用更少的字; 不重复用户的问题, 不解释你将要做什么。\n\
5. **诚实**。不确定就说不确定, 别用热情的措辞掩盖。\n\n\
例外: 用户明确要求详细展开、或需要分步教学时, 可适度展开 —— 但仍然先给结论、保持结构化。"
        .to_string()
}

/// 注入给 claude 的「输出文件约定」, 引导成品落到产物目录
fn output_convention(art_dir: &Path) -> String {
    let dir = art_dir.to_string_lossy().replace('\\', "/");
    format!(
        "## 输出文件约定 (Polaris)\n\n\
当你生成任何可供用户**查看或下载的成品文件**(HTML 网页 / 数据可视化 / 报告 / Markdown / 图片 / CSV / PDF 等)时,请遵守:\n\n\
1. 把成品文件保存到这个已授权可写的目录(用绝对路径):\n   `{dir}`\n\
2. 网页类成品请优先生成**单文件、自包含的 HTML**(把 CSS/JS 内联进去),以便在侧边栏直接预览。\n\
3. 在回答末尾**用绝对路径列出你生成/修改的成品文件**(不要只写文件名),例如:\n   `已生成: {dir}/report.html`\n   \
这样路径会被记进本项目的「产物地图」,下次对话里用户说「上次那个文件」时,模型能直接 Read,不必重新拖拽。\n\n\
普通问答无需创建文件。",
        dir = dir
    )
}

/// 可运行项目约定 (Polaris · 板块⑮) —— 这是本轮目标的核心。
///
/// 当用户要的是一个**能跑起来的应用/项目**(尤其同时有前端 + 后端, 或需要 dev server、
/// 多文件协作运行)时, **不要把文件散落一地**, 而是打包成 **一个自带运行清单的项目文件夹**,
/// 让用户在右侧抽屉点一下「运行」就能一键启动整套前后端、并内嵌预览 —— 无需用户再拖文件、
/// 也无需再说一句「打开这个项目」。
fn project_convention(art_dir: &Path) -> String {
    let dir = art_dir.to_string_lossy().replace('\\', "/");
    format!(
        "## 可运行项目约定 (Polaris · 一键启动) —— 关键\n\n\
当用户要的是一个**能运行起来的应用 / 项目**(典型: 同时有前端和后端、或要起 dev server、\
或多个文件要一起跑才能体验), 请**严格**这样做, **不要把前后端文件散落成一堆零散文件**:\n\n\
1. **整个项目放进一个文件夹**(用一个简短英文 slug 命名), 就在这个可写目录下(用绝对路径):\n   `{dir}/<项目slug>/`\n\
   前端、后端各自一个子目录(如 `web/`、`server/`), 别把前后端揉在一起、也别散到外面。\n\
2. 在**项目文件夹根**写一份运行清单 `polaris.project.json`, 声明怎么装依赖、怎么起、端口、预览地址。格式:\n\
```json\n\
{{\n\
  \"name\": \"待办清单\",\n\
  \"services\": [\n\
    {{ \"name\": \"backend\",  \"dir\": \"server\", \"install\": \"npm install\", \"run\": \"node index.js\", \"port\": 3001 }},\n\
    {{ \"name\": \"frontend\", \"dir\": \"web\",    \"install\": \"npm install\", \"run\": \"npm run dev -- --port 5173\", \"port\": 5173 }}\n\
  ],\n\
  \"open\": \"http://localhost:5173\"\n\
}}\n\
```\n\
   - `services` 按声明顺序启动(后端在前); 每个服务 `dir` 相对项目根, `install` 仅在依赖缺失时跑, `run` 是长驻命令, `port` 用于「起来了没」探测。\n\
   - `open` 是用户内嵌预览要打开的 URL(通常是前端地址)。\n\
   - 纯前端(无后端)也可以只放一个 service; 但凡有后端, 就前后端各一个 service。\n\
3. **依赖要少、要能离线起得来**: 前端优先用 Vite 这类零配置脚手架, 后端优先用运行时自带能力\
(Node 内置 `http`/`express`、Python 标准库)。能不引重依赖就不引, 让 `npm install` 快、\
让用户点一下就能看到东西。**前端要连后端时, 用相对路径或 `localhost:<后端端口>`**, 别写死外网地址。\n\
4. 真把文件写全、写对: `package.json`、源码、必要的静态资源都要齐, 确保 `install` + `run` 跑下来\
真能起来(端口别和清单写的不一致)。\n\
5. 回答末尾**一句话**告诉用户: 项目已打包好, 在右侧「项目」里点「运行」即可一键启动前后端并预览。\n\n\
若用户只是要一个**单页静态成品**(一张 HTML 海报 / 一份报告 / 一张图), 按上面的「输出文件约定」\
走单文件即可, **不用**套这个项目清单。只有「要跑起来的应用」才打包成项目。",
        dir = dir
    )
}

/// 粗估文本 token 数(无需 tokenizer 依赖)。ASCII 约 4 字符/token; 非 ASCII(中日韩等)
/// 按 1 token/字保守计(实际多在 0.5~1.5, 取上界让预算自检偏紧不偏松)。仅用于上下文
/// 预算自检与分批编排的自适应批量, 不求精确。
fn estimate_tokens(s: &str) -> usize {
    let mut ascii = 0usize;
    let mut wide = 0usize;
    for c in s.chars() {
        if c.is_ascii() {
            ascii += 1;
        } else {
            wide += 1;
        }
    }
    ascii / 4 + wide + 1
}

/// 分批长任务指令 (Polaris · Batch Build) —— 本轮目标的核心之一。
///
/// 把一次性的超长生成(典型: 60 页 PPT / 长文档 / 多文件项目)改成「先规划成清单, 再每轮
/// 只建有界一小批」的形态。单轮输出因此恒定有界, 流式连接不会因一口气吐几万 token 跑太久
/// 被掐死; `polaris.build.json` 清单落盘做 checkpoint, 某一轮崩了, 下一轮读清单从下一个
/// pending 单元接着干, 已建的不重做、不丢失。前端编排循环负责把多轮串起来跑到清单清空。
fn batch_build_directive(art_dir: &Path, batch_size: usize) -> String {
    let dir = art_dir.to_string_lossy().replace('\\', "/");
    format!(
        "## 分批长任务模式 (Polaris · Batch Build) —— 关键, 必须严格遵守\n\n\
本轮是一个**超长生成任务的其中一批**, **不是**要你一口气把全部产出做完。请把活儿拆成清单, \
**每轮只建一小批**, 用清单文件做断点续传。这样每轮输出有界、连接不会被拖死、崩了也能续。\n\n\
**清单文件(唯一事实源)**: `{dir}/polaris.build.json`, 结构:\n\
```json\n\
{{\n\
  \"goal\": \"用一句话复述总目标\",\n\
  \"kind\": \"ppt | doc | web | generic\",\n\
  \"batch_size\": {bs},\n\
  \"output\": \"最终产物的相对/绝对路径(单文件或目录, 如 deck.pptx 或 build_deck.py)\",\n\
  \"units\": [\n\
    {{ \"id\": \"u01\", \"title\": \"该单元(页/章/文件)简述\", \"status\": \"pending\", \"artifact\": \"\" }}\n\
  ]\n\
}}\n\
```\n\n\
**每轮的固定动作**:\n\
1. **先读清单**: 用 Read 看 `{dir}/polaris.build.json` 是否存在。\n\
2. **不存在 → 本轮是规划轮**: 把总目标拆成**全部**单元(每页/每章一个), 全部 `status:\"pending\"`, \
写出完整清单到上面那个路径。然后**接着**构建**前 {bs} 个** pending 单元(见第 4 步), 不要只规划不动手。\n\
3. **已存在 → 本轮是构建轮**: 读出清单, 找出仍为 `pending` 的单元。\n\
4. **只建这一批(≤{bs} 个)**: 按顺序取最多 **{bs}** 个 pending 单元, 认真做出每个单元的实际内容, \
**增量写入磁盘**——把每个单元的产物追加/写进 `output` 指向的文件(脚本就 Edit 追加对应代码段, \
文档就追加对应章节; **绝不**把整份产出堆在一条聊天消息里)。做完一个就把它的 `status` 改成 \
`\"done\"`、填上 `artifact` 路径, **立刻回写清单文件**。\n\
5. **本批做完即停**: 即使剩下的看着很简单, 也**不要**在这一轮继续往下做更多单元 —— 有界输出是本模式的全部意义。\n\
6. **末尾报进度**: 用一行写明 `BATCH 本轮完成 X 个; 累计 done D / 总 N; 剩余 P 个 pending`。\n\n\
**硬约束**:\n\
- 任何一轮都不得尝试超过 {bs} 个单元; 宁可多跑几轮, 不可让单轮输出过长。\n\
- 每建完一个单元就回写清单 + 落盘产物, 保证中途崩溃时进度不丢。\n\
- 最终产物始终写到这个可写目录(用绝对路径)之下: `{dir}`。\n\
- 当清单中**所有**单元都 `done` 时, 本轮额外做一次收尾(如把分段脚本跑一遍生成最终 .pptx/.pdf, \
或合并校验), 并在末尾写明 `BUILD COMPLETE: <最终产物绝对路径>`。",
        dir = dir,
        bs = batch_size
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

/// 「动态编排」指令: 把本轮当成多智能体编排(Dynamic Workflows)。
/// 思路严格对齐参考设计——编排器拆出 N 个【相互独立】的子任务, 用 Claude Code 自带的
/// `Task` 子代理【并行扇出】, 每条流水线 实现→对抗式校验→修复, 最后汇总成最终交付。
/// 不另造编排框架, 直接借 Claude Code 现成的子代理机制(这正是该架构本身的形状)。
fn dynamic_workflow_directive() -> String {
    "## 动态编排模式 (Dynamic Workflows · 多智能体)\n\n\
本轮开启**动态编排**。把你自己当作**编排器(orchestrator)**, 用 Claude Code 自带的 \
`Task` 子代理把活儿**拆开并行干**, 而不是一条道自己从头做到尾。\n\n\
**先判断该不该扇出(重要, 别浪费)**\n\
- 只有**能拆成多块、且每块做完能被独立检查**的任务才扇出(批量改写 / 多维审查 / 多角度调研 / \
逐条数据或文档处理 / 需要多方独立判断的决策)。\n\
- 若是普通问答、强顺序依赖(后一步必须等前一步结论)、或拆不开的整体任务: **不要扇出**, \
直接正常作答即可, 一句话说明「本任务无需并行编排」。\n\n\
**编排流程(扇出时)**\n\
1. **拆解**: 先把目标拆成若干**相互独立、边界不重叠**的子任务, 在对话里用一两句列出拆法(分配方案), \
让用户看清活儿是怎么分的。\n\
2. **扇出 + 限流**: 用 `Task` 工具并行派发子任务——**在同一条回复里一次发起多个 `Task` 调用**即可并发执行; \
但**每批最多 6~8 个**, 跑完再放下一批, 别一口气开几十个把额度和速率打爆。\n\
3. **每条流水线 = 实现 → 校验 → 修复**:\n\
   - **实现(implementer)**: 子代理认真完成它那一块。\n\
   - **对抗式校验(verifier, 精华所在)**: 再派一个**独立**子代理去检查, prompt 里写死「**默认这个结果有问题, 主动挑错、证伪**」; \
   光说「你看看对不对」没用。高风险的可以派 2~3 个校验各自独立投票, 多数说有问题才打回。\n\
   - **修复(fixer)**: 校验不通过就派子代理按校验意见改, 直到通过。\n\
4. **结构化交接**: 阶段之间让子代理返回**结构化结论(JSON / 明确字段)**, 别靠自然语言瞎猜对方说了啥。\n\
5. **流水线优先于齐步走(pipeline > barrier)**: 每条子任务自己跑完就继续往下, 不要等所有子任务都做完才一起进下一阶段, \
否则快的白等慢的。\n\
6. **文件隔离**: 若多个子任务会**改同一批文件**, 让它们各写各的、最后由你合并, 避免并行互相覆盖。\n\n\
**汇总收尾**\n\
- 所有子任务有结论后, **你(编排器)负责汇总**成一份连贯的最终交付, 别把一堆零散子结果直接甩给用户。\n\
- 回答末尾简要交代**分配效果**: 拆了几块、各块谁干的、校验拦下并修了哪些问题。\n\n\
**护栏**\n\
- 多智能体多阶段比单轮**贵很多**(token 是几倍到几十倍), 子任务数量按需要来, 别为拆而拆。\n\
- 子任务范围要聚焦, 边界讲清楚, 避免重叠返工。"
        .to_string()
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

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn artifact_read(path: String) -> Result<ArtifactPayload, String> {
    let p = ensure_artifact_path(&path)?;
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
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn artifact_open_external(path: String) -> Result<(), String> {
    // 护栏 + 规范化: 只允许打开 App 管理目录内的文件, 且用解析后的绝对路径喂给系统命令
    let path = ensure_artifact_path(&path)?.to_string_lossy().to_string();
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
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn artifact_reveal(path: String) -> Result<(), String> {
    // 护栏 + 规范化: 只允许定位 App 管理目录内的文件
    let path = ensure_artifact_path(&path)?.to_string_lossy().to_string();
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

/// 把编辑后的文本写回一个**已存在**的产物文件 (供「成品编辑器」保存 HTML / 网页 deck)。
/// 护栏: 复用 ensure_artifact_path —— 路径必须已存在且落在 App 管理目录内, 防越界写入。
/// 仅允许文本类后缀, 防止误把二进制 / 可执行覆盖掉。原子写 (先写临时文件再 rename)。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn artifact_write(path: String, content: String) -> Result<(), String> {
    let p = ensure_artifact_path(&path)?;
    if !p.is_file() {
        return Err("目标不是文件".into());
    }
    let ext = p
        .extension()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let editable = matches!(
        ext.as_str(),
        "html" | "htm" | "svg" | "md" | "markdown" | "txt" | "json" | "csv" | "css" | "js"
    );
    if !editable {
        return Err(format!("该文件类型不支持编辑保存: .{ext}"));
    }
    const MAX: usize = 16 * 1024 * 1024;
    if content.len() > MAX {
        return Err("内容过大, 拒绝保存 (>16MB)".into());
    }
    // 原子写: 同目录临时文件 → rename, 避免写一半损坏原文件。
    let parent = p.parent().ok_or("无法定位父目录")?;
    let tmp = parent.join(format!(
        ".{}.polaris-tmp",
        p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
    ));
    std::fs::write(&tmp, content.as_bytes()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })?;
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
#[cfg_attr(feature = "desktop", tauri::command)]
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

/// 产物命令 (read/open/reveal) 允许访问的根目录集合 (已规范化)。
/// = `~/Polaris` (含 data/artifacts、projects) + KB root (含 conversations 与 KB 资料)。
/// 这些是 App 自己产出/管理文件的地方; 命令传入的路径 canonicalize 后必须落在其一之内。
fn allowed_open_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(u) = UserDirs::new() {
        roots.push(u.home_dir().join("Polaris"));
    }
    let kb_root = PathBuf::from(kb::kb_root());
    if !kb_root.as_os_str().is_empty() {
        roots.push(kb_root);
    }
    roots
        .into_iter()
        .filter_map(|r| r.canonicalize().ok())
        .collect()
}

/// 产物访问护栏: 把前端传入的路径 canonicalize 后, 校验其落在某个允许根之内。
/// 挡前端 (或被构造的会话内容) 用任意系统路径去读取 / 用默认程序打开 / 资源管理器
/// 定位库外文件 (e.g. `C:\Windows\...`、`../../` 穿越)。返回规范化后的绝对路径。
fn ensure_artifact_path(path: &str) -> Result<PathBuf, String> {
    let canon = PathBuf::from(path)
        .canonicalize()
        .map_err(|_| format!("文件不存在或无法访问: {path}"))?;
    let roots = allowed_open_roots();
    if roots.iter().any(|r| kb::path_contains(r, &canon)) {
        Ok(canon)
    } else {
        Err("路径越界, 拒绝访问".into())
    }
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
#[cfg_attr(feature = "desktop", tauri::command)]
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
#[cfg_attr(feature = "desktop", tauri::command)]
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
        Ok(_) => {
            // PDF / Office 文件: Claude Read 工具读不了二进制, 先提取文本成 .md,
            // 只把 .md 路径传给 Claude (原文件仍留 uploads 目录供用户自行查看)。
            let ext = src
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            let convertible = matches!(
                ext.as_str(),
                "pdf" | "docx" | "doc" | "xlsx" | "xls" | "xlsm"
                    | "xlsb" | "pptx" | "ppt" | "ods" | "odt" | "odp"
            );
            if convertible {
                match convert::convert_to_markdown(src) {
                    Ok(Some(text)) => {
                        let stem = src
                            .file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| name.clone());
                        let md_name = format!("{}.extracted.md", stem);
                        let md_dst = unique_upload_path(dir, &md_name);
                        if std::fs::write(&md_dst, text.as_bytes()).is_ok() {
                            out.push(AttachedFile {
                                name: md_name,
                                path: md_dst.to_string_lossy().replace('\\', "/"),
                                kind: "text".into(),
                                size: text.len() as u64,
                                ok: true,
                                error: None,
                            });
                            return;
                        }
                        // write 失败 → 回退到原文件(带错误)
                        out.push(AttachedFile {
                            name,
                            path: String::new(),
                            kind: attach_kind(src).into(),
                            size,
                            ok: false,
                            error: Some("PDF/Office 文本提取成功但写入失败".into()),
                        });
                        return;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        out.push(AttachedFile {
                            name,
                            path: String::new(),
                            kind: attach_kind(src).into(),
                            size,
                            ok: false,
                            error: Some(format!("文本提取失败: {e}")),
                        });
                        return;
                    }
                }
            }
            // 图片 / 纯文本 / 无需转换的二进制 → 原样返回
            out.push(AttachedFile {
                name,
                path: dst.to_string_lossy().replace('\\', "/"),
                kind: attach_kind(src).into(),
                size,
                ok: true,
                error: None,
            });
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_artifacts_parses_marker_and_strips_body() {
        let content = "已生成报告。\n\n<!--POLARIS_ARTIFACTS:[\"D:/a/r.html\",\"D:/a/r.md\"]-->";
        let (clean, paths) = split_artifacts(content);
        assert_eq!(clean, "已生成报告。");
        assert_eq!(paths, vec!["D:/a/r.html".to_string(), "D:/a/r.md".to_string()]);
    }

    #[test]
    fn split_artifacts_no_marker_returns_trimmed_body() {
        let (clean, paths) = split_artifacts("  普通回答  ");
        assert_eq!(clean, "普通回答");
        assert!(paths.is_empty());
    }

    #[test]
    fn split_artifacts_malformed_marker_is_safe() {
        // 有前缀但没有闭合 --> : 不应 panic, 当作无产物处理
        let (clean, paths) = split_artifacts("x<!--POLARIS_ARTIFACTS:[\"a\"");
        assert!(paths.is_empty());
        assert!(clean.contains("POLARIS_ARTIFACTS"));
    }

    #[test]
    fn truncate_chars_is_char_safe_for_cjk() {
        assert_eq!(truncate_chars("中文", 5), "中文");
        let t = truncate_chars("一二三四五六", 3);
        assert!(t.starts_with("一二三"));
        assert!(t.ends_with("(略)"));
    }

    #[test]
    fn ymd_converts_known_epochs() {
        assert_eq!(ymd(0), "1970-01-01");
        // 2021-01-01T00:00:00Z = 1609459200000 ms
        assert_eq!(ymd(1_609_459_200_000), "2021-01-01");
    }
}
