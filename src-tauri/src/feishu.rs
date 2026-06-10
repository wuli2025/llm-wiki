//! 板块⑭ 飞书网关 — 阶段 A（配置 + 鉴权 + 连接测试 + 去重/权限/ReplyGuard 纯函数）
//!
//! 思想来源: WeSight 的「WebSocket 长连接 + 去重 + 权限 + ReplyGuard」链路。
//! Polaris 用 Rust 自研、**不抄其 TS 代码**。本文件先落地不依赖真实长连接即可验证的部分:
//! - 凭证配置存储（App ID/Secret/domain/策略）
//! - tenant_access_token 获取 + 机器人信息（连接测试）
//! - 去重环 / 权限判定 / ReplyGuard —— 均为纯函数并带单测
//!
//! 阶段 B（WebSocket 长连接收事件 → 跑对话 → 回发）需真实飞书 app 凭证联调，单列后续 PR。

use directories::UserDirs;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
#[cfg(feature = "desktop")]
use tauri::{AppHandle, Emitter};
#[cfg(not(feature = "desktop"))]
use crate::host::AppHandle;

// ───────────────────────── 配置 ─────────────────────────

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct FeishuConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub app_id: String,
    #[serde(default)]
    pub app_secret: String,
    /// "feishu"(国内) | "lark"(国际)
    #[serde(default = "default_domain")]
    pub domain: String,
    /// 私聊策略: "open" | "allowlist" | "disabled"
    #[serde(default = "default_dm_policy")]
    pub dm_policy: String,
    /// 群聊是否必须 @机器人才响应
    #[serde(default = "default_true")]
    pub group_require_mention: bool,
    /// 白名单（open_id 列表），dm_policy=allowlist 时生效
    #[serde(default)]
    pub allow_from: Vec<String>,
    /// App 启动时自动开启网关（开机自动上线）
    #[serde(default)]
    pub auto_start: bool,
}
fn default_domain() -> String {
    "feishu".into()
}
fn default_dm_policy() -> String {
    "open".into()
}
fn default_true() -> bool {
    true
}

impl FeishuConfig {
    fn base_url(&self) -> &'static str {
        if self.domain == "lark" {
            "https://open.larksuite.com"
        } else {
            "https://open.feishu.cn"
        }
    }
}

fn config_path() -> Option<PathBuf> {
    UserDirs::new().map(|u| {
        u.home_dir()
            .join("Polaris")
            .join("data")
            .join("feishu.json")
    })
}
fn read_config() -> FeishuConfig {
    config_path()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}
fn write_config(cfg: &FeishuConfig) {
    if let Some(p) = config_path() {
        if let Some(dir) = p.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(t) = serde_json::to_string_pretty(cfg) {
            let _ = fs::write(p, t);
        }
    }
}

// ───────────────────────── 鉴权 / REST ─────────────────────────

/// 用 App ID/Secret 换 tenant_access_token。
fn fetch_tenant_token(cfg: &FeishuConfig) -> Result<String, String> {
    let url = format!(
        "{}/open-apis/auth/v3/tenant_access_token/internal",
        cfg.base_url()
    );
    let resp = ureq::post(&url)
        .set("Content-Type", "application/json; charset=utf-8")
        .send_json(serde_json::json!({
            "app_id": cfg.app_id,
            "app_secret": cfg.app_secret,
        }))
        .map_err(|e| format!("请求 token 失败: {e}"))?;
    let v: serde_json::Value = resp.into_json().map_err(|e| e.to_string())?;
    let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
    if code != 0 {
        let msg = v.get("msg").and_then(|m| m.as_str()).unwrap_or("unknown");
        return Err(format!("飞书返回错误 code={code}: {msg}"));
    }
    v.get("tenant_access_token")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "响应缺少 tenant_access_token".into())
}

