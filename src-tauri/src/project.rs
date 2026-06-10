//! 板块⑮ —— 可运行项目 (Runnable Projects)
//!
//! 目标(用户原话): 把回答里生成的「前后端」打包项目化, 用户点一下就能启动整套项目、
//! 内嵌观看, 不用再把文件拖来拖去、也不用再说一句「打开这个项目」。
//!
//! 做法:
//! 1. 约定模型生成可运行项目时, 落成 **一个自带 `polaris.project.json` 清单的文件夹**
//!    (清单声明前端/后端各自怎么装依赖、怎么起、端口多少, 以及预览 URL)。见 chat.rs 的
//!    `project_convention` 指令。
//! 2. 本模块扫产物目录找出这些清单 → 列给前端; 点「运行」就 **一键** 装依赖 + 起前后端
//!    各服务进程, 把日志流式 emit 给前端, 端口起来后 emit `project:ready` 让前端内嵌 iframe 预览。
//! 3. 「停止」按钮 kill 整个进程树 (Windows 用 `taskkill /T /F`, 防 npm→node 子进程残留)。
//!
//! 一切进程都在宿主机本地跑(localhost), 内嵌 webview 的 CSP 为 null, 故 iframe 可直接加载
//! `http://localhost:<port>`。

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
#[cfg(feature = "desktop")]
use tauri::{AppHandle, Emitter};
#[cfg(not(feature = "desktop"))]
use crate::host::AppHandle;
use walkdir::WalkDir;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 给从 GUI 进程拉起的子进程加 `CREATE_NO_WINDOW`, 别每起一个服务就闪一个黑控制台。
fn no_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

/// 用系统 shell 跑一条命令行 (这样 `npm run dev` 这种走 npm.cmd / shell 内建的命令也能解析)。
fn shell_command(cmdline: &str) -> Command {
    #[cfg(windows)]
    {
        let mut c = Command::new("cmd");
        c.args(["/C", cmdline]);
        c
    }
    #[cfg(not(windows))]
    {
        let mut c = Command::new("sh");
        c.args(["-c", cmdline]);
        c
    }
}

// ───────────────────────── 清单 (polaris.project.json) ─────────────────────────

const MANIFEST_NAME: &str = "polaris.project.json";

#[derive(Debug, Clone, Deserialize)]
struct Manifest {
    #[serde(default)]
    name: Option<String>,
    /// 可选: 在 **项目根** 跑的一次性准备命令 (如建虚拟环境), 启动前依次 await。
    #[serde(default)]
    setup: Vec<String>,
    /// 各服务 (前端 / 后端 / …)。声明顺序即启动顺序。
    #[serde(default)]
    services: Vec<Service>,
    /// 预览 URL, 如 `http://localhost:5173`。前端起来后内嵌 iframe 加载它。
    #[serde(default)]
    open: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Service {
    #[serde(default)]
    name: Option<String>,
    /// 相对项目根的工作目录, 默认项目根。
    #[serde(default)]
    dir: Option<String>,
    /// 可选装依赖命令; **仅当该服务目录下 `node_modules` 缺失时** 才跑 (装过就跳过, 秒起)。
    #[serde(default)]
    install: Option<String>,
    /// 长驻命令, 如 `npm run dev` / `node server.js` / `python app.py`。
    run: String,
    /// 该服务监听端口; 填了就用它做「起来了没」的就绪探测。
    #[serde(default)]
    port: Option<u16>,
}

// ───────────────────────── 对外信息 (列给前端) ─────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    /// 项目根绝对路径 (正斜杠) —— 唯一标识, run/stop/status 都用它。
    pub root: String,
    pub name: String,
    /// 预览 URL (可能为空)。
    pub open: Option<String>,
    /// 是否正在运行。
    pub running: bool,
    /// 服务名列表 (展示用)。
    pub services: Vec<String>,
}

// ───────────────────────── 运行态注册表 ─────────────────────────

struct RunningProject {
    /// 各服务子进程 (持有以便 kill; 我们不 wait)。
    children: Vec<Child>,
    /// 各服务子进程 PID (Windows 下 taskkill 整树用)。
    pids: Vec<u32>,
}

/// root(正斜杠绝对路径) → 运行态。同一项目同一时刻只跑一份。
static RUNNING: Lazy<Mutex<HashMap<String, RunningProject>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

