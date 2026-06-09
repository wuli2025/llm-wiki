//! 自动更新 —— 显式状态机 + 单飞(single-flight) + 持久化「可装版本」+ emit 订阅
//!
//! 借鉴 OpenCode 桌面端 `updater-controller.ts` 的模式（见桌面端优化建议 ①）：
//! 把原先「纯前端、一堆离散 ref」的更新逻辑收进后端一个**唯一状态机**：
//!
//!   disabled / idle / checking / up-to-date / available / downloading / ready / installing / error
//!
//! - **单飞**：并发的 `check` / `apply` 只跑一次（`in_flight` 标志），多次点击不会重入。
//! - **可观测**：每次状态流转都 `emit("updater://state")`，前端订阅即得，无需各自轮询。
//! - **持久化 + 重启续提示**：发现新版本时把 `{version,notes}` 落盘；下次启动若它 ≠ 当前版本，
//!   立即先把状态摆成 `available`（离线也能看到「有更新待装」），再后台校验刷新；
//!   若已等于当前版本（说明已装上）→ 清掉标记。对标 OpenCode 的
//!   `if ready?.version === currentVersion clear`。
//!
//! 机制仍走 `tauri-plugin-updater`（GitHub Releases），这里补的是**结构**：可观测、防重入、续提示。

use anyhow::Result;
use directories::UserDirs;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

/// 推给前端的事件名；前端 `listen("updater://state")` 即得整个状态机当前态。
const EVENT: &str = "updater://state";

// ───────────────────────── 状态机 ─────────────────────────

/// 更新器的唯一可观测状态。`#[serde(tag="status")]` → 前端拿到
/// `{status:"available", version:"0.2.18", notes:"…"}` 这样的判别联合。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum UpdaterState {
    /// 更新器被禁用（非 Tauri 运行时 / 显式关闭）。
    Disabled,
    /// 空闲，尚未检查。
    Idle,
    /// 正在向 Releases 询问。
    Checking,
    /// 已是最新。
    UpToDate,
    /// 发现新版本，尚未下载。
    Available { version: String, notes: String },
    /// 正在下载（带百分比）。
    Downloading { version: String, percent: u8 },
    /// 已就绪（下载完成、即将/正在安装）。
    Ready { version: String },
    /// 正在安装（安装完成后自重启生效）。
    Installing { version: String },
    /// 出错（检查 / 下载 / 安装任一环节）。
    Error { message: String },
}

// ───────────────────────── 持久化 ─────────────────────────

/// 落盘的「可装版本」标记（只存这点轻量事实，可跨重启存活）。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Persisted {
    version: String,
    #[serde(default)]
    notes: String,
}

// ───────────────────────── 进程内单例 ─────────────────────────

struct Inner {
    state: UpdaterState,
    /// 单飞：check / apply 进行中时为 true，拦住并发重入。
    in_flight: bool,
    current_version: String,
    persist_path: PathBuf,
    enabled: bool,
}

static UPDATER: Lazy<Mutex<Inner>> = Lazy::new(|| {
    Mutex::new(Inner {
        state: UpdaterState::Idle,
        in_flight: false,
        current_version: String::new(),
        persist_path: PathBuf::new(),
        enabled: true,
    })
});

// ───────────────────────── 初始化 ─────────────────────────

/// 启动时调用一次（在 `setup` 内）：记录当前版本 + 持久化路径，
/// 并据落盘标记做「重启续提示」。不 emit（此刻前端还没监听）。
pub fn init(app: &AppHandle) -> Result<()> {
    let user = UserDirs::new().ok_or_else(|| anyhow::anyhow!("no user dir"))?;
    let dir = user.home_dir().join("Polaris").join("data");
    fs::create_dir_all(&dir)?;
    let path = dir.join("updater.json");

    let current = app.package_info().version.to_string();

    let mut g = UPDATER.lock();
    g.current_version = current.clone();
    g.persist_path = path.clone();

    // 重启续提示：上次发现的版本若仍 ≠ 当前 → 直接摆成 available（离线可见）；
    // 若已 == 当前（已装上）→ 清标记。
    if let Some(p) = load_persisted(&path) {
        if p.version != current {
            g.state = UpdaterState::Available {
                version: p.version,
                notes: p.notes,
            };
        } else {
            let _ = fs::remove_file(&path);
        }
    }
    Ok(())
}