/// 拉机器人自身信息（open_id + 名称），用于连接测试与「过滤自己的消息」。
fn fetch_bot_info(cfg: &FeishuConfig, token: &str) -> Result<(String, String), String> {
    let url = format!("{}/open-apis/bot/v3/info", cfg.base_url());
    let resp = ureq::get(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|e| format!("请求机器人信息失败: {e}"))?;
    let v: serde_json::Value = resp.into_json().map_err(|e| e.to_string())?;
    let bot = v.get("bot").cloned().unwrap_or(serde_json::Value::Null);
    let name = bot
        .get("app_name")
        .and_then(|n| n.as_str())
        .unwrap_or("(未知)")
        .to_string();
    let open_id = bot
        .get("open_id")
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_string();
    Ok((name, open_id))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuTestResult {
    pub ok: bool,
    pub bot_name: String,
    pub bot_open_id: String,
    pub message: String,
}

// ───────────────────────── 去重环（纯逻辑，可测） ─────────────────────────

/// 最近 N 条 message_id 去重，防 WebSocket 重投导致重复回答。
pub struct DedupRing {
    cap: usize,
    queue: VecDeque<String>,
    set: HashSet<String>,
}
impl DedupRing {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            queue: VecDeque::new(),
            set: HashSet::new(),
        }
    }
    /// 见过返回 true（应丢弃）；首见返回 false 并记录。
    pub fn seen(&mut self, id: &str) -> bool {
        if self.set.contains(id) {
            return true;
        }
        self.set.insert(id.to_string());
        self.queue.push_back(id.to_string());
        while self.queue.len() > self.cap {
            if let Some(old) = self.queue.pop_front() {
                self.set.remove(&old);
            }
        }
        false
    }
}

// ───────────────────────── 权限判定（纯逻辑，可测） ─────────────────────────

pub struct IncomingCtx<'a> {
    pub chat_type: &'a str, // "p2p"(私聊) | "group"
    pub sender_open_id: &'a str,
    pub bot_open_id: &'a str,
    pub mentioned_bot: bool,
}

/// 是否应处理该消息（去重之外的策略闸门）。
pub fn is_allowed(cfg: &FeishuConfig, ctx: &IncomingCtx) -> bool {
    // 永不处理自己发的消息（防自言自语死循环）
    if !ctx.bot_open_id.is_empty() && ctx.sender_open_id == ctx.bot_open_id {
        return false;
    }
    if ctx.chat_type == "p2p" {
        return match cfg.dm_policy.as_str() {
            "disabled" => false,
            "allowlist" => cfg.allow_from.iter().any(|id| id == ctx.sender_open_id),
            _ => true, // open
        };
    }
    // 群聊：默认需 @机器人
    if cfg.group_require_mention {
        return ctx.mentioned_bot;
    }
    true
}

// ───────────────────────── ReplyGuard（纯逻辑，可测） ─────────────────────────

/// 若回复「口头承诺了定时/提醒」但「实际未成功创建」，返回纠正文案替换原回复，
/// 否则返回 None（原样发送）。核对自然语言承诺 vs 工具实际结果，防 AI 撒谎。
pub fn guard_reply(text: &str, scheduled_ok: bool) -> Option<String> {
    if scheduled_ok {
        return None;
    }
    let committed = REMINDER_PATTERNS.iter().any(|p| text.contains(p));
    if committed {
        Some(
            "（系统提示）本次未能真正创建定时/提醒任务，所以不会自动提醒你。请重试或换种说法。"
                .to_string(),
        )
    } else {
        None
    }
}

const REMINDER_PATTERNS: &[&str] = &[
    "我会提醒",
    "我会在",
    "已设置提醒",
    "已创建提醒",
    "定时任务创建成功",
    "到时间我会",
    "届时提醒",
    "稍后提醒你",
    "稍后提醒您",
];