// ───────────────────────── 事件 ─────────────────────────

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectLogEvent {
    root: String,
    /// info(本应用旁白) | stdout | stderr
    stream: String,
    line: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectReadyEvent {
    root: String,
    open: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectExitEvent {
    root: String,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

fn log(app: &AppHandle, root: &str, stream: &str, line: impl Into<String>) {
    let _ = app.emit(
        "project:log",
        ProjectLogEvent {
            root: root.to_string(),
            stream: stream.to_string(),
            line: line.into(),
        },
    );
}

// ───────────────────────── 辅助 ─────────────────────────

fn norm(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn read_manifest(root: &Path) -> Result<Manifest, String> {
    let f = root.join(MANIFEST_NAME);
    let txt = std::fs::read_to_string(&f).map_err(|e| format!("读取项目清单失败: {}", e))?;
    serde_json::from_str::<Manifest>(&txt).map_err(|e| format!("项目清单格式有误: {}", e))
}

fn project_name(root: &Path, m: &Manifest) -> String {
    m.name.clone().unwrap_or_else(|| {
        root.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "项目".into())
    })
}

/// 从 open URL 抠出端口 (做就绪探测兜底用)。
fn port_from_url(url: &str) -> Option<u16> {
    let after = url.split("://").nth(1).unwrap_or(url);
    let hostport = after.split('/').next().unwrap_or(after);
    hostport.rsplit(':').next().and_then(|s| s.parse::<u16>().ok())
}

// ───────────────────────── 命令: 列项目 ─────────────────────────

/// 扫某会话产物目录, 找出全部带 `polaris.project.json` 的可运行项目。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn project_list(conversation_id: Option<String>) -> Vec<ProjectInfo> {
    let out_dir = crate::chat::artifacts_dir(conversation_id.as_deref());
    let mut found: Vec<ProjectInfo> = Vec::new();
    if !out_dir.exists() {
        return found;
    }
    let running = RUNNING.lock();
    // 限深度遍历, 找清单文件
    for w in WalkDir::new(&out_dir)
        .max_depth(5)
        .into_iter()
        .flatten()
    {
        if !w.file_type().is_file() || w.file_name() != MANIFEST_NAME {
            continue;
        }
        let root = match w.path().parent() {
            Some(p) => p.to_path_buf(),
            None => continue,
        };
        let m = match read_manifest(&root) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let root_s = norm(&root);
        let services: Vec<String> = m
            .services
            .iter()
            .enumerate()
            .map(|(i, s)| s.name.clone().unwrap_or_else(|| format!("服务{}", i + 1)))
            .collect();
        found.push(ProjectInfo {
            name: project_name(&root, &m),
            open: m.open.clone(),
            running: running.contains_key(&root_s),
            services,
            root: root_s,
        });
    }
    found
}

/// 单个项目是否正在运行。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn project_status(root: String) -> bool {
    RUNNING.lock().contains_key(&root)
}

// ───────────────────────── 命令: 运行 ─────────────────────────

/// 一键运行某项目: 装依赖(按需) → 起前后端各服务 → 就绪后 emit ready。
/// 立即返回, 真正的装/起在后台线程跑, 进度走 `project:log` / `project:ready` / `project:exit` 事件。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn project_run(app: AppHandle, root: String) -> Result<(), String> {
    if RUNNING.lock().contains_key(&root) {
        // 已在跑: 直接回报就绪, 前端据此加载 iframe。
        if let Ok(m) = read_manifest(Path::new(&root)) {
            let _ = app.emit(
                "project:ready",
                ProjectReadyEvent {
                    root: root.clone(),
                    open: m.open,
                },
            );
        }
        return Ok(());
    }
    let root_path = PathBuf::from(&root);
    if !root_path.join(MANIFEST_NAME).exists() {
        return Err("找不到项目清单 polaris.project.json".into());
    }
    let m = read_manifest(&root_path)?;
    if m.services.is_empty() {
        return Err("项目清单里没有声明任何可启动服务 (services)".into());
    }

    std::thread::spawn(move || {
        run_pipeline(&app, &root, &root_path, m);
    });
    Ok(())
}