fn load_persisted(path: &PathBuf) -> Option<Persisted> {
    let txt = fs::read_to_string(path).ok()?;
    serde_json::from_str(&txt).ok()
}

fn persist_available(version: &str, notes: &str) {
    let path = { UPDATER.lock().persist_path.clone() };
    if path.as_os_str().is_empty() {
        return;
    }
    let p = Persisted {
        version: version.to_string(),
        notes: notes.to_string(),
    };
    if let Ok(txt) = serde_json::to_string(&p) {
        let _ = fs::write(&path, txt);
    }
}

fn clear_persisted() {
    let path = { UPDATER.lock().persist_path.clone() };
    if !path.as_os_str().is_empty() {
        let _ = fs::remove_file(&path);
    }
}

// ───────────────────────── 状态流转 ─────────────────────────

/// 写入新状态并广播给前端。锁只在写状态时短暂持有，emit 在锁外（避免跨 await 持锁）。
fn transition(app: &AppHandle, next: UpdaterState) -> UpdaterState {
    {
        let mut g = UPDATER.lock();
        g.state = next.clone();
    }
    let _ = app.emit(EVENT, &next);
    next
}

/// 纯函数：根据「当前版本」与「检查结果」决定落点状态。抽出来便于单测（对标 OpenCode 的注入式可测性）。
pub fn resolve_check(current: &str, found: Option<(String, String)>) -> UpdaterState {
    match found {
        Some((version, notes)) if version != current => UpdaterState::Available { version, notes },
        _ => UpdaterState::UpToDate,
    }
}

// ───────────────────────── 核心动作 ─────────────────────────

async fn run_check(app: &AppHandle) -> UpdaterState {
    transition(app, UpdaterState::Checking);
    let current = { UPDATER.lock().current_version.clone() };

    let updater = match app.updater() {
        Ok(u) => u,
        Err(e) => return transition(app, UpdaterState::Error { message: format!("更新器不可用: {e}") }),
    };

    match updater.check().await {
        Ok(found) => {
            let mapped = resolve_check(
                &current,
                found.map(|up| (up.version.clone(), up.body.clone().unwrap_or_default())),
            );
            match &mapped {
                UpdaterState::Available { version, notes } => persist_available(version, notes),
                _ => clear_persisted(),
            }
            transition(app, mapped)
        }
        // 检查失败不清落盘标记（之前发现的「可装版本」仍有效，离线时照样能续提示）。
        Err(e) => transition(app, UpdaterState::Error { message: format!("检查更新失败: {e}") }),
    }
}