// ───────────────────────── Tauri commands ─────────────────────────

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn feishu_get_config() -> FeishuConfig {
    let mut cfg = read_config();
    // 不把 secret 明文回前端（仅指示是否已填）
    if !cfg.app_secret.is_empty() {
        cfg.app_secret = "********".into();
    }
    cfg
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn feishu_set_config(config: FeishuConfig) -> Result<(), String> {
    let mut cfg = config;
    // 前端回传的占位 secret 表示「不修改」，保留原值
    if cfg.app_secret == "********" {
        cfg.app_secret = read_config().app_secret;
    }
    write_config(&cfg);
    Ok(())
}

/// 连接测试：取 token + 机器人信息。验证凭证是否可用（阶段 A 的核心可验证项）。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn feishu_test_connection() -> FeishuTestResult {
    let cfg = read_config();
    if cfg.app_id.trim().is_empty() || cfg.app_secret.trim().is_empty() {
        return FeishuTestResult {
            ok: false,
            bot_name: String::new(),
            bot_open_id: String::new(),
            message: "请先填写 App ID 与 App Secret".into(),
        };
    }
    match fetch_tenant_token(&cfg) {
        Ok(token) => match fetch_bot_info(&cfg, &token) {
            Ok((name, open_id)) => FeishuTestResult {
                ok: true,
                bot_name: name.clone(),
                bot_open_id: open_id,
                message: format!("连接成功：机器人「{name}」凭证有效"),
            },
            Err(e) => FeishuTestResult {
                ok: false,
                bot_name: String::new(),
                bot_open_id: String::new(),
                message: format!("token 正常但拉取机器人信息失败：{e}"),
            },
        },
        Err(e) => FeishuTestResult {
            ok: false,
            bot_name: String::new(),
            bot_open_id: String::new(),
            message: e,
        },
    }
}

// ───────────────────────── 扫码创建机器人 ─────────────────────────
//
// 飞书没有「扫码即自动下发 App ID/Secret」的公开能力（企业微信智能机器人才有），
// 所以「扫码创建机器人」= 把飞书开放平台「创建企业自建应用」入口编码成二维码：
// 手机飞书扫一扫直达建应用页，建好后回到下方表单填 App ID/Secret 即接好机器人。
// 同时给一个「在浏览器打开」桌面兜底。诚实、可用，不伪造不存在的自动下发流程。

/// 飞书开放平台「创建应用」入口（按国内/国际域名区分）。
fn console_url(cfg: &FeishuConfig) -> &'static str {
    if cfg.domain == "lark" {
        "https://open.larksuite.com/app"
    } else {
        "https://open.feishu.cn/app"
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuQrResult {
    /// 二维码 SVG（本地生成，前端可直接内联渲染）。
    pub svg: String,
    /// 二维码指向的飞书开放平台建应用 URL（供「在浏览器打开」复用）。
    pub url: String,
}

/// 生成「扫码创建机器人」二维码：内容为飞书开放平台建应用入口。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn feishu_create_qr() -> Result<FeishuQrResult, String> {
    use qrcode::render::svg;
    use qrcode::QrCode;

    let cfg = read_config();
    let url = console_url(&cfg);
    let code = QrCode::new(url.as_bytes()).map_err(|e| format!("生成二维码失败: {e}"))?;
    let svg = code
        .render::<svg::Color>()
        .min_dimensions(240, 240)
        .quiet_zone(true)
        .dark_color(svg::Color("#111111"))
        .light_color(svg::Color("#ffffff"))
        .build();
    Ok(FeishuQrResult {
        svg,
        url: url.to_string(),
    })
}

/// 在系统默认浏览器打开飞书开放平台建应用页（桌面兜底，等价于扫码）。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn feishu_open_console() -> Result<(), String> {
    let cfg = read_config();
    let url = console_url(&cfg);
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ───────────────────────── 飞书对话引擎（阶段B：Node 桥长连接 → headless claude → 回发）─────────────────────────
//
// Node 桥(assets/feishu_bridge.mjs)借飞书官方 SDK 的 WSClient 起长连接收消息 → stdout(JSON 行)；
// 本模块读 stdout → 去重/权限闸门 → 跑 headless claude 得回复 → 写桥 stdin → 桥发回飞书。
// 借官方 SDK 的可靠长连接实现，避免 Rust 自撸飞书 protobuf 帧的高复杂度与高出错率。

const BRIDGE_MJS: &str = include_str!("../assets/feishu_bridge.mjs");
const BRIDGE_PKG: &str = include_str!("../assets/feishu_bridge_package.json");

struct Gateway {
    pid: Option<u32>,
    stdin: Option<ChildStdin>,
    running: bool,
}
static GATEWAY: Lazy<Mutex<Gateway>> =
    Lazy::new(|| Mutex::new(Gateway { pid: None, stdin: None, running: false }));
