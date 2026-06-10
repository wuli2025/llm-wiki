//! Polaris Forge · 跨平台渲染能力 preflight(对应《Forge 跨平台 PRD》§06 降级阶梯表)。
//!
//! 本模块**不做渲染**——Forge 渲染引擎(capture/codec/tts/pptx/fx)是 P0–P5 的工程路线。
//! 它先把「这台机器 / 这个容器**能走哪条渲染路、缺什么会降到哪**」探测清楚并透明上报:
//! 产品据此自动选路 + UI 红绿灯,落实两份 PRD 反复强调的「失败被设计过、每级降级都仍交付
//! 可用的东西」。三平台(Windows/macOS/Docker)各自报自己的阶梯,`cfg!(target_os)` 感知。
//!
//! 这是 Forge 工程的**第一块落地件**:在写任何重后端之前,先有一个诚实的能力地图,让用户
//! 一眼看清「我这环境出 PPT/视频走哪条路、要不要补东西」,而不是跑到一半报错。

use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;

/// 当前平台标识(给前端按平台展示对应阶梯)。
pub fn platform() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if Path::new("/.dockerenv").exists() || std::env::var("POLARIS_RENDER_FLAVOR").is_ok() {
        "docker"
    } else {
        "linux"
    }
}

/// 试运行一个可执行 + 版本参数, 成功(能 spawn 且退出码 0)即视为可用, 返回其名/路径。
fn probe_exe(cmd: &str, version_arg: &str) -> bool {
    Command::new(cmd)
        .arg(version_arg)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// 找 chromium/chrome/edge 可执行: 先看显式 env(容器里 ENV 已写 /usr/bin/chromium),
/// 再按平台候选名探测。返回命中的命令字符串。
pub fn find_chromium() -> Option<String> {
    if let Ok(p) = std::env::var("POLARIS_CHROMIUM") {
        if !p.is_empty() && (Path::new(&p).is_file() || probe_exe(&p, "--version")) {
            return Some(p);
        }
    }
    #[allow(unused_mut)] // macOS 分支才 push，其余平台不需要 mut
    let mut candidates: Vec<&str> = vec!["chromium", "chromium-browser", "google-chrome", "chrome"];
    // Windows: Edge/Chrome 常驻固定路径(不在 PATH 也能用)。
    #[cfg(target_os = "windows")]
    let win_paths = [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    ];
    #[cfg(target_os = "windows")]
    for p in win_paths {
        if Path::new(p).is_file() {
            return Some(p.to_string());
        }
    }
    // macOS: Chrome 标准安装路径。
    #[cfg(target_os = "macos")]
    {
        let mac = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
        if Path::new(mac).is_file() {
            return Some(mac.to_string());
        }
        candidates.push("/Applications/Chromium.app/Contents/MacOS/Chromium");
    }
    candidates
        .into_iter()
        .find(|c| probe_exe(c, "--version"))
        .map(|s| s.to_string())
}

/// ffmpeg 是否可用(逃生口 / Docker 主编码器)。
fn find_ffmpeg() -> bool {
    let cmd = std::env::var("POLARIS_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_string());
    probe_exe(&cmd, "-version")
}

/// 中文(CJK)字体是否就位——deck 截图「最隐蔽必踩」坑: 缺了全是豆腐块 □□□。
/// Linux/Docker 用 fc-list 探测; macOS/Windows 系统自带苹方/雅黑, 视为就位。
fn has_cjk_font() -> Option<bool> {
    if cfg!(target_os = "macos") || cfg!(target_os = "windows") {
        return Some(true); // 系统自带 PingFang / Microsoft YaHei
    }
    // Linux/Docker: fc-list :lang=zh 有输出即有中文字体。
    match Command::new("fc-list").arg(":lang=zh").output() {
        Ok(o) if o.status.success() => Some(!o.stdout.is_empty()),
        _ => None, // fc-list 都没有 → 无法判定(多半也没字体)
    }
}

/// 是否配了 MiniMax key(TTS L0 主力)。best-effort: 查常见 env。
fn minimax_key_present() -> bool {
    ["MINIMAX_API_KEY", "POLARIS_MINIMAX_KEY", "MINIMAXI_API_KEY"]
        .iter()
        .any(|k| std::env::var(k).map(|v| !v.is_empty()).unwrap_or(false))
}

/// 渲染能力 preflight 总入口。返回平台 + 各能力的「就绪/将走哪条路/缺啥降到哪」。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn forge_preflight() -> Value {
    let plat = platform();
    let chromium = find_chromium();
    let ffmpeg = find_ffmpeg();
    let cjk = has_cjk_font();
    let minimax = minimax_key_present();

    // ── 截图能力(PPT/视频取帧的前提)──
    let screenshot = match plat {
        "docker" | "linux" => json!({
            "primary": "chromium CDP",
            "ready": chromium.is_some(),
            "path": chromium,
            "degrades_to": "HTML 交付 + 提示浏览器打印(Ctrl/Cmd+P)",
            "blocker": if chromium.is_none() { Some("未发现 chromium：full 镜像才装(POLARIS_RENDER=1)") } else { None }
        }),
        "windows" => json!({
            "primary": "WebView2",
            "fallback": "Edge/Chrome CDP",
            "ready": true,
            "cdp_available": chromium.is_some(),
            "path": chromium,
            "degrades_to": "HTML 交付 + 打印"
        }),
        "macos" => json!({
            "primary": "WKWebView takeSnapshot",
            "ready": true,
            "cdp_available": chromium.is_some(),
            "degrades_to": "HTML 交付 + 打印",
            "note": "WKWebView 后端属 P4-mac，未落地前可用 Chrome CDP 兜底"
        }),
        _ => json!({ "ready": false }),
    };

    // ── 视频编码能力 ──
    let video = match plat {
        "docker" | "linux" => json!({
            "primary": "ffmpeg (镜像自带)",
            "ready": ffmpeg,
            "degrades_to": "交付 deck.html+音频段+timeline，换环境续跑出片",
            "blocker": if !ffmpeg { Some("未发现 ffmpeg：full 镜像才装") } else { None }
        }),
        "windows" => json!({
            "primary": "Media Foundation (P2)",
            "fallback": "ffmpeg(若在 PATH)",
            "ffmpeg_available": ffmpeg,
            "ready": true,
            "degrades_to": "交付 deck+音频+timeline，可续跑"
        }),
        "macos" => json!({
            "primary": "VideoToolbox (P4-mac)",
            "fallback": "ffmpeg(若在 PATH)",
            "ffmpeg_available": ffmpeg,
            "degrades_to": "交付 deck+音频+timeline，可续跑"
        }),
        _ => json!({ "ready": false }),
    };

    // ── 配音(TTS)能力阶梯 ──
    let tts = json!({
        "l0_minimax": { "ready": minimax, "note": "主力，需 key/额度" },
        "l1_edge_free": { "ready": plat != "offline", "note": "免费神经语音(edge-tts)，需联网，P5 接入" },
        "l2_offline_piper": { "ready": false, "note": "离线兜底，P5 可选" },
        "l3_system": {
            "ready": plat == "windows" || plat == "macos",
            "note": if plat == "docker" || plat == "linux" {
                "容器无系统语音 → 出视频默认必须 MiniMax key(诚实缺口)"
            } else {
                "系统语音兜底(Win OneCore / mac AVSpeech)"
            }
        },
        "degrades_to": "出无声版 + 字幕硬烧(内容仍可用)"
    });

    // ── CJK 字体闸(Docker 关键)──
    let fonts = json!({
        "cjk_ready": cjk,
        "critical": plat == "docker" || plat == "linux",
        "note": match cjk {
            Some(true) => "中文字体就位",
            Some(false) => "⚠ 无中文字体：deck 截图会出豆腐块 □□□，应拒跑而非产废片(装 fonts-noto-cjk)",
            None => "无法探测(fc-list 缺失)，多半也无中文字体"
        }
    });

    // ── 整体可出片判定 ──
    let can_render_ppt = match plat {
        "docker" | "linux" => chromium.is_some() && cjk == Some(true),
        _ => true,
    };
    let can_render_video = can_render_ppt && (ffmpeg || plat == "windows" || plat == "macos");

    json!({
        "ok": true,
        "platform": plat,
        "render_flavor": std::env::var("POLARIS_RENDER_FLAVOR").ok(),
        "forge_engine": "planned (P0–P5 路线图，本 preflight 是第一块落地件)",
        "capabilities": {
            "screenshot": screenshot,
            "video": video,
            "tts": tts,
            "fonts": fonts,
            "pptx_pack": { "ready": true, "note": "纯 Rust OOXML，平台无关(引擎 P1 落地)" },
            "animation_fx": { "ready": true, "note": "Web 标准 __fx.seek，三平台一致(引擎 P3 落地)" }
        },
        "summary": {
            "can_render_ppt": can_render_ppt,
            "can_render_video": can_render_video,
            "blockers": preflight_blockers(plat, &chromium, ffmpeg, cjk)
        }
    })
}

// ───────────── Forge 渲染命令(跨平台:win/mac/docker 同一份) ─────────────

/// 把一组幻灯图打成 .pptx(纯 Rust OOXML,替 pptxgenjs)。三平台字节级一致。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn forge_build_pptx(images: Vec<String>, out: String) -> Result<Value, String> {
    crate::forge_pptx::build_pptx(&images, &out)
}

