//! 自媒体「账号管理」— 平台登录态（浏览器 profile）的探测与解绑。
//!
//! 背景：发文用的 post-to-wechat / post-to-xhs 技能各自把登录态持久化在**固定的浏览器
//! profile 目录**里，扫一次码即可复用。但登录一直埋在发文流程里被动触发、状态不可见，
//! 用户感觉「每次都要扫」。本模块给「账号管理」面板提供 ground-truth：
//! - 公众号：`~/.polaris-mp-profile`（mp_draft.py 的 launch_persistent_context）
//! - 小红书：`%LOCALAPPDATA%\Google\Chrome\XiaohongshuProfiles\default`（account_manager.py），
//!   回退到旧路径 `...\XiaohongshuProfile`。
//!
//! 本模块**只读探测 + 解绑（删 profile）**；真正的扫码登录由前端拉起对话、让 claude 跑技能完成。

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStatus {
    /// 平台 id："wechat" | "xhs"
    pub platform: String,
    /// 展示名
    pub label: String,
    /// 是否已绑定（profile 目录存在且非空 = 扫过码）
    pub bound: bool,
    /// 登录态所在 profile 目录（绝对路径，给用户看 / 排查）
    pub profile_dir: String,
    /// profile 最近活动时间（unix 秒）；未绑定为 None
    pub last_active: Option<i64>,
    /// 一句话说明
    pub detail: String,
}

fn home() -> PathBuf {
    directories::UserDirs::new()
        .map(|u| u.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn local_app_data() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(home)
}

/// 公众号登录态目录（与 post-to-wechat/scripts/mp_draft.py 的 PROFILE_DIR 一致）。
fn wechat_profile() -> PathBuf {
    home().join(".polaris-mp-profile")
}

/// 小红书登录态候选目录（新路径优先，回退旧路径），与 post-to-xhs 的 account_manager 一致。
fn xhs_profile_candidates() -> Vec<PathBuf> {
    let lad = local_app_data();
    vec![
        lad.join("Google").join("Chrome").join("XiaohongshuProfiles").join("default"),
        lad.join("Google").join("Chrome").join("XiaohongshuProfile"),
    ]
}

/// 目录是否存在且非空（= 浏览器写过 profile = 扫过码）。
fn dir_bound(p: &Path) -> bool {
    fs::read_dir(p)
        .map(|mut it| it.next().is_some())
        .unwrap_or(false)
}

/// 取目录最近修改时间（unix 秒）。优先看 profile 内的会话文件（Cookies / 顶层条目），
/// 退化为目录自身 mtime。只看一层，避免遍历整个 Chrome profile。
fn dir_last_active(p: &Path) -> Option<i64> {
    let mut latest: Option<i64> = None;
    let mut consider = |path: &Path| {
        if let Ok(meta) = fs::metadata(path) {
            if let Ok(m) = meta.modified() {
                if let Ok(d) = m.duration_since(UNIX_EPOCH) {
                    let secs = d.as_secs() as i64;
                    if latest.map_or(true, |cur| secs > cur) {
                        latest = Some(secs);
                    }
                }
            }
        }
    };
    consider(p);
    if let Ok(entries) = fs::read_dir(p) {
        for e in entries.flatten().take(64) {
            consider(&e.path());
        }
    }
    latest
}

fn wechat_status() -> AccountStatus {
    let dir = wechat_profile();
    let bound = dir_bound(&dir);
    AccountStatus {
        platform: "wechat".into(),
        label: "微信公众号".into(),
        bound,
        last_active: if bound { dir_last_active(&dir) } else { None },
        profile_dir: dir.to_string_lossy().into_owned(),
        detail: if bound {
            "已扫码绑定，发文复用此登录态。公众号 session 会过期，过期后重新扫一次即可。".into()
        } else {
            "尚未绑定。点「扫码绑定」登录一次公众号后台，之后发文不再重复扫码。".into()
        },
    }
}

fn xhs_status() -> AccountStatus {
    let candidates = xhs_profile_candidates();
    // 选第一个已绑定的；都没有则用首选路径作为「未绑定」展示。
    let chosen = candidates.iter().find(|p| dir_bound(p)).cloned();
    let (dir, bound) = match chosen {
        Some(p) => (p, true),
        None => (candidates[0].clone(), false),
    };
    AccountStatus {
        platform: "xhs".into(),
        label: "小红书".into(),
        bound,
        last_active: if bound { dir_last_active(&dir) } else { None },
        profile_dir: dir.to_string_lossy().into_owned(),
        detail: if bound {
            "已扫码绑定，发文复用此 Chrome 登录态，扫一次能用较久。".into()
        } else {
            "尚未绑定。点「扫码绑定」登录一次小红书，之后发文不再重复扫码。".into()
        },
    }
}

/// 列出各平台登录态（账号管理面板）。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn media_accounts_status() -> Vec<AccountStatus> {
    vec![wechat_status(), xhs_status()]
}

/// 解绑某平台：删除其 profile 目录，强制下次重新扫码登录。
/// 安全：只允许删本模块固定推导出的已知路径，杜绝任意路径删除。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn media_account_forget(platform: String) -> Result<String, String> {
    let targets: Vec<PathBuf> = match platform.as_str() {
        "wechat" => vec![wechat_profile()],
        "xhs" => xhs_profile_candidates(),
        other => return Err(format!("未知平台：{other}")),
    };
    let mut removed = 0usize;
    for dir in targets {
        if dir.exists() {
            fs::remove_dir_all(&dir)
                .map_err(|e| format!("删除 {} 失败：{e}", dir.display()))?;
            removed += 1;
        }
    }
    Ok(if removed > 0 {
        "已解绑：登录态已清除，下次发文需重新扫码。".into()
    } else {
        "本来就没有登录态，无需解绑。".into()
    })
}
