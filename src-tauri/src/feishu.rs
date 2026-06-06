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
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::PathBuf;

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

#[tauri::command]
pub fn feishu_get_config() -> FeishuConfig {
    let mut cfg = read_config();
    // 不把 secret 明文回前端（仅指示是否已填）
    if !cfg.app_secret.is_empty() {
        cfg.app_secret = "********".into();
    }
    cfg
}

#[tauri::command]
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
#[tauri::command]
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
