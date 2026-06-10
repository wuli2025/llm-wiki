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

/// 跑外部命令并设超时:超时则杀进程树返回 Err,防 chromium/ffmpeg/say 挂死永久阻塞整个请求
/// (「让模块再也不会有问题」的硬化——看门狗只管 claude,管不到这些 forge 子进程)。
/// 调用方传入已配好 args 的 Command(stdio 由本函数置 null)。成功且退出码 0 → Ok。
pub fn run_with_timeout(mut cmd: std::process::Command, secs: u64, what: &str) -> Result<(), String> {
    use std::io::{BufRead, BufReader};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    // 捕获 stderr(失败时带上,便于诊断「缺库/编解码器没装/字体缺失」等),stdout 仍丢弃。
    let mut child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("{what} 启动失败: {e}"))?;
    // 后台线程边读边截断 stderr:既排空管道防进程写满阻塞,又只留尾部 ~4KB 防 OOM。
    let errbuf = Arc::new(Mutex::new(String::new()));
    let reader_handle = child.stderr.take().map(|se| {
        let buf = errbuf.clone();
        std::thread::spawn(move || {
            let mut r = BufReader::new(se);
            let mut line = String::new();
            loop {
                line.clear();
                match r.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let mut b = buf.lock().unwrap();
                        b.push_str(&line);
                        if b.len() > 8000 {
                            let cut = b.len() - 4000;
                            *b = b[cut..].to_string();
                        }
                    }
                }
            }
        })
    });
    let deadline = Instant::now() + Duration::from_secs(secs);
    // 循环只决定结局,把 join/格式化挪到循环外做一次,避免 reader_handle 在循环里被 move。
    let outcome: Result<std::process::ExitStatus, String> = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait(); // kill 后管道关闭,reader 线程随之结束
                    break Err(format!("{what} 超时({secs}s)被终止"));
                }
                std::thread::sleep(Duration::from_millis(120));
            }
            Err(e) => break Err(format!("{what} 等待失败: {e}")),
        }
    };
    // 不 join reader 线程:被杀进程的子进程(如 chromium 的子代理/cmd 的 ping)可能仍持 stderr
    // 管道,join 会阻塞到它们退出。给 50ms 让常规 stderr 排空后读取(诊断 best-effort,绝不阻塞)。
    std::thread::sleep(Duration::from_millis(50));
    drop(reader_handle); // 分离线程,随管道关闭自行结束
    let errtail = {
        let s = errbuf.lock().unwrap().trim().to_string();
        if s.is_empty() {
            String::new()
        } else {
            format!(": {s}")
        }
    };
    match outcome {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("{what} 失败(退出码 {:?}){errtail}", status.code())),
        Err(msg) => Err(format!("{msg}{errtail}")),
    }
}

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

/// 应用自带二进制发现(零安装打包用):查 exe 同级 / bin / vendor,以及 macOS `.app/Contents/Resources`。
/// 让桌面 App 把 chromium/ffmpeg 打进包里 → 用户**什么都不用装**(objc2 原生后端之外的零安装正路)。
fn bundled_exe(names: &[&str]) -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.to_path_buf();
    let mut roots = vec![dir.clone(), dir.join("bin"), dir.join("vendor")];
    // macOS: exe 在 .app/Contents/MacOS/ → 资源在 ../Resources(及其 bin/)。
    if let Some(contents) = dir.parent() {
        roots.push(contents.join("Resources"));
        roots.push(contents.join("Resources").join("bin"));
    }
    for r in roots {
        for n in names {
            #[cfg(target_os = "windows")]
            {
                let pe = r.join(format!("{n}.exe"));
                if pe.is_file() {
                    return Some(pe.to_string_lossy().to_string());
                }
            }
            let p = r.join(n);
            if p.is_file() {
                return Some(p.to_string_lossy().to_string());
            }
        }
    }
    None
}