/// deck.html → 多页 .pptx 一步到位(逐页截图 + 纯 Rust 打包)。三平台同一份。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn forge_deck_to_pptx(
    deck: String,
    out: String,
    width: Option<u32>,
    height: Option<u32>,
    slides: Option<usize>,
) -> Result<Value, String> {
    crate::forge_pptx::render_deck_to_pptx(
        &deck,
        &out,
        width.unwrap_or(1920),
        height.unwrap_or(1080),
        slides,
    )
}

/// deck.html → .mp4(逐页截图 + ffmpeg 编码)。配音:audio=现成音频 / narration=文本走 TTS / 都无=无声。
#[cfg_attr(feature = "desktop", tauri::command)]
#[allow(clippy::too_many_arguments)]
pub fn forge_deck_to_video(
    deck: String,
    out: String,
    seconds_per_slide: Option<f64>,
    fps: Option<u32>,
    width: Option<u32>,
    height: Option<u32>,
    slides: Option<usize>,
    audio: Option<String>,
    narration: Option<String>,
) -> Result<Value, String> {
    crate::forge_video::render_deck_to_video(
        &deck,
        &out,
        seconds_per_slide.unwrap_or(3.0),
        fps.unwrap_or(30),
        width.unwrap_or(1920),
        height.unwrap_or(1080),
        slides,
        audio,
        narration,
    )
}