static GW_DEDUP: Lazy<Mutex<DedupRing>> = Lazy::new(|| Mutex::new(DedupRing::new(256)));
/// 网关「应当在运行」总开关：守护线程据此决定崩溃后是否自动重起；stop 时置 false。
static SHOULD_RUN: AtomicBool = AtomicBool::new(false);

fn bridge_dir() -> Option<PathBuf> {
    UserDirs::new().map(|u| u.home_dir().join("Polaris").join("feishu-bridge"))
}
fn emit_log(app: &AppHandle, text: impl Into<String>) {
    let _ = app.emit("feishu://log", text.into());
}
/// 同步阻塞跑一次 chat_send（飞书 bridge 在独立线程里调用）。
/// 桌面走 tauri 运行时；server 临时建一个 current-thread tokio 运行时。
#[cfg(feature = "desktop")]
fn block_on_chat_send(
    app: AppHandle,
    args: crate::chat::ChatSendArgs,
) -> Result<String, String> {
    tauri::async_runtime::block_on(crate::chat::chat_send(app, args))
}
#[cfg(not(feature = "desktop"))]
fn block_on_chat_send(
    app: AppHandle,
    args: crate::chat::ChatSendArgs,
) -> Result<String, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("建运行时失败: {e}"))?
        .block_on(crate::chat::chat_send(app, args))
}

fn emit_status(app: &AppHandle, state: &str) {
    let _ = app.emit("feishu://status", state.to_string());
}

/// 物化桥脚本到 ~/Polaris/feishu-bridge 并确保依赖已装。返回桥目录。
fn ensure_bridge(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = bridge_dir().ok_or("无法定位用户目录")?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::write(dir.join("bridge.mjs"), BRIDGE_MJS).map_err(|e| e.to_string())?;
    fs::write(dir.join("package.json"), BRIDGE_PKG).map_err(|e| e.to_string())?;
    if !dir.join("node_modules").join("@larksuiteoapi").exists() {
        emit_status(app, "installing");
        emit_log(app, "首次启动：正在安装飞书 SDK 依赖（npm install，请稍候）…");
        if !npm_install(&dir)? {
            return Err("npm install 失败：请确认已安装 Node.js / npm".into());
        }
        emit_log(app, "依赖安装完成。");
    }
    Ok(dir)
}