/// 找 chromium/chrome/edge 可执行: env → 应用自带(零安装打包)→ 平台候选名探测。
pub fn find_chromium() -> Option<String> {
    if let Ok(p) = std::env::var("POLARIS_CHROMIUM") {
        if !p.is_empty() && (Path::new(&p).is_file() || probe_exe(&p, "--version")) {
            return Some(p);
        }
    }
    // 应用自带的浏览器优先(零安装):打进包的 chromium / chrome-headless-shell。
    if let Some(p) = bundled_exe(&["chrome-headless-shell", "chromium", "chrome", "Chromium"]) {
        return Some(p);
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
    searchable: Option<bool>,
    slides: Option<usize>,
) -> Result<Value, String> {
    crate::forge_pptx::render_deck_to_pptx(
        &deck,
        &out,
        width.unwrap_or(1920),
        height.unwrap_or(1080),
        searchable.unwrap_or(true), // 默认开隐形文本层(可搜索 PPT=差异化卖点)
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
    transition: Option<f64>,
    motion: Option<bool>,
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
        transition,
        motion.unwrap_or(false),
    )
}

/// deck 某页 CSS 动画 → 逐帧真动画视频(__fx.seek + chromium 逐帧截图 + ffmpeg,无需 chromiumoxide)。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn forge_deck_fx_video(
    deck: String,
    out: String,
    fps: Option<u32>,
    duration_ms: Option<u64>,
    width: Option<u32>,
    height: Option<u32>,
    slide: Option<usize>,
) -> Result<Value, String> {
    crate::forge_video::render_deck_fx_video(
        &deck,
        &out,
        fps.unwrap_or(15),
        duration_ms.unwrap_or(2000),
        width.unwrap_or(1280),
        height.unwrap_or(720),
        slide.unwrap_or(1),
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
    scale: Option<u32>,
) -> Result<Value, String> {
    crate::forge_pptx::screenshot(
        &url,
        &out,
        width.unwrap_or(1920),
        height.unwrap_or(1080),
        scale.unwrap_or(2), // 默认 2x 高清
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    // 验证 run_with_timeout 真能在超时后杀掉挂死进程并快速返回(让模块再也不会有问题的核心)。
    #[cfg(target_os = "windows")]
    #[test]
    fn timeout_kills_hanging_process() {
        use std::process::Command;
        use std::time::Instant;
        // 成功路径:立刻退出 0。
        let mut ok = Command::new("cmd");
        ok.args(["/c", "exit", "0"]);
        assert!(run_with_timeout(ok, 5, "test-ok").is_ok());
        // 超时路径:ping -n 20(~19s)应被 1s 超时杀掉,且很快返回。
        let mut hang = Command::new("cmd");
        hang.args(["/c", "ping", "-n", "20", "127.0.0.1"]);
        let t = Instant::now();
        let r = run_with_timeout(hang, 1, "test-hang");
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("超时"));
        assert!(t.elapsed().as_secs() < 5, "超时后应快速返回,而非等满 19s");
        // 失败时 stderr 应进错误信息(可诊断)。
        let mut fail = Command::new("cmd");
        fail.args(["/c", "echo BOOMERR 1>&2 & exit 1"]);
        let e = run_with_timeout(fail, 5, "test-fail").unwrap_err();
        assert!(e.contains("BOOMERR"), "失败错误应含 stderr,实际: {e}");
    }

    #[test]
    fn bundled_exe_safe_when_absent() {
        // 测试环境 exe 旁没有自带 chromium/ffmpeg → 返回 None,不 panic(零安装打包发现逻辑)。
        assert!(bundled_exe(&["definitely-not-bundled-xyz"]).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn timeout_kills_hanging_process() {
        use std::process::Command;
        use std::time::Instant;
        assert!(run_with_timeout(Command::new("true"), 5, "test-ok").is_ok());
        let mut hang = Command::new("sleep");
        hang.arg("20");
        let t = Instant::now();
        let r = run_with_timeout(hang, 1, "test-hang");
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("超时"));
        assert!(t.elapsed().as_secs() < 5);
        // 失败时 stderr 应进错误信息(可诊断)。
        let mut fail = Command::new("sh");
        fail.args(["-c", "echo BOOMERR >&2; exit 1"]);
        let e = run_with_timeout(fail, 5, "test-fail").unwrap_err();
        assert!(e.contains("BOOMERR"), "失败错误应含 stderr,实际: {e}");
    }
}