/// 后台线程: 串行装依赖 → 起各服务 → 就绪探测 → emit ready。失败处处 emit exit。
fn run_pipeline(app: &AppHandle, root: &str, root_path: &Path, m: Manifest) {
    log(app, root, "info", "▶ 准备启动项目…");

    // 1. 项目根的一次性 setup 命令 (await)。
    for cmd in &m.setup {
        log(app, root, "info", format!("$ {}", cmd));
        if !run_blocking(app, root, root_path, cmd) {
            emit_exit(app, root, false, Some(format!("准备命令失败: {}", cmd)));
            return;
        }
    }

    // 2. 各服务按需装依赖 (node_modules 缺失才装), await。
    for (i, s) in m.services.iter().enumerate() {
        let svc_dir = service_dir(root_path, s);
        let label = s.name.clone().unwrap_or_else(|| format!("服务{}", i + 1));
        if let Some(install) = &s.install {
            let needs = !svc_dir.join("node_modules").exists();
            if needs {
                log(app, root, "info", format!("[{}] 装依赖: $ {}", label, install));
                if !run_blocking(app, root, &svc_dir, install) {
                    emit_exit(app, root, false, Some(format!("[{}] 装依赖失败", label)));
                    return;
                }
            } else {
                log(app, root, "info", format!("[{}] 依赖已就绪, 跳过安装", label));
            }
        }
    }

    // 3. 起各服务 (长驻), 注册进程, 流式日志。
    let mut children: Vec<Child> = Vec::new();
    let mut pids: Vec<u32> = Vec::new();
    for (i, s) in m.services.iter().enumerate() {
        let svc_dir = service_dir(root_path, s);
        let label = s.name.clone().unwrap_or_else(|| format!("服务{}", i + 1));
        log(app, root, "info", format!("[{}] 启动: $ {}", label, s.run));
        match spawn_service(&svc_dir, &s.run) {
            Ok(mut child) => {
                let pid = child.id();
                pids.push(pid);
                // 流式转发该服务 stdout/stderr → project:log
                if let Some(out) = child.stdout.take() {
                    pump(app.clone(), root.to_string(), format!("{}", label), out, "stdout");
                }
                if let Some(err) = child.stderr.take() {
                    pump(app.clone(), root.to_string(), format!("{}", label), err, "stderr");
                }
                children.push(child);
            }
            Err(e) => {
                // 起失败: kill 已起的, 收摊。用 kill_tree 连同孙进程(cmd /C npm → node)一起带走,
                // 裸 c.kill() 只杀直接子进程会漏掉 dev server 真正的 node 进程 → 端口占用泄漏。
                for mut c in children.drain(..) {
                    kill_tree(c.id());
                    let _ = c.kill();
                    let _ = c.wait();
                }
                emit_exit(app, root, false, Some(format!("[{}] 启动失败: {}", label, e)));
                return;
            }
        }
    }

    // 注册运行态 (供 stop / status / 防重复启动)。
    RUNNING.lock().insert(
        root.to_string(),
        RunningProject {
            children,
            pids: pids.clone(),
        },
    );

    // 4. 就绪探测: 优先用 open URL 的端口, 否则用第一个声明了 port 的服务。
    let probe_port = m
        .open
        .as_deref()
        .and_then(port_from_url)
        .or_else(|| m.services.iter().find_map(|s| s.port));

    if let Some(port) = probe_port {
        log(app, root, "info", format!("⏳ 等待端口 {} 就绪…", port));
        if wait_port(port, Duration::from_secs(120)) {
            log(app, root, "info", "✓ 服务已就绪, 正在打开预览");
        } else {
            log(
                app,
                root,
                "info",
                "⚠ 等待端口超时 (服务可能仍在编译, 可稍后手动刷新预览)",
            );
        }
    } else {
        // 没端口可探: 给个固定缓冲, 让 dev server 起来。
        std::thread::sleep(Duration::from_secs(3));
    }

    // 项目可能在探测期间被用户停掉了; 没停才报就绪。
    if RUNNING.lock().contains_key(root) {
        let _ = app.emit(
            "project:ready",
            ProjectReadyEvent {
                root: root.to_string(),
                open: m.open.clone(),
            },
        );
    }
}

fn service_dir(root_path: &Path, s: &Service) -> PathBuf {
    match s.dir.as_deref() {
        Some(d) if !d.is_empty() && d != "." => root_path.join(d),
        _ => root_path.to_path_buf(),
    }
}

fn emit_exit(app: &AppHandle, root: &str, ok: bool, message: Option<String>) {
    if let Some(msg) = &message {
        log(app, root, "info", format!("■ {}", msg));
    }
    let _ = app.emit(
        "project:exit",
        ProjectExitEvent {
            root: root.to_string(),
            ok,
            message,
        },
    );
}

/// 阻塞跑一条命令, 流式转发 stdout+stderr, 返回是否成功 (exit 0)。
fn run_blocking(app: &AppHandle, root: &str, cwd: &Path, cmdline: &str) -> bool {
    let mut cmd = shell_command(cmdline);
    cmd.current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    no_window(&mut cmd);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            log(app, root, "stderr", format!("无法执行: {} ({})", cmdline, e));
            return false;
        }
    };
    let mut handles = Vec::new();
    if let Some(out) = child.stdout.take() {
        handles.push(pump(app.clone(), root.to_string(), String::new(), out, "stdout"));
    }
    if let Some(err) = child.stderr.take() {
        handles.push(pump(app.clone(), root.to_string(), String::new(), err, "stderr"));
    }
    let status = child.wait();
    for h in handles {
        let _ = h.join();
    }
    matches!(status, Ok(s) if s.success())
}

