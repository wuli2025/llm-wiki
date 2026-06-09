//! 企业微信智能机器人「扫码自动配置」—— OAuth 回环模式。
//!
//! 背景：Tauri v2 在 webview 里无条件阻止弹窗创建（tauri#14263），企业微信官方
//! SDK 的 `window.open` + postMessage 在 Tauri webview 内跑不通；向外部页面注入
//! 脚本回传 Tauri IPC 又被 CSP 挡（tauri#8476）。
//!
//! 解法（同命令行工具 OAuth 回环）：
//!   1. Rust 起本地 HTTP 服务，提供一个内联了企业微信官方 SDK 的页面；
//!   2. 用系统浏览器打开该本地页（浏览器里 window.open+postMessage 原生支持）；
//!   3. 页面调 `openBotInfoAuthWindow` → 弹企业微信授权窗 → 扫码得 botid/secret；
//!   4. 页面把结果 POST 回本地 /result；
//!   5. Rust 收到后返回前端，自动填入表单。
//!
//! 这样彻底绕开 Tauri 弹窗限制。注：`source` 仍需向企业微信智能机器人平台登记，
//! 未登记扫完返回 ACCESS_DENIED —— 那是平台准入，非本流程能解决。

use serde::Serialize;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 企业微信官方浏览器 SDK（UMD，全局 `WecomAIBotSDK`），编译期内嵌、运行时离线可用。
const SDK_JS: &str = include_str!("../assets/wecom-aibot-sdk.umd.min.js");
const BASE_PORT: u16 = 52580;
const PORT_TRIES: u16 = 20;
const TIMEOUT_SECS: u64 = 300;
const IO_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WecomBotInfo {
    pub bot_id: String,
    pub secret: String,
}

fn bind_port() -> Result<(TcpListener, u16), String> {
    for off in 0..PORT_TRIES {
        let p = BASE_PORT + off;
        if let Ok(l) = TcpListener::bind(("127.0.0.1", p)) {
            return Ok((l, p));
        }
    }
    Err("无法绑定本地端口(52580–52599 都被占用)".into())
}