fn npm_install(dir: &std::path::Path) -> Result<bool, String> {
    #[allow(unused_mut)]
    let mut cmd;
    #[cfg(target_os = "windows")]
    {
        cmd = Command::new("cmd"); // CreateProcessW 不认 npm.cmd → 经 cmd /c
        cmd.args([
            "/C",
            "npm",
            "install",
            "--no-audit",
            "--no-fund",
            "--registry=https://registry.npmmirror.com",
        ]);
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    #[cfg(not(target_os = "windows"))]
    {
        cmd = Command::new("npm");
        cmd.args([
            "install",
            "--no-audit",
            "--no-fund",
            "--registry=https://registry.npmmirror.com",
        ]);
    }
    cmd.current_dir(dir);
    crate::doctor::harden_child_env(&mut cmd);
    let out = cmd.output().map_err(|e| format!("调起 npm 失败: {e}"))?;
    Ok(out.status.success())
}

/// 飞书机器人专用项目名：所有飞书会话都落在这个 Polaris 项目下，UI 里可见。
const FEISHU_PROJECT_NAME: &str = "飞书机器人";

/// 取/建「飞书机器人」项目下、对应该 chat_id 的对话，返回 conversation_id。
/// 每个飞书会话 ↔ 一条 Polaris 对话（标题带 chat_id），跨重启可复用、平台上可见。
fn ensure_feishu_conversation(chat_id: &str) -> Result<String, String> {
    let pid = match crate::conv::conv_list_projects()
        .into_iter()
        .find(|p| p.name == FEISHU_PROJECT_NAME && !p.archived)
    {
        Some(p) => p.id,
        None => crate::conv::conv_create_project(FEISHU_PROJECT_NAME.into())?.id,
    };
    let title = format!("飞书 · {chat_id}");
    if let Some(c) = crate::conv::conversations_of_project(&pid)
        .into_iter()
        .find(|c| c.title == title)
    {
        return Ok(c.id);
    }
    let c = crate::conv::conv_create_conversation(pid)?;
    let _ = crate::conv::conv_rename_conversation(c.id.clone(), title);
    Ok(c.id)
}

/// 轮询对话，等到出现「比 before_asst 多」的 assistant 消息，返回其正文（剥掉产物 marker）。
fn poll_new_assistant(conv_id: &str, before_asst: usize, timeout: Duration) -> Option<String> {
    let deadline = Instant::now() + timeout;
    loop {
        let msgs = crate::conv::get_messages(conv_id);
        let assts: Vec<&crate::conv::Message> =
            msgs.iter().filter(|m| m.role == "assistant").collect();
        if assts.len() > before_asst {
            let mut content = assts.last().unwrap().content.clone();
            if let Some(idx) = content.find(crate::chat::ARTIFACT_MARKER_PREFIX) {
                content.truncate(idx);
            }
            return Some(content.trim().to_string());
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(800));
    }
}

fn kill_pid(pid: u32) {
    #[cfg(target_os = "windows")]
    {
        let mut c = Command::new("taskkill");
        c.args(["/PID", &pid.to_string(), "/T", "/F"]);
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
        let _ = c.output();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = Command::new("kill").arg(pid.to_string()).output();
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayStatus {
    pub running: bool,
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn feishu_gateway_status() -> GatewayStatus {
    GatewayStatus { running: SHOULD_RUN.load(Ordering::Relaxed) }
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn feishu_gateway_stop(app: AppHandle) -> Result<(), String> {
    SHOULD_RUN.store(false, Ordering::Relaxed);
    let mut g = GATEWAY.lock();
    if let Some(pid) = g.pid.take() {
        kill_pid(pid);
    }
    g.stdin = None;
    g.running = false;
    drop(g);
    emit_status(&app, "stopped");
    emit_log(&app, "网关已停止。");
    Ok(())
}

/// 处理桥的一行 JSON 输出。message → 走 chat_send 真管线让 Claude Code 执行并回发飞书。
fn handle_bridge_line(app: &AppHandle, cfg: &FeishuConfig, bot_open_id: &str, line: &str) {
    let v: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return,
    };
    match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
        "status" => match v.get("state").and_then(|s| s.as_str()) {
            Some("connected") => {
                emit_status(app, "connected");
                emit_log(app, "长连接已建立，机器人在线。");
            }
            Some("reconnecting") => {
                emit_status(app, "reconnecting");
                emit_log(app, "连接中断，正在自动重连…");
            }
            _ => {}
        },
        "log" => emit_log(app, v.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string()),
        "fatal" => emit_log(
            app,
            format!("致命错误: {}", v.get("text").and_then(|t| t.as_str()).unwrap_or("")),
        ),
        "message" => {
            let msg_id = v.get("messageId").and_then(|x| x.as_str()).unwrap_or("");
            if GW_DEDUP.lock().seen(msg_id) {
                return;
            }
            let text = v.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let chat_id = v.get("chatId").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let ctx = IncomingCtx {
                chat_type: v.get("chatType").and_then(|x| x.as_str()).unwrap_or("p2p"),
                sender_open_id: v.get("senderOpenId").and_then(|x| x.as_str()).unwrap_or(""),
                bot_open_id,
                mentioned_bot: v.get("mentioned").and_then(|x| x.as_bool()).unwrap_or(false),
            };
            if text.is_empty() || !is_allowed(cfg, &ctx) {
                return;
            }
            // 接进 Polaris 真对话管线：「飞书机器人」项目下建/取对话 → chat_send(AutoAll,全工具)
            // 让 Claude Code 真执行(可操作软件/写文件) → 落库(UI 实时可见) → 轮询回复发回飞书。
            let conv_id = match ensure_feishu_conversation(&chat_id) {
                Ok(id) => id,
                Err(e) => {
                    emit_log(app, format!("建对话失败: {e}"));
                    return;
                }
            };
            let before_asst = crate::conv::get_messages(&conv_id)
                .iter()
                .filter(|m| m.role == "assistant")
                .count();
            emit_log(app, format!("收到：{text} → 交给 Claude Code（项目「飞书机器人」）执行…"));
            let args = crate::chat::ChatSendArgs {
                prompt: text.clone(),
                permission_mode: crate::chat::PermissionMode::AutoAll,
                use_sandbox: false,
                skill_ids: None,
                conversation_id: Some(conv_id.clone()),
                goal: None,
                dynamic_workflow: false,
                use_kb: false,
                batch_build: false,
                batch_size: None,
            };
            if let Err(e) = block_on_chat_send(app.clone(), args) {
                emit_log(app, format!("调起对话失败: {e}"));
                return;
            }
            let reply = match poll_new_assistant(&conv_id, before_asst, Duration::from_secs(600)) {
                Some(r) if !r.is_empty() => r,
                _ => {
                    emit_log(app, "等待 Claude 回复超时或为空。");
                    return;
                }
            };
            let payload = serde_json::json!({"type":"reply","chatId":chat_id,"text":reply}).to_string();
            let mut g = GATEWAY.lock();
            if let Some(si) = g.stdin.as_mut() {
                let _ = si.write_all(payload.as_bytes());
                let _ = si.write_all(b"\n");
                let _ = si.flush();
            }
            drop(g);
            emit_log(app, "已回复。");
        }
        _ => {}
    }
}

/// 起一次桥进程并读到其退出；返回本次连接存活秒数（守护线程据此决定退避）。
fn run_bridge_once(app: &AppHandle, dir: &std::path::Path, cfg: &FeishuConfig, bot_open_id: &str) -> u64 {
    let started = Instant::now();
    let mut cmd = Command::new("node");
    cmd.arg(dir.join("bridge.mjs"))
        .current_dir(dir)
        .env("FEISHU_APP_ID", &cfg.app_id)
        .env("FEISHU_APP_SECRET", &cfg.app_secret)
        .env("FEISHU_DOMAIN", &cfg.domain)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    crate::doctor::harden_child_env(&mut cmd);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            emit_log(app, format!("调起 node 桥失败（确认已装 Node.js）: {e}"));
            return 0;
        }
    };
    let pid = child.id();
    // stderr 必须排水: 设了 piped 却没人读, node 写满 ~64KB 管道缓冲就会阻塞, 连带停止
    // 写 stdout → 网关「活着但不发消息」静默挂死, 守护线程也不会重起。开线程读掉并转日志。
    if let Some(stderr) = child.stderr.take() {
        let app_err = app.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let line = line.trim();
                if !line.is_empty() {
                    emit_log(&app_err, format!("[bridge stderr] {line}"));
                }
            }
        });
    }
    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => return 0,
    };
    {
        let mut g = GATEWAY.lock();
        g.pid = Some(pid);
        g.stdin = child.stdin.take();
        g.running = true;
    }
    let reader = BufReader::new(stdout);
    for line in reader.lines().map_while(Result::ok) {
        let line = line.trim();
        if !line.is_empty() {
            handle_bridge_line(app, cfg, bot_open_id, line);
        }
        if !SHOULD_RUN.load(Ordering::Relaxed) {
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    {
        let mut g = GATEWAY.lock();
        g.stdin = None;
        g.pid = None;
        g.running = false;
    }
    started.elapsed().as_secs()
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn feishu_gateway_start(app: AppHandle) -> Result<(), String> {
    if SHOULD_RUN.load(Ordering::Relaxed) {
        return Err("网关已在运行".into());
    }
    let cfg = read_config();
    if cfg.app_id.trim().is_empty() || cfg.app_secret.trim().is_empty() {
        return Err("请先填写并保存 App ID 与 App Secret".into());
    }
    emit_status(&app, "starting");
    let dir = ensure_bridge(&app)?;
    // 机器人 open_id（「不回复自己」闸门），best-effort
    let bot_open_id = fetch_tenant_token(&cfg)
        .and_then(|t| fetch_bot_info(&cfg, &t))
        .map(|(_, oid)| oid)
        .unwrap_or_default();

    // 守护线程：进程崩了/断了就带指数退避自动重起（防断）。
    SHOULD_RUN.store(true, Ordering::Relaxed);
    let app2 = app.clone();
    std::thread::spawn(move || {
        let mut backoff = 1u64;
        while SHOULD_RUN.load(Ordering::Relaxed) {
            let lived = run_bridge_once(&app2, &dir, &cfg, &bot_open_id);
            if !SHOULD_RUN.load(Ordering::Relaxed) {
                break;
            }
            if lived >= 20 {
                backoff = 1; // 连过一阵才断 → 重置退避
            }
            emit_status(&app2, "reconnecting");
            emit_log(&app2, format!("网关进程退出，{backoff}s 后自动重起…"));
            let mut waited = 0u64;
            while waited < backoff * 10 && SHOULD_RUN.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(100));
                waited += 1;
            }
            backoff = (backoff * 2).min(30);
        }
        let mut g = GATEWAY.lock();
        g.running = false;
        g.stdin = None;
        g.pid = None;
        drop(g);
        emit_status(&app2, "stopped");
        emit_log(&app2, "网关已停止。");
    });

    emit_log(&app, "网关启动中…");
    Ok(())
}

