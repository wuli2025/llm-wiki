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
    transition: Option<f64>,
) -> Result<Value, String> {
    let secs = if seconds_per_slide > 0.0 { seconds_per_slide } else { 3.0 };
    let fps = if fps == 0 { 30 } else { fps };
    // fail-fast:指定了配音文件但不存在 → 立刻报错,别白截完所有图再被 ffmpeg 拒(用户省事)。
    if let Some(a) = audio.as_deref().filter(|s| !s.is_empty()) {
        if !Path::new(a).is_file() {
            return Err(format!("指定的配音文件不存在: {a}"));
        }
    }
    // 视频用 1x(帧分辨率 = 目标 width×height,不膨胀编码量);高清交给分辨率参数控制。
    let (frames, pngs) =
        crate::forge_pptx::capture_slides(deck, width, height, 1, slides_override)?;
    let n = pngs.len();

    // 配音解析:现成音频 > narration 文本走 TTS > 无。
    let mut audio_label = "none (无声)";
    let audio_path: Option<String> = if let Some(a) = audio.filter(|s| !s.is_empty()) {
        audio_label = "external";
        Some(a)
    } else if let Some(text) = narration.filter(|s| !s.trim().is_empty()) {
        let mp3 = frames.join("narration.mp3");
        match crate::forge_tts::synth(&text, &mp3.to_string_lossy(), None, None) {
            Ok(res) => {
                // 实际音频路径以返回为准(macOS say 会落 .m4a 而非 .mp3)。
                let actual = res
                    .get("out")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| mp3.to_string_lossy().to_string());
                audio_label = match res.get("engine").and_then(|x| x.as_str()) {
                    Some("macos-say") => "tts (macOS say 离线)",
                    _ => "tts (MiniMax)",
                };
                Some(actual)
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

    let result = encode_images(&frames, &pngs, out_mp4, secs, fps, audio_path.as_deref(), transition);
    let _ = std::fs::remove_dir_all(&frames);
    result?;
    let dur = match transition {
        Some(t) if n > 1 => secs * n as f64 - (n as f64 - 1.0) * t.clamp(0.1, secs * 0.8),
        _ => secs * n as f64,
    };
    Ok(json!({
        "ok": true,
        "out": out_mp4,
        "slides": n,
        "seconds_per_slide": secs,
        "fps": fps,
        "duration_sec": dur,
        "transition": transition,
        "audio": audio_label
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_audio_fails_fast_before_capture() {
        // 指定不存在的配音文件应在截图之前就报错(无需 chromium),省掉无用截图。
        let r = render_deck_to_video(
            "any-deck.html",
            "/tmp/x.mp4",
            3.0,
            30,
            1280,
            720,
            None,
            Some("definitely-not-here.mp3".to_string()),
            None,
            None,
        );
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("配音文件不存在"));
    }
}

fn encode_images(
    frames_dir: &Path,
    pngs: &[String],
    out_mp4: &str,
    secs: f64,
    fps: u32,
    audio: Option<&str>,
    transition: Option<f64>,
) -> Result<(), String> {
    if pngs.is_empty() {
        return Err("没有帧可编码".into());
    }
    // 自动建 out 父目录(两路共用)。
    if let Some(parent) = Path::new(out_mp4).parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
    // 转场模式(架构文档§06④):页间交叉淡入。opt-in,多于 1 页才有意义。
    if let Some(t) = transition {
        if pngs.len() > 1 {
            return encode_xfade(pngs, out_mp4, secs, fps, audio, t);
        }
    }
    // ── 默认:concat 硬切 ──
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
        // 偶数宽高(libx264/yuv420p 要求)+ sRGB→BT.709 真矩阵转换(out_color_matrix)避免偏色发灰
        //(架构文档§06⑤);下面再打 BT.709 标签使矩阵与标签一致,规避 Remotion「只打标签不转换」的坑。
        "-vf".into(),
        "scale=trunc(iw/2)*2:trunc(ih/2)*2:out_color_matrix=bt709,format=yuv420p".into(),
        "-r".into(),
        fps.to_string(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "veryfast".into(),
        "-colorspace".into(),
        "bt709".into(),
        "-color_primaries".into(),
        "bt709".into(),
        "-color_trc".into(),
        "bt709".into(),
    ]);
    if audio.is_some() {
        // 配音:EBU R128 响度归一到 -16 LUFS(口播惯例,成片「专业感」来源——架构文档 §06)+
        // AAC 音轨;-shortest 让成片随较短流收尾(避免拖尾黑屏/静音)。
        args.extend([
            "-af".into(),
            "loudnorm=I=-16:TP=-1.5:LRA=11".into(),
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "128k".into(),
            "-shortest".into(),
        ]);
    }
    args.extend(["-movflags".into(), "+faststart".into(), out_mp4.to_string()]);

    let mut cmd = Command::new(ffmpeg_bin());
    cmd.args(&args);
    // 600s 超时:幻灯类低运动编码很快,纯 CPU 多页也够;挂死则杀掉防永久阻塞。
    crate::forge::run_with_timeout(cmd, 600, "ffmpeg 编码")?;
    if !Path::new(out_mp4).is_file() {
        return Err("ffmpeg 编码失败(未生成 mp4)".into());
    }
    Ok(())
}

/// 页间交叉淡入(xfade)编码:N 张图各驻留 secs 秒,相邻间 t 秒淡入淡出。
/// 总时长 = n*secs - (n-1)*t。架构文档§06④「页间转场」。
fn encode_xfade(
    pngs: &[String],
    out_mp4: &str,
    secs: f64,
    fps: u32,
    audio: Option<&str>,
    transition: f64,
) -> Result<(), String> {
    let n = pngs.len();
    let t = transition.clamp(0.1, secs * 0.8); // 转场不超过每页时长 80%
    let mut args: Vec<String> = vec!["-y".into()];
    for p in pngs {
        args.push("-loop".into());
        args.push("1".into());
        args.push("-t".into());
        args.push(format!("{secs}"));
        args.push("-i".into());
        args.push(p.clone());
    }
    if let Some(a) = audio {
        args.push("-i".into());
        args.push(a.to_string());
    }
    // filter_complex:每输入 scale+BT.709+fps+统一时基,再 xfade 链。
    let mut fc = String::new();
    for k in 0..n {
        fc.push_str(&format!(
            "[{k}:v]scale=trunc(iw/2)*2:trunc(ih/2)*2:out_color_matrix=bt709,format=yuv420p,fps={fps},settb=AVTB[s{k}];"
        ));
    }
    let mut prev = "s0".to_string();
    for k in 1..n {
        let offset = (k as f64) * (secs - t);
        let label = if k == n - 1 { "vout".to_string() } else { format!("x{k}") };
        fc.push_str(&format!(
            "[{prev}][s{k}]xfade=transition=fade:duration={t}:offset={offset}[{label}];"
        ));
        prev = label;
    }
    fc.pop(); // 去掉末尾 ;
    args.push("-filter_complex".into());
    args.push(fc);
    args.push("-map".into());
    args.push("[vout]".into());
    if audio.is_some() {
        args.push("-map".into());
        args.push(format!("{n}:a"));
        args.push("-af".into());
        args.push("loudnorm=I=-16:TP=-1.5:LRA=11".into());
        args.push("-c:a".into());
        args.push("aac".into());
        args.push("-b:a".into());
        args.push("128k".into());
        args.push("-shortest".into());
    }
    args.extend([
        "-r".into(),
        fps.to_string(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "veryfast".into(),
        "-colorspace".into(),
        "bt709".into(),
        "-color_primaries".into(),
        "bt709".into(),
        "-color_trc".into(),
        "bt709".into(),
        "-movflags".into(),
        "+faststart".into(),
        out_mp4.to_string(),
    ]);
    let mut cmd = Command::new(ffmpeg_bin());
    cmd.args(&args);
    crate::forge::run_with_timeout(cmd, 600, "ffmpeg xfade 编码")?;
    if !Path::new(out_mp4).is_file() {
        return Err("ffmpeg xfade 编码失败(未生成 mp4)".into());
    }
    Ok(())
}