/// 构造下载候选源：从 latest.json 给出的 url 里剥出裸 `github.com` 地址，
/// 再依次套国内镜像前缀、最后回退直连 github。**国内更新失败的根因就是下载这一跳没走镜像**
/// （检查 latest.json 走了 endpoints 镜像，但 Tauri updater 下载安装包时只认 latest.json 里写死的 url）。
/// download() 内部对字节做 minisign 验签——镜像若被劫持/返回错误页，签名必不过 → 自动跳到下一个源，
/// 故镜像顺序安全，最坏退化到直连。非 github 源（如将来自托管）不套镜像、直连。
fn mirror_candidates(url: &str) -> Vec<String> {
    // latest.json 里的 url 可能本身已是 `https://<镜像>/https://github.com/...`，
    // 取最后一段裸地址，避免把镜像套娃。
    let bare = match url.rfind("https://github.com/") {
        Some(idx) => url[idx..].to_string(),
        None => return vec![url.to_string()],
    };
    // 文件名（路径最后一段）→ Cloudflare 自托管兜底：站点 `polaris-2us.pages.dev/downloads/<文件名>`，
    // 独立于 github + 镜像，国内可达性最好。装包很小(win 6MB / mac 14MB)，Pages 直接扛得住、无需 R2。
    // 发版时把 setup.exe 与 Polaris.app.tar.gz 传进站点 downloads/ 并 `wrangler pages deploy`（见 release-manual）。
    let filename = bare.rsplit('/').next().unwrap_or("");
    let mut out = vec![format!("https://gh-proxy.com/{bare}")];
    // Cloudflare 排第二：首个源一「卡死」(停滞看门狗 ~30s 触发)就直接切到最可靠的自托管源，而非再耗在第二个 github 镜像上。
    if !filename.is_empty() {
        out.push(format!("https://polaris-2us.pages.dev/downloads/{filename}"));
    }
    out.push(format!("https://ghfast.top/{bare}"));
    out.push(bare); // 直连 github，最后兜底
    out
}

async fn run_apply(app: &AppHandle, version: &str) -> Result<(), String> {
    let updater = app.updater().map_err(|e| format!("更新器不可用: {e}"))?;
    let mut update = updater
        .check()
        .await
        .map_err(|e| format!("校验更新失败: {e}"))?
        .ok_or_else(|| "更新已不可用".to_string())?;

    transition(app, UpdaterState::Downloading { version: version.to_string(), percent: 0 });

    // ════════ 防「更新卡死」两道独立闸门 ════════
    // 单道闸门会漏：纯总超时会误杀「慢但在动」的下载；纯停滞检测扛不住「一直慢吞吞吐数据但永不完」。
    // 两道互补、各自独立触发，命中任一道都立刻放弃当前源、切下一个（Cloudflare 排第二）：
    //   ① 总超时(reqwest)：每个源整请求 300s 硬顶——防「连上但永远读不完」的兜底天花板，给慢网留足余量。
    //   ② 停滞看门狗：与下载 future 竞速，连续 STALL_SECS 秒「字节数零增长」即判定冻住、取消该源——
    //      比总超时快得多地识别真正的「卡死」(连接还活着但不出数据 / 连接握手挂起)。
    update.timeout = Some(Duration::from_secs(300)); // 闸门①
    const STALL_SECS: u64 = 30; // 闸门②：30s 无新字节 = 卡死
    const STALL_TICK: u64 = 5; // 看门狗采样间隔

    // 多镜像兜底下载：逐个候选源尝试 download()（内部验签），任一成功即拿到字节。
    let candidates = mirror_candidates(update.download_url.as_str());
    let mut last_err = String::from("无可用下载源");
    let mut bytes: Option<Vec<u8>> = None;

    for (i, cand) in candidates.iter().enumerate() {
        match cand.parse() {
            Ok(u) => update.download_url = u,
            Err(e) => {
                last_err = format!("镜像地址非法 {cand}: {e}");
                continue;
            }
        }

        // 每个候选源都重置进度 + 重建闭包（on_finish 是 FnOnce，on_chunk 内含可变累计状态）。
        transition(app, UpdaterState::Downloading { version: version.to_string(), percent: 0 });
        // 看门狗与 on_chunk 共享的「已下载字节」计数（停滞判定的唯一依据）。
        let progress = Arc::new(AtomicU64::new(0));
        let progress_chunk = progress.clone();
        let app_chunk = app.clone();
        let version_chunk = version.to_string();
        let mut last_pct: i64 = -1;
        let on_chunk = move |chunk_len: usize, content_len: Option<u64>| {
            let downloaded = progress_chunk.fetch_add(chunk_len as u64, Ordering::Relaxed) + chunk_len as u64;
            let pct = match content_len {
                Some(total) if total > 0 => ((downloaded.min(total) * 100) / total) as i64,
                _ => 0,
            };
            if pct != last_pct {
                last_pct = pct;
                transition(
                    &app_chunk,
                    UpdaterState::Downloading { version: version_chunk.clone(), percent: pct as u8 },
                );
            }
        };

        // 闸门②：停滞看门狗。每 STALL_TICK 秒看一次字节数；连续 STALL_SECS 秒零增长 → 结束（判定卡死）。
        let progress_wd = progress.clone();
        let watchdog = async move {
            let mut last = 0u64;
            let mut idle = 0u64;
            loop {
                tokio::time::sleep(Duration::from_secs(STALL_TICK)).await;
                let now = progress_wd.load(Ordering::Relaxed);
                if now > last {
                    last = now;
                    idle = 0;
                } else {
                    idle += STALL_TICK;
                    if idle >= STALL_SECS {
                        break;
                    }
                }
            }
        };

        // 下载 future 与看门狗竞速：谁先完成谁说了算。看门狗先到 → drop 下载 future = 取消在飞的
        // reqwest 连接，立刻切下一个源，而不是干等闸门① 的 300s。
        let download = update.download(on_chunk, || {});
        tokio::pin!(download);
        tokio::pin!(watchdog);
        let outcome = tokio::select! {
            r = &mut download => r.map_err(|e| e.to_string()),
            _ = &mut watchdog => Err(format!("连续 {STALL_SECS}s 无数据，判定卡死")),
        };

        match outcome {
            Ok(b) => {
                bytes = Some(b);
                break;
            }
            // 网络发不出 / 状态非 2xx / 验签失败 / 总超时 / 停滞看门狗 都落这里 → 换下一个源。
            Err(e) => last_err = format!("源{}/{} 失败: {e}", i + 1, candidates.len()),
        }
    }

    let bytes = match bytes {
        Some(b) => b,
        None => {
            let msg = format!("下载失败（已试 {} 个源）：{last_err}", candidates.len());
            transition(app, UpdaterState::Error { message: msg.clone() });
            return Err(msg);
        }
    };

    // 下载完成、开始安装 → Installing。
    transition(app, UpdaterState::Installing { version: version.to_string() });
    update.install(bytes).map_err(|e| {
        let msg = format!("安装失败: {e}");
        transition(app, UpdaterState::Error { message: msg.clone() });
        msg
    })?;

    clear_persisted();
    transition(app, UpdaterState::Ready { version: version.to_string() });
    // 安装完成 → 自重启到新版本（即「关掉、过一会再开就是新版」）。restart() 不返回。
    app.restart();
}