/// App 启动时调用：若配置开了 auto_start 且凭证齐全，则后台自动拉起网关（不阻塞启动）。
pub fn auto_start_if_enabled(app: &AppHandle) {
    let cfg = read_config();
    if !cfg.auto_start || cfg.app_id.trim().is_empty() || cfg.app_secret.trim().is_empty() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        // 等启动稳定（PATH/网络就绪）后再拉起
        std::thread::sleep(Duration::from_secs(3));
        let _ = feishu_gateway_start(app);
    });
}

// ───────────────────────── 单元测试 ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_rejects_repeat_and_evicts() {
        let mut ring = DedupRing::new(2);
        assert!(!ring.seen("a"));
        assert!(ring.seen("a")); // 重复
        assert!(!ring.seen("b"));
        assert!(!ring.seen("c")); // 触发淘汰 "a"
        assert!(!ring.seen("a")); // "a" 已被淘汰，视为首见
    }

    fn cfg_with(dm: &str, allow: &[&str], require_mention: bool) -> FeishuConfig {
        FeishuConfig {
            dm_policy: dm.into(),
            group_require_mention: require_mention,
            allow_from: allow.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn never_reply_to_self() {
        let cfg = cfg_with("open", &[], true);
        let ctx = IncomingCtx {
            chat_type: "p2p",
            sender_open_id: "bot1",
            bot_open_id: "bot1",
            mentioned_bot: false,
        };
        assert!(!is_allowed(&cfg, &ctx));
    }

    #[test]
    fn dm_policy_gates() {
        let open = cfg_with("open", &[], true);
        let allow = cfg_with("allowlist", &["u1"], true);
        let off = cfg_with("disabled", &[], true);
        let mk = |sender: &'static str| IncomingCtx {
            chat_type: "p2p",
            sender_open_id: sender,
            bot_open_id: "bot",
            mentioned_bot: false,
        };
        assert!(is_allowed(&open, &mk("u2")));
        assert!(is_allowed(&allow, &mk("u1")));
        assert!(!is_allowed(&allow, &mk("u2")));
        assert!(!is_allowed(&off, &mk("u1")));
    }

    #[test]
    fn group_requires_mention() {
        let cfg = cfg_with("open", &[], true);
        let no_at = IncomingCtx {
            chat_type: "group",
            sender_open_id: "u1",
            bot_open_id: "bot",
            mentioned_bot: false,
        };
        let at = IncomingCtx {
            mentioned_bot: true,
            ..no_at
        };
        assert!(!is_allowed(&cfg, &no_at));
        assert!(is_allowed(&cfg, &at));
    }

    #[test]
    fn reply_guard_catches_empty_promise() {
        // 承诺了提醒但没真正创建 → 拦截
        assert!(guard_reply("好的，我会提醒你开会", false).is_some());
        // 真创建成功 → 放行
        assert!(guard_reply("好的，我会提醒你开会", true).is_none());
        // 没有承诺 → 放行
        assert!(guard_reply("这是你要的总结。", false).is_none());
    }
}
