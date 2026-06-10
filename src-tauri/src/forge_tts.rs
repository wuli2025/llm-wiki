//! Polaris Forge · TTS(MiniMax T2A v2 配音,纯 Rust ureq——替掉 minimax-tts.mjs 的 Node 依赖)。
//!
//! 契约对齐既有 minimax-tts.mjs(已验证 sk-cp 粉丝福利 key 直通 T2A、无需 GroupId):
//!   POST https://api.minimaxi.com/v1/t2a_v2  Authorization: Bearer <key>
//!   body {model,text,stream:false,voice_setting,audio_setting{format:mp3}}  →  data.audio = hex(mp3)
//! key 发现顺序:env MINIMAX_API_KEY → ~/Polaris/data/providers.json 的 minimax 供应商 token。
//! 无 key 时返回明确错误(调用方据此降级到无声视频)。这是 TTS 阶梯 L0(主力);L1 edge-tts 等后续。

use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

const DEFAULT_ENDPOINT: &str = "https://api.minimaxi.com/v1/t2a_v2";
const DEFAULT_MODEL: &str = "speech-02-turbo";
const DEFAULT_VOICE: &str = "male-qn-qingse";

/// 发现 MiniMax key:env 优先,再读 Polaris 供应商坞存储(providers.json)。
pub fn discover_key() -> Option<String> {
    if let Ok(k) = std::env::var("MINIMAX_API_KEY") {
        if !k.trim().is_empty() {
            return Some(k.trim().to_string());
        }
    }
    let home = directories::UserDirs::new()?.home_dir().to_path_buf();
    let pj = home.join("Polaris").join("data").join("providers.json");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(pj).ok()?).ok()?;
    for it in v.get("items")?.as_array()? {
        let id = it.get("id").and_then(|x| x.as_str()).unwrap_or("");
        let name = it.get("name").and_then(|x| x.as_str()).unwrap_or("");
        if id == "minimax" || name.to_lowercase().contains("minimax") {
            if let Some(env) = it.get("settings_config").and_then(|s| s.get("env")) {
                for key in ["ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY", "MINIMAX_API_KEY"] {
                    if let Some(k) = env.get(key).and_then(|x| x.as_str()) {
                        if !k.trim().is_empty() {
                            return Some(k.trim().to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

fn hex_to_bytes(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.is_empty() || s.len() % 2 != 0 {
        return None;
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    let mut i = 0;
    while i < b.len() {
        let hi = (b[i] as char).to_digit(16)?;
        let lo = (b[i + 1] as char).to_digit(16)?;
        out.push(((hi << 4) | lo) as u8);
        i += 2;
    }
    Some(out)
}

/// 文本 → mp3 配音文件。voice/language_boost 可选(缺省男声青涩)。
pub fn synth(
    text: &str,
    out_mp3: &str,
    voice: Option<&str>,
    language_boost: Option<&str>,
) -> Result<Value, String> {
    // 有 key → MiniMax(L0 最佳);无 key → macOS 退系统 say 离线配音(L3,零安装),
    // 其余平台报错让调用方降级无声。
    if let Some(key) = discover_key() {
        return synth_minimax(text, out_mp3, voice, language_boost, &key);
    }
    // 无 key 兜底:每个 OS 上恰好一个 cfg 块被编译,作为函数尾表达式返回 Result(勿用 return/分号)。
    #[cfg(target_os = "macos")]
    {
        synth_macos_say(text, out_mp3)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("找不到 MiniMax key：在供应商坞启用「MiniMax」或设环境变量 MINIMAX_API_KEY（macOS 无 key 可走系统 say 离线配音）".to_string())
    }
}

/// macOS 系统内置 `say` → 离线配音(零安装,PRD 的 AvSpeech/L3 等价物;走系统 CLI，
/// 与 chromium/ffmpeg 同philosophy，不碰无法验证的 objc2 FFI)。say 不支持 mp3，
/// 输出 .m4a(AAC);ffmpeg mux 按内容识别，扩展名无所谓。
#[cfg(target_os = "macos")]
fn synth_macos_say(text: &str, out: &str) -> Result<Value, String> {
    let m4a = std::path::Path::new(out).with_extension("m4a");
    if let Some(p) = m4a.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let m4a_str = m4a.to_string_lossy().to_string();
    // 分次 .arg() 避免数组元素类型不齐;-o 指定输出文件,最后是要念的文本。
    let status = std::process::Command::new("say")
        .arg("-o")
        .arg(&m4a_str)
        .arg(text)
        .status()
        .map_err(|e| format!("say 启动失败: {e}"))?;
    if !status.success() || !m4a.is_file() {
        return Err("macOS say 合成失败".into());
    }
    let bytes = std::fs::metadata(&m4a).map(|m| m.len()).unwrap_or(0);
    Ok(json!({ "ok": true, "out": m4a_str, "engine": "macos-say", "bytes": bytes }))
}

fn synth_minimax(
    text: &str,
    out_mp3: &str,
    voice: Option<&str>,
    language_boost: Option<&str>,
    key: &str,
) -> Result<Value, String> {
    let endpoint = std::env::var("MINIMAX_T2A_URL").unwrap_or_else(|_| DEFAULT_ENDPOINT.to_string());
    let model = std::env::var("MINIMAX_TTS_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
    let voice = voice
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("MINIMAX_TTS_VOICE").ok())
        .unwrap_or_else(|| DEFAULT_VOICE.to_string());

    let mut body = json!({
        "model": model,
        "text": text,
        "stream": false,
        "voice_setting": { "voice_id": voice, "speed": 1, "vol": 1, "pitch": 0 },
        "audio_setting": { "sample_rate": 32000, "bitrate": 128000, "format": "mp3" }
    });
    if let Some(b) = language_boost.filter(|s| !s.is_empty()) {
        body["language_boost"] = json!(b);
    }

    let resp = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(60))
        .build()
        .post(&endpoint)
        .set("Authorization", &format!("Bearer {key}"))
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(|e| format!("T2A 请求失败: {e}"))?;
    let v: Value = resp
        .into_json()
        .map_err(|e| format!("T2A 响应解析失败: {e}"))?;
    let hex = v
        .get("data")
        .and_then(|d| d.get("audio"))
        .and_then(|a| a.as_str())
        .ok_or_else(|| {
            let msg = v
                .get("base_resp")
                .and_then(|b| b.get("status_msg"))
                .and_then(|m| m.as_str())
                .unwrap_or("响应无 data.audio");
            format!("T2A 无音频返回: {msg}")
        })?;
    let bytes = hex_to_bytes(hex).ok_or_else(|| "hex 音频解码失败".to_string())?;
    if let Some(parent) = Path::new(out_mp3).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(out_mp3, &bytes).map_err(|e| format!("写 mp3 失败: {e}"))?;
    Ok(json!({ "ok": true, "out": out_mp3, "bytes": bytes.len(), "voice": voice }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // macOS 系统 say 离线配音(无 key 兜底)。仅在 macOS 编译/运行——CI macos runner 自带 say,
    // 英文默认音色即可验证「真能产出音频」。这是 macOS 原生 TTS 路的运行时证据,无需 Mac 硬件。
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_say_offline_tts() {
        let out = std::env::temp_dir().join("forge_say_test.m4a");
        let _ = std::fs::remove_file(&out);
        let outp = out.to_string_lossy().to_string();
        let r = synth_macos_say("Hello from Polaris Forge on macOS.", &outp)
            .expect("say 应成功产出音频");
        assert_eq!(r["engine"], "macos-say");
        let p = std::path::Path::new(r["out"].as_str().unwrap());
        assert!(p.is_file(), "say 应产出 m4a 文件");
        assert!(std::fs::metadata(p).unwrap().len() > 0, "音频应非空");
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn hex_decode_roundtrip() {
        assert_eq!(hex_to_bytes("48656c6c6f"), Some(b"Hello".to_vec()));
        assert_eq!(hex_to_bytes("ff00a1"), Some(vec![0xff, 0x00, 0xa1]));
        assert_eq!(hex_to_bytes("abc"), None); // 奇数长度
        assert_eq!(hex_to_bytes("zz"), None); // 非 hex
    }
}