// ───────────────────────── Tauri 命令 ─────────────────────────

/// 前端挂载时取一次当前态（事件之外的同步快照，避免错过 init 阶段设好的状态）。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn updater_get_state() -> UpdaterState {
    UPDATER.lock().state.clone()
}

/// 检查更新（自动 / 手动共用）。单飞：进行中 / 正在下载安装时直接返回当前态。
#[cfg_attr(feature = "desktop", tauri::command)]
pub async fn updater_check(app: AppHandle) -> Result<UpdaterState, String> {
    {
        let mut g = UPDATER.lock();
        if !g.enabled {
            return Ok(g.state.clone());
        }
        if g.in_flight
            || matches!(g.state, UpdaterState::Downloading { .. } | UpdaterState::Installing { .. })
        {
            return Ok(g.state.clone());
        }
        g.in_flight = true;
    }
    let result = run_check(&app).await;
    UPDATER.lock().in_flight = false;
    Ok(result)
}

/// 用户点「立即更新」：下载 + 安装 + 自重启。需当前处于 available / ready。单飞防重入。
#[cfg_attr(feature = "desktop", tauri::command)]
pub async fn updater_apply(app: AppHandle) -> Result<(), String> {
    let version = {
        let mut g = UPDATER.lock();
        if g.in_flight {
            return Err("更新正在进行中".into());
        }
        let v = match &g.state {
            UpdaterState::Available { version, .. } => version.clone(),
            UpdaterState::Ready { version } => version.clone(),
            _ => return Err("当前没有可安装的更新".into()),
        };
        g.in_flight = true;
        v
    };
    let res = run_apply(&app, &version).await;
    // 正常路径里 run_apply 末尾 app.restart() 不返回；走到这里多半是出错了。
    UPDATER.lock().in_flight = false;
    res
}