/// 本地授权页：内联官方 SDK + 调 openBotInfoAuthWindow + 结果 POST 回 /result。
fn page_html(source: &str) -> String {
    // source 安全转义进 JS 字符串字面量（防注入破坏页面）
    let safe = source
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('<', "")
        .replace('>', "");
    format!(
        r#"<!doctype html><html lang="zh"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>接入企业微信智能机器人</title><style>
html,body{{height:100%;margin:0;background:#0f1115;color:#e6e6e6;font-family:system-ui,'Microsoft YaHei',sans-serif}}
.wrap{{height:100%;display:flex;align-items:center;justify-content:center;padding:24px;box-sizing:border-box}}
.card{{background:#171a21;border:1px solid #262b36;border-radius:16px;padding:34px 40px;max-width:460px;text-align:center}}
h1{{font-size:17px;margin:0 0 16px}}
p{{font-size:13px;line-height:1.8;color:#9aa0a6;margin:8px 0}}
.ok{{color:#4ade80}} .err{{color:#f87171}}
.btn{{margin-top:16px;display:inline-block;padding:9px 20px;border-radius:9px;background:#3b6ef5;color:#fff;border:none;font-size:13px;cursor:pointer}}
</style></head><body><div class="wrap"><div class="card">
<h1>接入企业微信智能机器人</h1>
<p id="msg">点击下方按钮打开企业微信授权窗口，用企业微信扫码完成创建。</p>
<button class="btn" id="go">扫码创建机器人</button>
</div></div>
<script>{sdk}</script>
<script>
var SOURCE="{src}";
var msg=document.getElementById('msg'),go=document.getElementById('go');
function post(b){{return fetch('/result',{{method:'POST',headers:{{'Content-Type':'application/json'}},body:JSON.stringify(b)}});}}
async function run(){{
  go.disabled=true;go.textContent='等待扫码授权…';
  try{{
    var ns=window.WecomAIBotSDK||{{}};
    var sdk=ns.default||ns.sdk||(ns.openBotInfoAuthWindow?ns:null);
    if(!sdk||!sdk.openBotInfoAuthWindow){{throw new Error('企业微信 SDK 加载失败');}}
    var bot=await sdk.openBotInfoAuthWindow({{source:SOURCE,debug:true}});
    await post({{botid:bot.botid,secret:bot.secret}});
    msg.className='ok';msg.innerHTML='✅ 机器人创建成功，凭证已回传 Polaris。<br>可关闭此页面返回 Polaris。';
    go.style.display='none';
  }}catch(e){{
    var em=((e&&e.code)?e.code+': ':'')+((e&&e.message)||e);
    await post({{error:em}});
    msg.className='err';msg.innerHTML='创建失败：'+em+'<br>可关闭此页返回 Polaris。';
    go.disabled=false;go.textContent='重试';
  }}
}}
go.addEventListener('click',run);
</script></body></html>"#,
        sdk = SDK_JS,
        src = safe
    )
}

fn write_resp(stream: &mut TcpStream, status: &str, ctype: &str, body: &[u8]) {
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

enum Captured {
    Bot(WecomBotInfo),
    Err(String),
}

/// 处理一个连接；Some(_) 表示已捕获最终结果(应结束 accept 循环)。
fn handle(stream: &mut TcpStream, html: &str) -> Option<Captured> {
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let peer = stream.try_clone().ok()?;
    let mut reader = BufReader::new(peer);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).ok()? == 0 {
        return None;
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();

    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            break;
        }
        let t = line.trim_end();
        if t.is_empty() {
            break;
        }
        if let Some((k, v)) = t.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse().unwrap_or(0);
            }
        }
    }

    let path_only = path.split('?').next().unwrap_or("");
    if method == "POST" && path_only == "/result" {
        let mut body = vec![0u8; content_length.min(8192)];
        let _ = reader.read_exact(&mut body);
        write_resp(stream, "200 OK", "text/plain; charset=utf-8", b"ok");
        let v: serde_json::Value =
            serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
        if let Some(err) = v.get("error").and_then(|x| x.as_str()) {
            return Some(Captured::Err(err.to_string()));
        }
        let bot_id = v.get("botid").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let secret = v.get("secret").and_then(|x| x.as_str()).unwrap_or("").to_string();
        if bot_id.is_empty() {
            return Some(Captured::Err("回传缺少 botid".into()));
        }
        return Some(Captured::Bot(WecomBotInfo { bot_id, secret }));
    }
    if method == "GET" && (path_only == "/" || path_only.is_empty()) {
        write_resp(stream, "200 OK", "text/html; charset=utf-8", html.as_bytes());
        return None;
    }
    write_resp(stream, "404 Not Found", "text/plain", b"not found");
    None
}

fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        Command::new("cmd")
            .args(["/C", "start", "", url])
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(url).spawn().map_err(|e| e.to_string())?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open").arg(url).spawn().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 扫码自动配置：起本地回环服务 + 开系统浏览器，阻塞等待扫码结果（最多 5 分钟）。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn wecom_scan_create(source: String) -> Result<WecomBotInfo, String> {
    let src = if source.trim().is_empty() {
        "polaris-ai".to_string()
    } else {
        source.trim().to_string()
    };
    let (listener, port) = bind_port()?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let html = page_html(&src);

    let (tx, rx) = mpsc::channel::<Captured>();
    let running = Arc::new(AtomicBool::new(true));
    let r2 = running.clone();
    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(TIMEOUT_SECS + 10);
        while r2.load(Ordering::Relaxed) && Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    if let Some(c) = handle(&mut stream, &html) {
                        let _ = tx.send(c);
                        break;
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(120));
                }
                Err(_) => break,
            }
        }
    });

    open_browser(&format!("http://127.0.0.1:{port}/"))?;

    let out = rx.recv_timeout(Duration::from_secs(TIMEOUT_SECS));
    running.store(false, Ordering::Relaxed);
    match out {
        Ok(Captured::Bot(b)) => Ok(b),
        Ok(Captured::Err(e)) => Err(e),
        Err(_) => Err("扫码超时或已取消（5 分钟内未完成授权）".into()),
    }
}