/// 文本 → mp3 配音(MiniMax T2A,纯 Rust)。无 key 时返回明确错误。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn forge_tts(
    text: String,
    out: String,
    voice: Option<String>,
    language_boost: Option<String>,
) -> Result<Value, String> {
    crate::forge_tts::synth(&text, &out, voice.as_deref(), language_boost.as_deref())
}

/// 用 chromium/chrome headless 给 URL/本地 HTML 截图(Forge capture 原始能力)。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn forge_screenshot(
    url: String,
    out: String,
    width: Option<u32>,
    height: Option<u32>,
) -> Result<Value, String> {
    crate::forge_pptx::screenshot(&url, &out, width.unwrap_or(1920), height.unwrap_or(1080))
}

/// 汇总当前环境出片的拦路项(给 UI 红灯直接展示)。
fn preflight_blockers(plat: &str, chromium: &Option<String>, ffmpeg: bool, cjk: Option<bool>) -> Vec<String> {
    let mut b = Vec::new();
    if (plat == "docker" || plat == "linux") && chromium.is_none() {
        b.push("缺 chromium：用 full 镜像(--build-arg POLARIS_RENDER=1)".to_string());
    }
    if (plat == "docker" || plat == "linux") && cjk != Some(true) {
        b.push("缺中文字体：装 fonts-noto-cjk，否则截图豆腐块".to_string());
    }
    if (plat == "docker" || plat == "linux") && !ffmpeg {
        b.push("缺 ffmpeg：出视频需 full 镜像".to_string());
    }
    b
}