// ───────────────────────── 单测 ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_resolves_to_available_when_version_differs() {
        let s = resolve_check("0.2.17", Some(("0.2.18".into(), "新特性".into())));
        assert_eq!(
            s,
            UpdaterState::Available { version: "0.2.18".into(), notes: "新特性".into() }
        );
    }

    #[test]
    fn check_resolves_up_to_date_when_no_update() {
        assert_eq!(resolve_check("0.2.17", None), UpdaterState::UpToDate);
    }

    #[test]
    fn check_resolves_up_to_date_when_same_version() {
        // 远程报了一个版本但与当前相同 → 视为已最新（对标 OpenCode 的 version===current 判定）。
        assert_eq!(
            resolve_check("0.2.18", Some(("0.2.18".into(), String::new()))),
            UpdaterState::UpToDate
        );
    }

    #[test]
    fn mirror_candidates_wraps_bare_github_url() {
        let c = mirror_candidates(
            "https://github.com/wuli2025/polaris_coworker/releases/download/v0.2.18/Polaris_0.2.18_x64-setup.exe",
        );
        // gh-proxy / Cloudflare / ghfast / 直连 共 4 个候选源。
        assert_eq!(c.len(), 4);
        assert!(c[0].starts_with("https://gh-proxy.com/https://github.com/"));
        // Cloudflare 排第二（首源卡死即切自托管），按文件名取。
        assert_eq!(
            c[1],
            "https://polaris-2us.pages.dev/downloads/Polaris_0.2.18_x64-setup.exe"
        );
        assert!(c[2].starts_with("https://ghfast.top/https://github.com/"));
        // 末位是直连兜底（无镜像前缀）。
        assert_eq!(
            c[3],
            "https://github.com/wuli2025/polaris_coworker/releases/download/v0.2.18/Polaris_0.2.18_x64-setup.exe"
        );
    }

    #[test]
    fn mirror_candidates_unwraps_already_mirrored_url() {
        // latest.json 里若已写成镜像 url，不能套娃，要剥回裸地址再重套。
        let c = mirror_candidates(
            "https://gh-proxy.com/https://github.com/wuli2025/polaris_coworker/releases/download/v0.2.18/Polaris.app.tar.gz",
        );
        assert_eq!(c.len(), 4);
        // Cloudflare 兜底（第二位）按文件名，不带版本路径前缀。
        assert_eq!(c[1], "https://polaris-2us.pages.dev/downloads/Polaris.app.tar.gz");
        assert_eq!(
            c[3],
            "https://github.com/wuli2025/polaris_coworker/releases/download/v0.2.18/Polaris.app.tar.gz"
        );
        // 不出现双重镜像前缀。
        assert!(!c[0].contains("gh-proxy.com/https://gh-proxy.com"));
    }

    #[test]
    fn mirror_candidates_passthrough_non_github() {
        // 非 github 源（如将来自托管 Cloudflare）直连、不套镜像。
        let c = mirror_candidates("https://polaris-2us.pages.dev/v0.2.18/setup.exe");
        assert_eq!(c, vec!["https://polaris-2us.pages.dev/v0.2.18/setup.exe".to_string()]);
    }

    #[test]
    fn state_serializes_with_status_tag() {
        let json = serde_json::to_string(&UpdaterState::Downloading {
            version: "0.2.18".into(),
            percent: 42,
        })
        .unwrap();
        assert!(json.contains("\"status\":\"downloading\""));
        assert!(json.contains("\"percent\":42"));
    }
}