/// 起一个长驻服务进程 (stdout/stderr piped 供流式日志)。
fn spawn_service(cwd: &Path, cmdline: &str) -> Result<Child, String> {
    let mut cmd = shell_command(cmdline);
    cmd.current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    no_window(&mut cmd);
    cmd.spawn().map_err(|e| e.to_string())
}

/// 起读线程: 把子进程一路输出按行 emit 成 project:log。label 非空时给行加 `[label]` 前缀。
fn pump<R: std::io::Read + Send + 'static>(
    app: AppHandle,
    root: String,
    label: String,
    reader: R,
    stream: &'static str,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let buf = BufReader::new(reader);
        for line in buf.lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let line = if label.is_empty() {
                line
            } else {
                format!("[{}] {}", label, line)
            };
            let _ = app.emit(
                "project:log",
                ProjectLogEvent {
                    root: root.clone(),
                    stream: stream.to_string(),
                    line,
                },
            );
        }
    })
}

/// 反复 TCP connect 127.0.0.1:port, 通了就算就绪; 超时返回 false。
fn wait_port(port: u16, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if TcpStream::connect_timeout(
            &(std::net::Ipv4Addr::LOCALHOST, port).into(),
            Duration::from_millis(800),
        )
        .is_ok()
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    false
}

// ───────────────────────── 命令: 停止 ─────────────────────────

/// 停止某项目: kill 整个进程树 (Windows taskkill /T /F, 其它平台 child.kill)。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn project_stop(app: AppHandle, root: String) -> Result<(), String> {
    let proj = RUNNING.lock().remove(&root);
    let Some(mut proj) = proj else {
        return Ok(()); // 本就没跑
    };
    for pid in proj.pids.drain(..) {
        kill_tree(pid);
    }
    for mut c in proj.children.drain(..) {
        let _ = c.kill();
        let _ = c.wait();
    }
    emit_exit(&app, &root, true, Some("项目已停止".into()));
    Ok(())
}

/// 按 PID kill 整个进程树。npm/vite 会拉起 node 子进程, 只 kill 父进程会留孤儿占着端口。
fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("taskkill");
        cmd.args(["/PID", &pid.to_string(), "/T", "/F"]);
        no_window(&mut cmd);
        let _ = cmd.output();
    }
    #[cfg(not(windows))]
    {
        // 杀进程组 (shell -c 起的子孙); 失败再退化为 kill 单进程。
        let _ = Command::new("kill")
            .args(["-TERM", &format!("-{}", pid)])
            .output()
            .or_else(|_| Command::new("kill").arg(pid.to_string()).output());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_parsing() {
        assert_eq!(port_from_url("http://localhost:5173"), Some(5173));
        assert_eq!(port_from_url("http://127.0.0.1:3001/app"), Some(3001));
        assert_eq!(port_from_url("http://localhost"), None); // 无端口
        assert_eq!(port_from_url("https://example.com/x"), None);
    }

    #[test]
    fn manifest_parses_directive_example() {
        // 与 chat.rs 的 project_convention 指令里给模型看的示例保持一致, 防两边漂移。
        let json = r#"{
          "name": "待办清单",
          "services": [
            { "name": "backend",  "dir": "server", "install": "npm install", "run": "node index.js", "port": 3001 },
            { "name": "frontend", "dir": "web",    "install": "npm install", "run": "npm run dev -- --port 5173", "port": 5173 }
          ],
          "open": "http://localhost:5173"
        }"#;
        let m: Manifest = serde_json::from_str(json).expect("应能解析示例清单");
        assert_eq!(m.name.as_deref(), Some("待办清单"));
        assert_eq!(m.open.as_deref(), Some("http://localhost:5173"));
        assert_eq!(m.services.len(), 2);
        assert_eq!(m.services[0].name.as_deref(), Some("backend"));
        assert_eq!(m.services[0].dir.as_deref(), Some("server"));
        assert_eq!(m.services[0].run, "node index.js");
        assert_eq!(m.services[1].port, Some(5173));
    }

    #[test]
    fn manifest_minimal_defaults() {
        // 最简: 只一个 service、无 dir/install/port/setup。
        let json = r#"{ "services": [ { "run": "python app.py" } ] }"#;
        let m: Manifest = serde_json::from_str(json).expect("最简清单应能解析");
        assert!(m.name.is_none());
        assert!(m.setup.is_empty());
        assert_eq!(m.services.len(), 1);
        assert!(m.services[0].dir.is_none());
        assert!(m.services[0].install.is_none());
        assert!(m.services[0].port.is_none());
    }
}
