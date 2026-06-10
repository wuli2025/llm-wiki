//! Polaris Forge · 视频编码(FfmpegEncoder——跨平台 PRD §05 钦定的 Docker 主编码器/全平台逃生口)。
//!
//! deck.html → 逐页截图(复用 forge_pptx::capture_slides)→ ffmpeg 把图序列编成 .mp4。
//! 幻灯类低运动内容 x264 veryfast 绰绰有余,NAS 纯 CPU 可跑。首版出**无声片**(确定性、不需 key);
//! 配音(MiniMax / 字幕硬烧)是后续(TTS 模块)。架构文档的 openh264/MF/VideoToolbox 是「可选优化」
//! 后端,本版先把「能真出 mp4」这条主路打通并验证。
//!
//! ffmpeg 用 concat demuxer 读图+每图驻留 N 秒:稳、无需把图先转视频再拼。

use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;

fn ffmpeg_bin() -> String {
    std::env::var("POLARIS_FFMPEG")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "ffmpeg".to_string())
}

/// deck.html → .mp4(每页驻留 seconds_per_slide 秒)。三平台同一份(依赖镜像/系统的 ffmpeg)。
/// 配音:`audio`=现成音频文件直接 mux;否则 `narration`=文本走 MiniMax TTS 合成再 mux;都没有=无声。
pub fn render_deck_to_video(
    deck: &str,
    out_mp4: &str,
    seconds_per_slide: f64,
    fps: u32,
    width: u32,
    height: u32,
    slides_override: Option<usize>,
    audio: Option<String>,
    narration: Option<String>,
) -> Result<Value, String> {
    let secs = if seconds_per_slide > 0.0 { seconds_per_slide } else { 3.0 };
    let fps = if fps == 0 { 30 } else { fps };
    let (frames, pngs) = crate::forge_pptx::capture_slides(deck, width, height, slides_override)?;
    let n = pngs.len();

    // 配音解析:现成音频 > narration 文本走 TTS > 无。
    let mut audio_label = "none (无声)";
    let audio_path: Option<String> = if let Some(a) = audio.filter(|s| !s.is_empty()) {
        audio_label = "external";
        Some(a)
    } else if let Some(text) = narration.filter(|s| !s.trim().is_empty()) {
        let mp3 = frames.join("narration.mp3");
        match crate::forge_tts::synth(&text, &mp3.to_string_lossy(), None, None) {
            Ok(_) => {
                audio_label = "tts (MiniMax)";
                Some(mp3.to_string_lossy().to_string())
            }
            Err(e) => {
                // 配音失败不阻断出片:退化为无声(诚实告知)。
                audio_label = "none (TTS 失败，退无声)";
                eprintln!("[forge_video] TTS 失败，出无声版: {e}");
                None
            }
        }
    } else {
        None
    };

    let result = encode_images(&frames, &pngs, out_mp4, secs, fps, audio_path.as_deref());
    let _ = std::fs::remove_dir_all(&frames);
    result?;
    Ok(json!({
        "ok": true,
        "out": out_mp4,
        "slides": n,
        "seconds_per_slide": secs,
        "fps": fps,
        "duration_sec": secs * n as f64,
        "audio": audio_label
    }))
}

fn encode_images(
    frames_dir: &Path,
    pngs: &[String],
    out_mp4: &str,
    secs: f64,
    fps: u32,
    audio: Option<&str>,
) -> Result<(), String> {
    if pngs.is_empty() {
        return Err("没有帧可编码".into());
    }
    // concat demuxer 清单:每图一条 file + duration;最后一张需再列一次(concat 末帧时长怪癖)。
    let mut list = String::new();
    for p in pngs {
        let pp = p.replace('\\', "/").replace('\'', "");
        list.push_str(&format!("file '{pp}'\n"));
        list.push_str(&format!("duration {secs}\n"));
    }
    if let Some(last) = pngs.last() {
        let pp = last.replace('\\', "/").replace('\'', "");
        list.push_str(&format!("file '{pp}'\n"));
    }
    let list_path = frames_dir.join("frames.txt");
    std::fs::write(&list_path, list).map_err(|e| format!("写 concat 清单失败: {e}"))?;

    let mut args: Vec<String> = vec![
        "-y".into(),
        "-f".into(),
        "concat".into(),
        "-safe".into(),
        "0".into(),
        "-i".into(),
        list_path.to_string_lossy().to_string(),
    ];
    if let Some(a) = audio {
        args.push("-i".into());
        args.push(a.to_string());
    }
    args.extend([
        "-vsync".into(),
        "vfr".into(),
        // 偶数宽高(libx264/yuv420p 要求)+ 像素格式;幻灯多为偶数尺寸,这步兜底。
        "-vf".into(),
        "scale=trunc(iw/2)*2:trunc(ih/2)*2,format=yuv420p".into(),
        "-r".into(),
        fps.to_string(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "veryfast".into(),
    ]);
    if audio.is_some() {
        // 配音:AAC 音轨,-shortest 让成片随较短流收尾(避免拖尾黑屏/静音)。
        args.extend([
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "128k".into(),
            "-shortest".into(),
        ]);
    }
    args.extend(["-movflags".into(), "+faststart".into(), out_mp4.to_string()]);

    let status = Command::new(ffmpeg_bin())
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("启动 ffmpeg 失败(检查镜像是否 full / 系统是否装 ffmpeg): {e}"))?;
    if !status.success() || !Path::new(out_mp4).is_file() {
        return Err("ffmpeg 编码失败(未生成 mp4)".into());
    }
    Ok(())
}
