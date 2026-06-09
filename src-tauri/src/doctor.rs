//! 板块 ⑦ 环境医生 (Environment Doctor) — 新用户开箱的「环境监测 + 配置安装」
//!
//! 设计目标 (PRD: 新用户点开软件应先过一道环境关):
//! - **监测**: Claude Code (`claude.exe`) 与 PowerShell 7 (`pwsh`) 是否就绪;
//!   附带 Node.js / npm (Claude Code 的可选安装路径) 的探测。
//! - **安装**: Claude Code 没装时一键安装 —— 默认走 **npm + 国内镜像**
//!   `npm i -g @anthropic-ai/claude-code --registry=https://registry.npmmirror.com`:
//!   该包的原生二进制经 `optionalDependencies` (`@anthropic-ai/claude-code-win32-x64`)
//!   同源镜像分发, postinstall 只是把它拷成 `bin/claude.exe` —— 整个安装不碰 claude.ai / GCS,
//!   故**国内可装**。装出的是真·原生 `claude.exe`, chat.rs 解析其全路径直接 spawn。
//!   官方原生脚本 `irm https://claude.ai/install.ps1 | iex` 改作兜底 (国内常被墙, 故不再首选)。
//!   npm 方式需要 Node.js —— 缺失时用 winget 装 Node; PowerShell 7 缺失时同样用 winget。
//! - **改环境变量 (关键)**: Windows 上原生安装把 `claude.exe` 落到
//!   `~/.local/bin`, 但该目录常不在 PATH —— 不修则装了也找不到。这里
//!   **双写**: ① 持久化进「用户 PATH」(注册表, `[Environment]::SetEnvironmentVariable`,
//!   会广播 WM_SETTINGCHANGE), 让以后开的终端/重启后的 app 都能找到;
//!   ② 立刻塞进**当前进程 PATH** (`std::env::set_var`), 让本次会话不重启即可
//!   spawn claude。安装成功后自动执行, 对应「你帮他配置一下 / 一定要记得改环境变量」。
//!
//! 跨平台: 探测两端通用 (Windows 走 where.exe / cmd, 类 Unix 走 which / 直接执行)。
//! 安装 Claude Code **两端默认一致走 npm+npmmirror**: 原生二进制 (win32 / darwin-arm64 /
//! darwin-x64 …) 经 optionalDependencies 由 npmmirror 同源镜像分发, 安装不碰 claude.ai/GCS,
//! 故国内 (含 macOS) 可装; 官方原生脚本 (install.ps1 / install.sh) 因从 claude.ai 拉二进制、
//! 国内常被墙, 仅作「境外网络」兜底。npm 方式需要 Node.js —— 缺失时:
//! Windows 用 winget / 官方 MSI 装, **macOS 免 sudo 下载官方 darwin tar.gz (npmmirror 镜像)**
//! 解压到 `~/.local/polaris-node` 并写 shell 配置。经 `build_install_shell` 选 PowerShell 或 sh。
//! 持久化 PATH: Windows 写注册表用户 PATH; macOS·Linux 写 `~/.zshrc` 等 shell 配置。

use parking_lot::Mutex;
use serde::Serialize;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
#[cfg(feature = "desktop")]
use tauri::{AppHandle, Emitter};
#[cfg(not(feature = "desktop"))]
use crate::host::AppHandle;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 给从 GUI 进程拉起的子进程加 `CREATE_NO_WINDOW`, 免得每次探测都闪一个黑色控制台窗口。
#[cfg_attr(not(windows), allow(unused_variables))]
fn no_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

// ───────────────────────── 视图模型 ─────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    /// 稳定标识: claude | pwsh | node | npm
    pub key: String,
    /// 展示名
    pub name: String,
    /// 是否在机器上找到 (PATH 命中或已知安装位置存在)
    pub found: bool,
    /// 版本号 (探测到才有)
    pub version: Option<String>,
    /// 解析到的可执行文件路径 (正斜杠)
    pub path: Option<String>,
    /// 是否能通过 PATH 直接发现 (即终端里敲命令能用)
    pub on_path: bool,
    /// 是否是「必须」(claude 必须; 其余推荐)
    pub required: bool,
    /// 一句话状态说明 / 安装建议
    pub hint: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvReport {
    /// "windows" | "macos" | "linux" ...
    pub os: String,
    pub claude: ToolStatus,
    pub pwsh: ToolStatus,
    pub node: ToolStatus,
    pub npm: ToolStatus,
    /// claude.exe 应在 / 已在的目录 (用于「修复 PATH」)
    pub claude_dir: Option<String>,
    /// 该目录是否已在「用户 PATH」里 (Windows)。false ⇒ 需要修复
    pub claude_dir_on_user_path: bool,
    /// 是否有 claude 可用的 shell —— 真身 PowerShell 7 (非 Store 别名) 或 Git Bash。
    /// false ⇒ 即便装了 claude, 对话里也会报「找不到 PowerShell / bash」。
    pub shell_ready: bool,
    /// 整体是否就绪 (claude 已装 **且** 有可用 shell 才算真能跑起来)
    pub ready: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathFixResult {
    pub ok: bool,
    /// 实际加入 PATH 的目录
    pub dir: Option<String>,
    /// "added" | "present" | "process_only" | "skipped"
    pub status: String,
    pub message: String,
}

// ───────────────────────── 流式事件 ─────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvStreamEvent {
    pub req_id: String,
    /// "log" | "error" | "done"
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
    /// done 时: 是否成功
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    /// done 时: 收尾说明 (含 PATH 配置结果)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

static CHILDREN: once_cell::sync::Lazy<Arc<Mutex<HashMap<String, Child>>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));
static REQ_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_req_id() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let c = REQ_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("env-{:x}-{:x}", ts, c)
}

// ───────────────────────── 探测原语 ─────────────────────────

fn home_dir() -> Option<PathBuf> {
    directories::UserDirs::new().map(|u| u.home_dir().to_path_buf())
}

fn to_fwd(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// 跑一个子命令, 最多等 `timeout`; 超时则 kill 子进程并返回 None。
/// 探测类调用 (npm view / npm prefix / 版本号 / where / 读注册表 PATH 等) 用它兜底:
/// 网络卡死或进程僵死时不让 `env_check` 这条同步 Tauri 命令永久阻塞。
/// stdout/stderr 各由独立线程读到 EOF, 避免子进程写满管道反压自锁。
fn output_with_timeout(mut cmd: Command, timeout: Duration) -> Option<std::process::Output> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().ok()?;
    let mut out_pipe = child.stdout.take()?;
    let mut err_pipe = child.stderr.take()?;
    let (tx_o, rx_o) = std::sync::mpsc::channel();
    let (tx_e, rx_e) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = std::io::Read::read_to_end(&mut out_pipe, &mut b);
        let _ = tx_o.send(b);
    });
    std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = std::io::Read::read_to_end(&mut err_pipe, &mut b);
        let _ = tx_e.send(b);
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None; // 超时: 杀掉并放弃
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return None,
        }
    };
    let stdout = rx_o.recv().unwrap_or_default();
    let stderr = rx_e.recv().unwrap_or_default();
    Some(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

/// 用 `where.exe`(Windows) / `which`(unix) 找出某命令的全部命中路径 (存在的才留)。
fn which_all(bin: &str) -> Vec<PathBuf> {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("where.exe");
        c.arg(bin);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new("which");
        c.args(["-a", bin]);
        c
    };
    cmd.stdin(Stdio::null());
    no_window(&mut cmd);
    let out = match output_with_timeout(cmd, Duration::from_secs(20)) {
        Some(o) => o,
        None => return Vec::new(),
    };
    if !out.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect()
}

/// 取某命令的版本号。Windows 走 `cmd /c <bin> <args>` 以便正确解析 .exe/.cmd (PATHEXT);
/// 其余平台直接执行。返回首个非空行 (去掉前后空白)。
fn probe_version(bin: &str, args: &[&str]) -> Option<String> {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("cmd");
        let mut full = vec!["/c".to_string(), bin.to_string()];
        full.extend(args.iter().map(|s| s.to_string()));
        c.args(full);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new(bin);
        c.args(args);
        c
    };
    cmd.stdin(Stdio::null());
    no_window(&mut cmd);
    let out = output_with_timeout(cmd, Duration::from_secs(20))?;
    let pick = |bytes: &[u8]| -> Option<String> {
        String::from_utf8_lossy(bytes)
            .lines()
            .map(|l| l.trim())
            .find(|l| !l.is_empty())
            .map(|s| s.to_string())
    };
    if out.status.success() {
        // 优先 stdout, 个别工具把版本写到 stderr
        pick(&out.stdout).or_else(|| pick(&out.stderr))
    } else {
        None
    }
}

/// npm 全局安装前缀。走 `npm prefix -g` —— **用户可能改过前缀**(实测有人放在 `D:\Users\x\npm`,
/// 而非默认 `%APPDATA%\npm`), 硬编码默认值会漏掉。失败 / 目录不存在 → None。
fn npm_global_prefix() -> Option<PathBuf> {
    #[cfg(windows)]
    let mut cmd = {
        // 经 cmd /c 以便解析 npm.cmd (CreateProcessW 不认 .cmd)
        let mut c = Command::new("cmd");
        c.args(["/c", "npm", "prefix", "-g"]);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new("npm");
        c.args(["prefix", "-g"]);
        c
    };
    cmd.stdin(Stdio::null());
    no_window(&mut cmd);
    let out = output_with_timeout(cmd, Duration::from_secs(20))?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())?
        .to_string();
    let p = PathBuf::from(line);
    p.exists().then_some(p)
}

/// 某个 npm 全局前缀下 Claude Code 的**真·原生 exe** 路径
/// (`<prefix>/node_modules/@anthropic-ai/claude-code/bin/claude.exe`)。
/// postinstall 把平台二进制拷到这里; 这是可被 `Command::new` 直接 spawn 的目标,
/// 而 `<prefix>/claude.cmd` 只是调它的 shim。
fn npm_claude_native_exe(prefix: &std::path::Path) -> PathBuf {
    prefix
        .join("node_modules")
        .join("@anthropic-ai")
        .join("claude-code")
        .join("bin")
        .join("claude.exe")
}

/// 已知的 claude 可执行文件候选位置。原生 `.exe` 优先 (能直接 spawn),
/// npm 的 `claude.cmd` shim 仅作探测 / PATH 兜底。
fn claude_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(h) = home_dir() {
        // 官方原生脚本: ~/.local/bin/claude(.exe)
        v.push(h.join(".local").join("bin").join("claude.exe"));
        v.push(h.join(".local").join("bin").join("claude"));
        // macOS 免 sudo 装的 Node (~/.local/polaris-node) 的全局 bin: `npm i -g` 把 claude
        // 链到这里。mac GUI 从 Finder 启动时 PATH 极简、`npm prefix -g` 又拿不到 → 显式兜底,
        // 让重启后 chat spawn 仍找得到。
        v.push(h.join(".local").join("polaris-node").join("bin").join("claude"));
    }
    // npm 全局 (用户真实前缀): 先原生 exe, 再 shim
    if let Some(prefix) = npm_global_prefix() {
        v.push(npm_claude_native_exe(&prefix));
        v.push(prefix.join("claude.exe"));
        v.push(prefix.join("claude.cmd"));
    }
    // 默认前缀兜底 (拿不到 `npm prefix -g` 时, 例如 npm 不在 PATH)
    if let Some(h) = home_dir() {
        let appdata_npm = h.join("AppData").join("Roaming").join("npm");
        v.push(npm_claude_native_exe(&appdata_npm));
        v.push(appdata_npm.join("claude.cmd"));
        v.push(appdata_npm.join("claude.exe"));
    }
    v
}

/// chat.rs spawn 用的解析结果缓存 —— 避免每次发消息都跑 `where.exe` / `npm prefix -g`。
/// 安装成功后 (`stream_install`) 会清空, 下次重新解析。
static CLAUDE_EXE_CACHE: once_cell::sync::Lazy<Mutex<Option<PathBuf>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

/// 子进程环境净化（借鉴 OpenCode 桌面端 sidecar：loopback 强制 NO_PROXY + 清干扰变量）。
/// 给将要 spawn 的 **宿主机 claude** 子进程套上两层防护：
///
/// ① **回环并进 NO_PROXY**：切到 Codex 时 claude 的 `ANTHROPIC_BASE_URL` 被指向
///    `http://127.0.0.1:{port}`（见 provider.rs 的 `codex_route_config`）。若用户配了系统级
///    `HTTP(S)_PROXY`（国内/企业网常见），claude 底层 HTTP 客户端会把这个 **loopback 请求也走代理**
///    → 连不上本地翻译代理、报「连接 ChatGPT 后端失败」。把回环列入 `NO_PROXY`/`no_proxy` 即绕开代理直连。
///    只补回环、不动其他代理设置 —— 代理本身（claude 直连远端 API 时要用）照常生效。
/// ② **清干扰继承变量**：`DEBUG`（让 Node 生态吐调试噪声、行为不可预测）；Linux 的 `LD_PRELOAD`（注入）。
pub fn harden_child_env(cmd: &mut Command) {
    for key in ["NO_PROXY", "no_proxy"] {
        let current = std::env::var(key).unwrap_or_default();
        cmd.env(key, merge_no_proxy(&current));
    }
    cmd.env_remove("DEBUG");
    #[cfg(target_os = "linux")]
    cmd.env_remove("LD_PRELOAD");
}

/// 把回环主机（`127.0.0.1` / `localhost` / `::1`）并进既有 NO_PROXY 值：
/// 保留用户原有条目、大小写不敏感去重、已存在则不重复添加。抽纯函数便于单测。
pub fn merge_no_proxy(current: &str) -> String {
    let mut items: Vec<String> = current
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    for host in ["127.0.0.1", "localhost", "::1"] {
        if !items.iter().any(|v| v.eq_ignore_ascii_case(host)) {
            items.push(host.to_string());
        }
    }
    items.join(",")
}

/// 解析一个「可直接 spawn」的 claude 可执行文件全路径, 供 chat.rs 调起宿主机 CLI。
///
/// 为什么不让 chat.rs 用裸名 `Command::new("claude")`: Windows 的 `CreateProcessW` 解析裸名时
/// 只补 `.exe`、不查 PATHEXT, 而 **npm 装只在 PATH 放 `claude.cmd`** → 裸名根本找不到。
/// 这里偏好真·原生 `.exe` (PATH 命中的 .exe → 已知候选里的 .exe), 实在没有才回退到 `.cmd`;
/// 全部落空返回 None, 让调用方退回裸名靠 PATH。带进程内缓存。
pub fn resolve_claude_exe() -> Option<PathBuf> {
    // 命中缓存且文件仍在 → 直接用
    if let Some(p) = CLAUDE_EXE_CACHE.lock().as_ref() {
        if p.exists() {
            return Some(p.clone());
        }
    }
    let resolved = resolve_claude_exe_uncached();
    *CLAUDE_EXE_CACHE.lock() = resolved.clone();
    resolved
}

fn resolve_claude_exe_uncached() -> Option<PathBuf> {
    let is_exe = |p: &std::path::Path| {
        p.extension()
            .map(|e| e.eq_ignore_ascii_case("exe"))
            .unwrap_or(false)
    };
    let hits = which_all("claude"); // 已过滤为「存在的」路径
    // 1. PATH 命中里的 .exe (原生装常见)
    if let Some(p) = hits.iter().find(|p| is_exe(p)) {
        return Some(p.clone());
    }
    // 2. 已知候选里存在的 .exe (npm 装 → node_modules 里的原生 exe)
    let cands = claude_candidates();
    if let Some(p) = cands.iter().find(|p| is_exe(p) && p.exists()) {
        return Some(p.clone());
    }
    // 3. 退而求其次: 任意 PATH 命中 / 存在候选 (可能是 .cmd)
    hits.into_iter()
        .next()
        .or_else(|| cands.into_iter().find(|p| p.exists()))
}

fn pwsh_candidates() -> Vec<PathBuf> {
    vec![
        PathBuf::from(r"C:\Program Files\PowerShell\7\pwsh.exe"),
        PathBuf::from(r"C:\Program Files\PowerShell\7-preview\pwsh.exe"),
    ]
}

/// Node.js 可执行文件所在目录候选 (放 `node`/`npm`)。装完 Node 后用它把目录塞进进程 PATH,
/// 让同一会话紧接着的 npm/claude 安装立刻找得到 npm (免重启 app)。
fn node_dir_candidates() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        vec![
            PathBuf::from(r"C:\Program Files\nodejs"),
            PathBuf::from(r"C:\Program Files (x86)\nodejs"),
        ]
    }
    #[cfg(not(windows))]
    {
        let mut v = Vec::new();
        if let Some(h) = home_dir() {
            // 本应用免 sudo 装 Node 的落脚处 (见 MAC_NODE_INSTALL_SCRIPT), 优先
            v.push(h.join(".local").join("polaris-node").join("bin"));
            v.push(h.join(".local").join("bin"));
        }
        // Homebrew (Apple Silicon / Intel) 与系统常见位置
        v.push(PathBuf::from("/opt/homebrew/bin"));
        v.push(PathBuf::from("/usr/local/bin"));
        v
    }
}

/// Windows「应用执行别名」空壳: `%LOCALAPPDATA%\Microsoft\WindowsApps\` 下的 0 字节重解析点
/// (从 Microsoft Store 装 PowerShell 7 / Python 等会留下)。交互式终端里它能转发到 Store 真身,
/// 但**本应用是 GUI 进程、以 CREATE_NO_WINDOW 无控制台方式 spawn claude**, claude 再去拉这个
/// 别名时在该上下文下起不来 → 报「找不到 PowerShell」。故探测时把它当「没装」, 引导装
/// Program Files 里的真身 (普通 exe, 任何子进程都能稳定 spawn) 替代。
fn is_app_exec_alias(p: &std::path::Path) -> bool {
    #[cfg(windows)]
    {
        let in_windows_apps = p
            .components()
            .any(|c| c.as_os_str().to_string_lossy().eq_ignore_ascii_case("WindowsApps"));
        if !in_windows_apps {
            return false;
        }
        // 0 字节 = 典型的执行别名占位 (reparse point), 不是真二进制
        std::fs::metadata(p).map(|m| m.len() == 0).unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        let _ = p;
        false
    }
}

/// 探测可用的 Git Bash (claude 在 Windows 上可接受的另一种 shell)。
/// 先认 `CLAUDE_CODE_GIT_BASH_PATH` 覆盖, 再扫常见安装位置。
/// 仅 Windows 需要 (扫的全是 Windows 路径); 类 Unix 用系统自带 shell, 不走这里。
#[cfg(windows)]
fn git_bash_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("CLAUDE_CODE_GIT_BASH_PATH") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    [
        r"C:\Program Files\Git\bin\bash.exe",
        r"C:\Program Files\Git\usr\bin\bash.exe",
        r"C:\Program Files (x86)\Git\bin\bash.exe",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.exists())
}

/// 通用工具探测: which 命中 + 已知候选, 取首个可用; on_path = 是否被 PATH 发现。
fn detect(
    key: &str,
    name: &str,
    bin: &str,
    version_args: &[&str],
    candidates: &[PathBuf],
    required: bool,
    install_hint: &str,
) -> ToolStatus {
    // 滤掉 WindowsApps 的执行别名空壳 —— 它对无控制台 spawn 的 claude 不可用, 不能算「已装」
    let on_path_hits: Vec<PathBuf> = which_all(bin)
        .into_iter()
        .filter(|p| !is_app_exec_alias(p))
        .collect();
    let on_path = !on_path_hits.is_empty();

    // 解析出一个具体路径: PATH 命中优先 (Windows 偏好 .exe), 否则用存在的候选
    let resolved: Option<PathBuf> = {
        // 偏好 .exe 命中 (chat.rs 的 Command::new 在 Windows 只认 .exe)
        let exe_hit = on_path_hits
            .iter()
            .find(|p| {
                p.extension()
                    .map(|e| e.eq_ignore_ascii_case("exe"))
                    .unwrap_or(false)
            })
            .cloned();
        exe_hit
            .or_else(|| on_path_hits.first().cloned())
            .or_else(|| candidates.iter().find(|p| p.exists()).cloned())
    };

    let found = resolved.is_some();
    let version = if found {
        probe_version(bin, version_args)
    } else {
        None
    };

    let hint = if found {
        match &version {
            Some(v) => v.clone(),
            None => "已安装".to_string(),
        }
    } else {
        install_hint.to_string()
    };

    ToolStatus {
        key: key.to_string(),
        name: name.to_string(),
        found,
        version,
        path: resolved.as_deref().map(to_fwd),
        on_path,
        required,
        hint,
    }
}

// ───────────────────────── 用户 PATH (Windows) ─────────────────────────

/// 读「用户级 PATH」(注册表 HKCU\Environment), 经 PowerShell .NET API 拿。
#[cfg(windows)]
fn read_user_path() -> Option<String> {
    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "[Environment]::GetEnvironmentVariable('Path','User')",
    ]);
    cmd.stdin(Stdio::null());
    no_window(&mut cmd);
    let out = output_with_timeout(cmd, Duration::from_secs(20))?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(not(windows))]
fn read_user_path() -> Option<String> {
    None
}

/// dir 是否(忽略大小写/尾斜杠)出现在分号分隔的 PATH 串里。
fn path_contains_dir(path_str: &str, dir: &str) -> bool {
    let norm = |s: &str| s.trim().trim_end_matches(['\\', '/']).to_lowercase();
    let target = norm(dir);
    if target.is_empty() {
        return false;
    }
    path_str.split(';').any(|p| norm(p) == target)
}

/// 把 dir 前插进**当前进程 PATH** (若尚不在其中)。仅改本进程、不碰注册表; 返回是否真加了。
/// 这是「装完 / 启动后不重启即可 spawn claude」的底座 —— 子进程继承本进程 env。
fn prepend_process_path(dir: &str) -> bool {
    let dir = dir.trim();
    if dir.is_empty() {
        return false;
    }
    let proc_path = std::env::var("PATH").unwrap_or_default();
    if path_contains_dir(&proc_path, dir) {
        return false;
    }
    let sep = if cfg!(windows) { ';' } else { ':' };
    let new = if proc_path.is_empty() {
        dir.to_string()
    } else {
        format!("{dir}{sep}{proc_path}")
    };
    std::env::set_var("PATH", new);
    true
}

/// 把 dir 追加进「用户 PATH」(持久化, 注册表) + 当前进程 PATH (立即生效)。
/// Windows 专属; 其余平台仅尝试改进程 PATH。
fn ensure_dir_on_path(dir: &str) -> PathFixResult {
    let dir = dir.trim();
    if dir.is_empty() || !PathBuf::from(dir).exists() {
        return PathFixResult {
            ok: false,
            dir: Some(dir.to_string()),
            status: "skipped".into(),
            message: "目标目录不存在, 无法加入 PATH (请先安装)。".into(),
        };
    }

    // ① 当前进程 PATH (prepend → 本次会话立即能 spawn claude, 无需重启 app)
    prepend_process_path(dir);

    // ② 用户级持久化 PATH (Windows)。用显式 return 收尾, 避免 cfg 块尾表达式歧义。
    #[cfg(windows)]
    {
        if let Some(user_path) = read_user_path() {
            if path_contains_dir(&user_path, dir) {
                return PathFixResult {
                    ok: true,
                    dir: Some(dir.to_string()),
                    status: "present".into(),
                    message: format!("{dir} 已在用户 PATH 中 (进程 PATH 也已同步)。"),
                };
            }
        }
        return match append_user_path(dir) {
            Ok(_) => PathFixResult {
                ok: true,
                dir: Some(dir.to_string()),
                status: "added".into(),
                message: format!(
                    "已把 {dir} 加入用户 PATH 并同步到当前进程。新开的终端 / 重启后均生效。"
                ),
            },
            Err(e) => PathFixResult {
                ok: false,
                dir: Some(dir.to_string()),
                status: "process_only".into(),
                message: format!(
                    "已加入当前进程 PATH, 但持久化到用户 PATH 失败: {e}。可手动把 {dir} 加到 PATH。"
                ),
            },
        };
    }
    #[cfg(not(windows))]
    {
        return persist_unix_path(dir);
    }
}

/// 类 Unix(含 macOS) 持久化 PATH: 把 `export PATH="dir:$PATH"` 追加进 shell 配置
/// (zsh 为 macOS 默认, 同时照顾 bash/sh), 已存在则跳过。进程 PATH 已在调用处 prepend。
#[cfg(not(windows))]
fn persist_unix_path(dir: &str) -> PathFixResult {
    use std::io::Write;
    let home = match home_dir() {
        Some(h) => h,
        None => {
            return PathFixResult {
                ok: true,
                dir: Some(dir.to_string()),
                status: "process_only".into(),
                message: format!("已加入当前进程 PATH ({dir})。"),
            }
        }
    };
    let line = format!("export PATH=\"{dir}:$PATH\"");
    let mut wrote = false;
    let mut already = false;
    for rc in [".zshrc", ".zprofile", ".bash_profile", ".profile"] {
        let p = home.join(rc);
        let existing = std::fs::read_to_string(&p).unwrap_or_default();
        if existing.contains(dir) {
            already = true;
            continue;
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
            let _ = writeln!(f, "\n# Added by Polaris\n{line}");
            wrote = true;
        }
    }
    if wrote {
        PathFixResult {
            ok: true,
            dir: Some(dir.to_string()),
            status: "added".into(),
            message: format!("已把 {dir} 写进 shell 配置 (~/.zshrc 等) 并同步当前进程。新开终端即生效。"),
        }
    } else if already {
        PathFixResult {
            ok: true,
            dir: Some(dir.to_string()),
            status: "present".into(),
            message: format!("{dir} 已在 shell 配置中 (进程 PATH 已同步)。"),
        }
    } else {
        PathFixResult {
            ok: true,
            dir: Some(dir.to_string()),
            status: "process_only".into(),
            message: format!("已加入当前进程 PATH ({dir})。"),
        }
    }
}

/// 通过 PowerShell .NET API 把 dir 追加进用户 PATH (会广播 WM_SETTINGCHANGE)。
#[cfg(windows)]
fn append_user_path(dir: &str) -> Result<(), String> {
    // 单引号转义: PowerShell 里单引号字符串内的 ' 写成 ''
    let safe = dir.replace('\'', "''");
    let script = format!(
        "$d = '{safe}'; \
$u = [Environment]::GetEnvironmentVariable('Path','User'); \
if ($null -eq $u) {{ $u = '' }}; \
$parts = $u.Split(';') | Where-Object {{ $_ -ne '' }}; \
if ($parts -notcontains $d) {{ \
  $base = $u.TrimEnd(';'); \
  if ($base -eq '') {{ $new = $d }} else {{ $new = $base + ';' + $d }}; \
  [Environment]::SetEnvironmentVariable('Path', $new, 'User'); \
  Write-Output 'ADDED' \
}} else {{ Write-Output 'PRESENT' }}"
    );
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
    cmd.stdin(Stdio::null());
    no_window(&mut cmd);
    let out = cmd
        .output()
        .map_err(|e| format!("调用 PowerShell 写 PATH 失败: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// 由一个具体的 claude 可执行文件路径推出「该上 PATH 的目录」。
/// npm 装解析到的常是 `node_modules/.../bin/claude.exe` —— 该上 PATH 的是 npm 全局前缀
/// (放 `claude.cmd` 的地方, npm 通常已替我们加好), 而非内部 bin 目录; 其余情况取父目录。
fn claude_dir_from_path(p: &std::path::Path) -> Option<PathBuf> {
    if p.components().any(|c| c.as_os_str() == "node_modules") {
        if let Some(prefix) = npm_global_prefix() {
            return Some(prefix);
        }
    }
    p.parent().map(|x| x.to_path_buf())
}

/// claude 应该落脚的目录 (用于「修复 PATH」): 已解析路径的父目录优先, 否则 ~/.local/bin。
fn claude_dir_for_fix(claude: &ToolStatus) -> Option<PathBuf> {
    if let Some(p) = &claude.path {
        let pb = PathBuf::from(p.replace('/', std::path::MAIN_SEPARATOR_STR));
        return claude_dir_from_path(&pb);
    }
    home_dir().map(|h| h.join(".local").join("bin"))
}

/// 装完 PowerShell 7 后, 把它的目录 (`C:\Program Files\PowerShell\7`) 塞进 PATH (进程 + 用户),
/// 让**本进程**后续 spawn 的 claude 立刻找到真身, 而不是 WindowsApps 里起不来的 Store 别名 —— 装完免重启即用。
/// 真身不存在 (没装成功) 时返回 None。
fn ensure_pwsh_on_path() -> Option<PathFixResult> {
    let exe = pwsh_candidates().into_iter().find(|p| p.exists())?;
    let dir = exe.parent()?.to_string_lossy().to_string();
    Some(ensure_dir_on_path(&dir))
}

/// 探测可用的 Git Bash, 供 chat.rs spawn 时显式喂给子 claude (跨平台签名; 类 Unix 恒 None)。
#[cfg(windows)]
pub fn detect_git_bash() -> Option<PathBuf> {
    git_bash_path()
}
#[cfg(not(windows))]
pub fn detect_git_bash() -> Option<PathBuf> {
    None
}

/// 启动期环境预热 —— 让本进程**之后** spawn 的 claude CLI 一定「找得到、且有 shell 可用」,
/// 不必等用户走一遍「环境医生 / 安装」, 也不必重启 app。对应「环境配置时把 PATH 改成适合
/// claude code CLI 调用, 避免类似(找不到 claude / 找不到 shell)的问题」。
///
/// 只改**当前进程** env (set_var), **不写注册表** —— 启动期保持轻量、幂等、无副作用;
/// 持久化仍由「安装成功」与显式「修复 PATH」按钮负责。三件事, 每件仅在尚未满足时才动:
/// ① claude 所在目录 → 进程 PATH (即便 app 从一个 PATH 不含它的上下文被拉起也能裸名命中);
/// ② 真身 PowerShell 7 目录 → 进程 PATH (claude 在 Windows 靠 pwsh/git-bash 跑工具, 缺则报错);
/// ③ 找到 Git Bash 就设 `CLAUDE_CODE_GIT_BASH_PATH` (claude 在 Windows 默认偏好 git-bash)。
///
/// 内部会跑 where.exe / `npm prefix -g` (可能各几百 ms), 故在后台线程里做, 不阻塞 app 启动。
pub fn prime_path_for_claude() {
    std::thread::spawn(prime_path_for_claude_inner);
}

/// macOS/Linux: 从 Finder/Dock 启动的 GUI 进程只继承极简 PATH (`/usr/bin:/bin:/usr/sbin:/sbin`),
/// 拿不到用户 shell (`~/.zprofile`/`~/.profile` 里 —— 我们装 Node 时及 Homebrew 都写在这) 配的
/// node/npm/claude 目录 → `which`/spawn 全落空。跑一次**登录 shell** 把真实 PATH 取回来。
/// 用 `-lc`(登录、非交互): 读 `.zprofile`/`.profile`/`.bash_profile`, 不读 `.zshrc`, 既能拿到
/// 我们写的 node 目录与 Homebrew, 又避开交互式 shell 无 tty 时可能的卡顿。
#[cfg(not(windows))]
fn login_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let mut cmd = Command::new(&shell);
    cmd.args(["-lc", "printf %s \"$PATH\""]);
    cmd.stdin(Stdio::null());
    let out = output_with_timeout(cmd, Duration::from_secs(20))?;
    if !out.status.success() {
        return None;
    }
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if p.is_empty() {
        None
    } else {
        Some(p)
    }
}

/// 把登录 shell 的 PATH 里「进程 PATH 还没有的目录」并到进程 PATH 末尾 (系统目录仍优先,
/// 仅补充用户目录, 不抢占)。让本进程之后 `which`/spawn 与终端里行为一致。
#[cfg(not(windows))]
fn merge_login_path_into_process(login_path: &str) {
    use std::collections::HashSet;
    let cur = std::env::var("PATH").unwrap_or_default();
    let have: HashSet<String> = cur.split(':').map(|s| s.trim_end_matches('/').to_string()).collect();
    let adds: Vec<&str> = login_path
        .split(':')
        .filter(|s| !s.is_empty() && !have.contains(&s.trim_end_matches('/').to_string()))
        .collect();
    if adds.is_empty() {
        return;
    }
    let merged = if cur.is_empty() {
        adds.join(":")
    } else {
        format!("{cur}:{}", adds.join(":"))
    };
    std::env::set_var("PATH", merged);
}

/// 预热的实际逻辑 (同步)。抽出来便于单测直接调用并断言, 公开入口只负责丢到后台线程。
fn prime_path_for_claude_inner() {
    // ⓪ macOS/Linux: 先并入登录 shell 的真实 PATH (Finder 启动只有极简 PATH), 再直接补上
    // 本应用装 Node 的目录 —— 即便 shell 配置因故没生效, node/npm/claude 也都找得到。
    #[cfg(not(windows))]
    {
        if let Some(lp) = login_shell_path() {
            merge_login_path_into_process(&lp);
        }
        for dir in node_dir_candidates() {
            if dir.exists() {
                prepend_process_path(&dir.to_string_lossy());
            }
        }
    }
    // ① claude 目录上进程 PATH
    if let Some(exe) = resolve_claude_exe() {
        if let Some(dir) = claude_dir_from_path(&exe) {
            prepend_process_path(&dir.to_string_lossy());
        }
    }
    // ② 真身 pwsh 目录上进程 PATH (滤掉 Store 别名: pwsh_candidates 只列 Program Files 真身)
    #[cfg(windows)]
    if let Some(exe) = pwsh_candidates().into_iter().find(|p| p.exists()) {
        if let Some(dir) = exe.parent() {
            prepend_process_path(&dir.to_string_lossy());
        }
    }
    // ③ Git Bash → 环境变量 (用户没显式设过才补)
    #[cfg(windows)]
    if std::env::var_os("CLAUDE_CODE_GIT_BASH_PATH").is_none() {
        if let Some(bash) = git_bash_path() {
            std::env::set_var("CLAUDE_CODE_GIT_BASH_PATH", bash);
        }
    }
}

// ───────────────────────── Commands ─────────────────────────

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn env_check() -> EnvReport {
    let os = std::env::consts::OS.to_string();

    let claude = detect(
        "claude",
        "Claude Code",
        "claude",
        &["--version"],
        &claude_candidates(),
        true,
        "未安装 —— 可一键安装 (官方脚本)",
    );
    let pwsh = detect(
        "pwsh",
        "PowerShell 7",
        "pwsh",
        &["--version"],
        &pwsh_candidates(),
        false,
        "未安装 —— 建议安装 (winget)",
    );
    let node = detect(
        "node",
        "Node.js",
        "node",
        &["--version"],
        &[],
        false,
        "未安装 (npm 安装方式需要它)",
    );
    let npm = detect(
        "npm",
        "npm",
        "npm",
        &["--version"],
        &[],
        false,
        "未安装",
    );

    // PATH 体检: claude 安装目录是否在用户 PATH 里
    let claude_dir = claude_dir_for_fix(&claude);
    let claude_dir_on_user_path = match (&claude_dir, read_user_path()) {
        (Some(d), Some(up)) => path_contains_dir(&up, &d.to_string_lossy()),
        // 没装 / 拿不到用户 PATH → 当作「无需提示修复」(待安装后再判)
        _ => true,
    };

    // 可用 shell: Windows 需真身 pwsh (detect 已滤掉 Store 别名) 或 Git Bash;
    // 类 Unix(含 macOS) 自带 /bin/sh、zsh/bash, claude 直接可用 → 恒就绪。
    #[cfg(windows)]
    let shell_ready = pwsh.found || git_bash_path().is_some();
    #[cfg(not(windows))]
    let shell_ready = true;
    let ready = claude.found && shell_ready;

    EnvReport {
        os,
        claude,
        pwsh,
        node,
        npm,
        claude_dir: claude_dir.as_deref().map(to_fwd),
        claude_dir_on_user_path,
        shell_ready,
        ready,
    }
}

/// 修复 PATH: 把 claude 所在目录写进用户 PATH + 当前进程 PATH。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn env_fix_path() -> Result<PathFixResult, String> {
    let report = env_check();
    match report.claude_dir {
        Some(d) => Ok(ensure_dir_on_path(&d)),
        None => Ok(PathFixResult {
            ok: false,
            dir: None,
            status: "skipped".into(),
            message: "尚未找到 Claude Code 安装目录, 请先安装。".into(),
        }),
    }
}

/// 安装 Claude Code。method: "npm" (默认, 经国内镜像) | "native" (官方原生脚本, 兜底)。
/// 流式把安装日志通过 `env:stream` 事件推给前端; 成功后自动修 PATH。
/// 跨平台: Windows 经 PowerShell, macOS/Linux 经 `sh`(npm 方式两端一致; native 各走各的官方脚本)。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn env_install_claude(app: AppHandle, method: Option<String>) -> Result<String, String> {
    let method = method.unwrap_or_else(|| "npm".to_string());
    let inner = claude_install_cmd(&method);
    let req_id = next_req_id();
    let cmd = build_install_shell(&inner);
    stream_install(app, req_id.clone(), cmd, true, "Claude Code");
    Ok(req_id)
}

/// 安装 Node.js LTS —— npm 安装方式的前置依赖。两端都走「国内镜像优先」, 装完把 bin 目录
/// 塞进进程 PATH (stream_install 收尾处), 故 `fix_path_after=false`。
///
/// - **Windows**: 两层策略 —— ① 有 winget 先用 winget; ② 缺失/失败 → 下载官方 MSI (npmmirror
///   镜像加速) 静默安装 (Win10 常无 winget, 故必须有 MSI 兜底)。
/// - **macOS**: 下载 Node 官方 darwin tar.gz (走 npmmirror 二进制镜像, 国内可达) **免 sudo**
///   解压到 `~/.local/polaris-node`, 并把其 `bin` 写进 shell 配置 —— 不动系统目录、不弹 UAC/密码。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn env_install_node(app: AppHandle) -> Result<String, String> {
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = &app;
        return Err("Node.js 自动安装目前支持 Windows 与 macOS; Linux 请用系统包管理器安装。".into());
    }
    #[cfg(any(windows, target_os = "macos"))]
    {
        let req_id = next_req_id();
        #[cfg(windows)]
        let cmd = build_powershell(NODE_INSTALL_SCRIPT);
        #[cfg(target_os = "macos")]
        let cmd = build_install_shell(MAC_NODE_INSTALL_SCRIPT);
        stream_install(app, req_id.clone(), cmd, false, "Node.js");
        Ok(req_id)
    }
}

/// macOS Node.js 安装脚本 (POSIX sh) —— 免 sudo: 下载官方 darwin tar.gz 解压到
/// `~/.local/polaris-node`, 把 `bin` 写进 zsh/bash 配置。下载走 npmmirror 二进制镜像 (国内可达),
/// 不行再退 nodejs.org 直连。选 20.x LTS, 与 Windows 一致。
#[cfg(target_os = "macos")]
const MAC_NODE_INSTALL_SCRIPT: &str = r#"
VER=20.18.1
ARCH=$(uname -m)
case "$ARCH" in
  arm64|aarch64) NARCH=arm64 ;;
  x86_64) NARCH=x64 ;;
  *) NARCH=x64 ;;
esac
PKG="node-v${VER}-darwin-${NARCH}.tar.gz"
DEST="$HOME/.local"
NODE_DIR="$DEST/polaris-node"
TMP="$(mktemp -d)"
TARBALL="$TMP/$PKG"
echo "目标架构: $NARCH; Node 版本: v$VER"
OK=0
for U in \
  "https://cdn.npmmirror.com/binaries/node/v${VER}/${PKG}" \
  "https://npmmirror.com/mirrors/node/v${VER}/${PKG}" \
  "https://nodejs.org/dist/v${VER}/${PKG}" ; do
  echo "下载: $U"
  if curl -fsSL "$U" -o "$TARBALL" && [ -s "$TARBALL" ]; then OK=1; break; fi
  echo "  下载失败, 试下一个镜像..."
done
if [ "$OK" != "1" ]; then echo "Node.js 下载失败 (检查网络/代理后重试)。"; rm -rf "$TMP"; exit 1; fi
mkdir -p "$DEST"
rm -rf "$NODE_DIR"
mkdir -p "$NODE_DIR"
tar -xzf "$TARBALL" -C "$NODE_DIR" --strip-components=1 || { echo "解压失败。"; rm -rf "$TMP"; exit 1; }
rm -rf "$TMP"
BIN="$NODE_DIR/bin"
if [ ! -x "$BIN/node" ]; then echo "解压后未找到 node 可执行文件。"; exit 1; fi
# 把 node bin 写进 shell 配置 (zsh 为 macOS 默认; 同时照顾 bash/sh), 已存在则不重复
LINE="export PATH=\"$BIN:\$PATH\""
for RC in "$HOME/.zshrc" "$HOME/.zprofile" "$HOME/.bash_profile" "$HOME/.profile" ; do
  touch "$RC" 2>/dev/null || true
  grep -qF "$BIN" "$RC" 2>/dev/null || printf '\n# Added by Polaris (Node.js)\n%s\n' "$LINE" >> "$RC"
done
export PATH="$BIN:$PATH"
echo "Node.js 安装完成: node $("$BIN/node" -v), npm $("$BIN/npm" -v)"
echo "已写入 ~/.zshrc 等; 本次会话已即时生效。"
"#;

/// Node.js LTS 安装脚本: winget 优先, 失败则下载官方 MSI (国内 npmmirror 镜像加速) 静默安装。
/// 选 20.x LTS ("Iron"): 长期支持、兼容 Windows 10。
const NODE_INSTALL_SCRIPT: &str = r#"
$ErrorActionPreference = 'Continue'
# ① 优先 winget (能拿最新 LTS, 自带配 PATH)
$wg = Get-Command winget -ErrorAction SilentlyContinue
if ($wg) {
  Write-Output '检测到 winget, 优先用它安装 Node.js LTS...'
  & winget install --id OpenJS.NodeJS.LTS -e --source winget --accept-package-agreements --accept-source-agreements
  if ($LASTEXITCODE -eq 0) { Write-Output 'Node.js (winget) 安装完成。'; exit 0 }
  Write-Output ('winget 安装未成功 (退出码 ' + $LASTEXITCODE + '), 改用直接下载 MSI...')
} else {
  Write-Output '未检测到 winget (Windows 10 常见), 改用直接下载官方 MSI...'
}
# ② 下载官方 Node LTS MSI -> %TEMP% -> msiexec 静默安装。下载路径走国内 npmmirror 镜像兜底。
$ver = '20.18.1'
$arch = switch ($env:PROCESSOR_ARCHITECTURE) { 'ARM64' { 'arm64' } 'AMD64' { 'x64' } default { 'x86' } }
$msi = "node-v$ver-$arch.msi"
$dst = Join-Path $env:TEMP $msi
$urls = @(
  "https://cdn.npmmirror.com/binaries/node/v$ver/$msi",
  "https://npmmirror.com/mirrors/node/v$ver/$msi",
  "https://nodejs.org/dist/v$ver/$msi"
)
$ok = $false
foreach ($u in $urls) {
  try {
    Write-Output "下载: $u"
    Invoke-WebRequest -Uri $u -OutFile $dst -UseBasicParsing -TimeoutSec 600
    if ((Test-Path $dst) -and ((Get-Item $dst).Length -gt 1MB)) { $ok = $true; break }
  } catch {
    Write-Output ("  下载失败: " + $_.Exception.Message)
  }
}
if (-not $ok) {
  Write-Output 'Node.js 安装包下载失败 (可检查网络 / 代理后重试)。'
  exit 1
}
# 安装到 Program Files 需要管理员权限 -> 用 RunAs 触发 UAC (拒绝则友好报错, 不静默失败)
Write-Output "安装中 (msiexec, 会弹一次 UAC 授权): $dst"
try {
  $p = Start-Process msiexec.exe -ArgumentList ('/i "' + $dst + '" /quiet /norestart') -Wait -PassThru -Verb RunAs
} catch {
  Write-Output ('安装启动失败 (可能未授予管理员权限): ' + $_.Exception.Message)
  exit 1
}
Remove-Item $dst -ErrorAction SilentlyContinue
if ($p.ExitCode -ne 0) { Write-Output ('msiexec 退出码 ' + $p.ExitCode); exit 1 }
Write-Output 'Node.js 安装完成。'
"#;

/// 安装 PowerShell 7。成功无需改 PATH (MSI / winget 安装都会自带配 PATH)。
///
/// 之前只用 `winget`, 但很多机器上要么没有 winget、要么 winget 源在国内拉不动
/// → 用户报「PowerShell 7 下载不了」。这里改成**两层策略**:
/// ① 有 winget 先用 winget (官方、能拿最新版);
/// ② winget 缺失 / 失败 → **直接下载官方 MSI 再 msiexec 静默安装**, 且下载走
///    国内可达的 GitHub 文件代理 (gh-proxy / ghfast) 兜底, 实在不行再走 GitHub 直连。
///    这就是「下载路径」修复 —— 明确把 MSI 落到 `%TEMP%` 再装, 不再黑盒依赖 winget。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn env_install_pwsh(app: AppHandle) -> Result<String, String> {
    if !cfg!(windows) {
        return Err("PowerShell 7 自动安装仅支持 Windows。".into());
    }
    let req_id = next_req_id();
    let cmd = build_powershell(PWSH_INSTALL_SCRIPT);
    stream_install(app, req_id.clone(), cmd, false, "PowerShell 7");
    Ok(req_id)
}

/// PowerShell 7 安装脚本: winget 优先, 失败则下载官方 MSI (国内代理加速) 静默安装。
/// 版本仅用于 MSI 兜底直链 (winget 路径自动取最新); 选 7.4.x LTS, 稳定且长期可用。
const PWSH_INSTALL_SCRIPT: &str = r#"
$ErrorActionPreference = 'Continue'
# ① 优先 winget (能拿最新版, 自带配 PATH)
$wg = Get-Command winget -ErrorAction SilentlyContinue
if ($wg) {
  Write-Output '检测到 winget, 优先用它安装 PowerShell 7...'
  & winget install --id Microsoft.PowerShell -e --source winget --accept-package-agreements --accept-source-agreements
  if ($LASTEXITCODE -eq 0) { Write-Output 'PowerShell 7 (winget) 安装完成。'; exit 0 }
  Write-Output ('winget 安装未成功 (退出码 ' + $LASTEXITCODE + '), 改用直接下载 MSI...')
} else {
  Write-Output '未检测到 winget, 改用直接下载官方 MSI...'
}
# ② 下载官方 MSI -> %TEMP% -> msiexec 静默安装。下载路径走国内可达的 GitHub 代理兜底。
$ver = '7.4.6'
$arch = switch ($env:PROCESSOR_ARCHITECTURE) { 'ARM64' { 'arm64' } 'AMD64' { 'x64' } default { 'x86' } }
$msi = "PowerShell-$ver-win-$arch.msi"
$dst = Join-Path $env:TEMP $msi
$rel = "https://github.com/PowerShell/PowerShell/releases/download/v$ver/$msi"
$urls = @(
  "https://gh-proxy.com/$rel",
  "https://ghfast.top/$rel",
  "https://ghproxy.net/$rel",
  $rel
)
$ok = $false
foreach ($u in $urls) {
  try {
    Write-Output "下载: $u"
    Invoke-WebRequest -Uri $u -OutFile $dst -UseBasicParsing -TimeoutSec 600
    if ((Test-Path $dst) -and ((Get-Item $dst).Length -gt 1MB)) { $ok = $true; break }
  } catch {
    Write-Output ("  下载失败: " + $_.Exception.Message)
  }
}
if (-not $ok) {
  Write-Output 'PowerShell 7 安装包下载失败 (可检查网络 / 代理后重试)。'
  exit 1
}
# 安装到 Program Files 需要管理员权限 -> 用 RunAs 触发 UAC (拒绝则友好报错, 不静默失败)
Write-Output "安装中 (msiexec, 会弹一次 UAC 授权): $dst"
try {
  $p = Start-Process msiexec.exe -ArgumentList ('/i "' + $dst + '" /quiet /norestart ADD_PATH=1') -Wait -PassThru -Verb RunAs
} catch {
  Write-Output ('安装启动失败 (可能未授予管理员权限): ' + $_.Exception.Message)
  exit 1
}
Remove-Item $dst -ErrorAction SilentlyContinue
if ($p.ExitCode -ne 0) { Write-Output ('msiexec 退出码 ' + $p.ExitCode); exit 1 }
Write-Output 'PowerShell 7 安装完成。'
"#;

// ───────────────────────── Claude Code 更新 ─────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeUpdateInfo {
    /// 是否已安装 (装了才谈更新)
    pub installed: bool,
    /// 当前版本 (纯 x.y.z, 解析不出则原样)
    pub current: Option<String>,
    /// 镜像上的最新版本
    pub latest: Option<String>,
    /// 是否有可用更新 (latest > current)
    pub update_available: bool,
    /// 是否成功查到了 latest (网络/镜像可用)
    pub checked: bool,
    /// 一句话说明
    pub message: String,
}

/// 把 "1.0.44 (Claude Code)" 这类串里第一个形如 a.b.c 的版本号解析成元组。
fn parse_triplet(tok: &str) -> Option<(u64, u64, u64)> {
    let mut it = tok.split('.');
    let a = it.next()?.parse::<u64>().ok()?;
    let b = it.next()?.parse::<u64>().ok()?;
    let c = it.next()?.parse::<u64>().ok()?;
    Some((a, b, c))
}

fn extract_semver(s: &str) -> Option<(u64, u64, u64)> {
    for tok in s.split(|c: char| !(c.is_ascii_digit() || c == '.')) {
        if tok.is_empty() {
            continue;
        }
        if let Some(t) = parse_triplet(tok) {
            return Some(t);
        }
    }
    None
}

/// npm 镜像上 Claude Code 的最新版本号 (`npm view ... version`, 走 npmmirror)。
fn npm_view_latest() -> Option<String> {
    let pkg = "@anthropic-ai/claude-code";
    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args([
            "/c",
            "npm",
            "view",
            pkg,
            "version",
            "--registry=https://registry.npmmirror.com",
        ]);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new("npm");
        c.args([
            "view",
            pkg,
            "version",
            "--registry=https://registry.npmmirror.com",
        ]);
        c
    };
    cmd.stdin(Stdio::null());
    no_window(&mut cmd);
    let out = output_with_timeout(cmd, Duration::from_secs(20))?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .map(|s| s.to_string())
}

/// 没有 npm 时的兜底: 直接打 npmmirror 的 registry HTTP 接口取 dist-tags.latest。
#[cfg(windows)]
fn registry_latest_via_http() -> Option<String> {
    let script = "(Invoke-RestMethod -UseBasicParsing \
'https://registry.npmmirror.com/@anthropic-ai/claude-code').'dist-tags'.latest";
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", script]);
    cmd.stdin(Stdio::null());
    no_window(&mut cmd);
    let out = output_with_timeout(cmd, Duration::from_secs(20))?;
    if !out.status.success() {
        return None;
    }
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!v.is_empty()).then_some(v)
}

#[cfg(not(windows))]
fn registry_latest_via_http() -> Option<String> {
    None
}

/// 检测 Claude Code 是否有新版本: 当前版本 (`claude --version`) vs 镜像 latest。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn env_claude_update_check() -> ClaudeUpdateInfo {
    let current_raw = probe_version("claude", &["--version"]);
    let installed = current_raw.is_some() || resolve_claude_exe().is_some();
    if !installed {
        return ClaudeUpdateInfo {
            installed: false,
            current: None,
            latest: None,
            update_available: false,
            checked: false,
            message: "未检测到 Claude Code, 请先安装。".into(),
        };
    }

    // 当前版本: 优先展示解析出的纯 semver, 否则原样
    let cur_semver = current_raw.as_deref().and_then(extract_semver);
    let current = cur_semver
        .map(|(a, b, c)| format!("{a}.{b}.{c}"))
        .or_else(|| current_raw.clone());

    let latest = npm_view_latest().or_else(registry_latest_via_http);
    match latest {
        Some(l) => {
            let lv = extract_semver(&l);
            let update_available = match (cur_semver, lv) {
                (Some(c), Some(n)) => n > c,
                _ => false,
            };
            let message = if update_available {
                format!("发现新版本 {l} (当前 {})。", current.clone().unwrap_or_default())
            } else {
                format!("已是最新版本 ({})。", current.clone().unwrap_or_default())
            };
            ClaudeUpdateInfo {
                installed: true,
                current,
                latest: Some(l),
                update_available,
                checked: true,
                message,
            }
        }
        None => ClaudeUpdateInfo {
            installed: true,
            current,
            latest: None,
            update_available: false,
            checked: false,
            message: "无法获取最新版本号 (可检查网络 / npm 后重试)。".into(),
        },
    }
}

/// 更新 Claude Code 到最新版 —— 走国内 npmmirror, 与默认安装方式同源, 国内最快。
/// 复用流式安装管线; 成功后清解析缓存并自动修 PATH (与首次安装一致)。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn env_update_claude(app: AppHandle) -> Result<String, String> {
    let inner = "npm install -g @anthropic-ai/claude-code@latest \
--registry=https://registry.npmmirror.com";
    let req_id = next_req_id();
    let cmd = build_install_shell(inner);
    stream_install(app, req_id.clone(), cmd, true, "Claude Code 更新");
    Ok(req_id)
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn env_cancel(req_id: String) -> Result<(), String> {
    if let Some(mut child) = CHILDREN.lock().remove(&req_id) {
        let _ = child.kill();
    }
    Ok(())
}

// ───────────────────────── 内部: 流式安装 ─────────────────────────

/// 构造一个跑给定内联命令的系统 shell 进程:
/// - Windows → PowerShell (见 `build_powershell`);
/// - 类 Unix(含 macOS) → `sh -lc`(`-l` 走登录配置以拿到用户 PATH, npm 全局 bin 才在内)。
/// 安装/更新 Claude Code 这类跨平台命令统一走它。
fn build_install_shell(inner: &str) -> Command {
    #[cfg(windows)]
    {
        build_powershell(inner)
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new("sh");
        cmd.args(["-lc", inner]);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }
}

/// Claude Code 的安装命令串 (按方式选择, 两端默认一致)。
/// - `npm` (**默认, 两端**): `npm i -g @anthropic-ai/claude-code --registry=npmmirror.com`。
///   原生二进制 (win32 / darwin-arm64 / darwin-x64 …) 经 `optionalDependencies` 由 npmmirror
///   **同源镜像**分发, postinstall 只把它拷成 `bin/claude` —— 整个过程**不碰 claude.ai / GCS**,
///   故国内 (含 macOS) 可装。这是国内最稳的路径, 需要 Node.js (缺则先 `env_install_node`)。
/// - `native` (**兜底, 境外网络**): 官方原生脚本 —— Windows `install.ps1` / macOS·Linux
///   `install.sh`。它从 claude.ai/GCS 拉二进制, **国内常被墙 → 默认不走**, 仅给能访问外网的人。
fn claude_install_cmd(method: &str) -> String {
    match method {
        "native" => {
            #[cfg(windows)]
            {
                "irm https://claude.ai/install.ps1 | iex".to_string()
            }
            #[cfg(not(windows))]
            {
                "curl -fsSL https://claude.ai/install.sh | bash".to_string()
            }
        }
        _ => "npm install -g @anthropic-ai/claude-code --registry=https://registry.npmmirror.com"
            .to_string(),
    }
}

/// 构造一个跑给定内联命令的 PowerShell 进程 (Bypass 执行策略, 以便 iex 远程脚本)。
fn build_powershell(inner: &str) -> Command {
    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        inner,
    ]);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    no_window(&mut cmd);
    cmd
}

fn emit(app: &AppHandle, ev: EnvStreamEvent) {
    let _ = app.emit("env:stream", ev);
}

/// 起子进程, 双线程读 stdout/stderr → `env:stream` 日志; 退出后(可选)修 PATH, 再发 done。
fn stream_install(app: AppHandle, req_id: String, mut cmd: Command, fix_path_after: bool, label: &str) {
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            emit(
                &app,
                EnvStreamEvent {
                    req_id,
                    kind: "done".into(),
                    line: None,
                    ok: Some(false),
                    message: Some(format!("启动安装进程失败: {e} (系统 shell 是否可用?)")),
                },
            );
            return;
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    CHILDREN.lock().insert(req_id.clone(), child);

    // stderr 线程
    if let Some(stderr) = stderr {
        let app_e = app.clone();
        let req_e = req_id.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                let Ok(line) = line else { continue };
                if line.trim().is_empty() {
                    continue;
                }
                emit(
                    &app_e,
                    EnvStreamEvent {
                        req_id: req_e.clone(),
                        kind: "log".into(),
                        line: Some(line),
                        ok: None,
                        message: None,
                    },
                );
            }
        });
    }

    // stdout 线程 (主): 读完 → wait → 修 PATH → done
    let label = label.to_string();
    std::thread::spawn(move || {
        if let Some(stdout) = stdout {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let Ok(line) = line else { continue };
                if line.trim().is_empty() {
                    continue;
                }
                emit(
                    &app,
                    EnvStreamEvent {
                        req_id: req_id.clone(),
                        kind: "log".into(),
                        line: Some(line),
                        ok: None,
                        message: None,
                    },
                );
            }
        }

        let child_opt = CHILDREN.lock().remove(&req_id);
        let success = if let Some(mut child) = child_opt {
            child.wait().map(|s| s.success()).unwrap_or(false)
        } else {
            // 被 cancel 掉了
            emit(
                &app,
                EnvStreamEvent {
                    req_id: req_id.clone(),
                    kind: "done".into(),
                    line: None,
                    ok: Some(false),
                    message: Some("安装已取消。".into()),
                },
            );
            return;
        };

        let mut message = if success {
            format!("{label} 安装完成。")
        } else {
            format!("{label} 安装未成功 (进程非零退出)，可查看上方日志或改用其他方式重试。")
        };

        // 装完 Node: 把 node bin 目录塞进**进程** PATH, 让同一会话里紧接着的 npm/claude 安装
        // 立刻找得到 npm —— 安装器(Win MSI / mac 写 shell 配置)只为「新进程/新终端」配 PATH,
        // 本进程不刷新就会「装了 Node 还说没 npm」。两端通用 (node_dir_candidates 按平台给候选)。
        if success {
            if let Some(dir) = node_dir_candidates().into_iter().find(|p| p.exists()) {
                prepend_process_path(&dir.to_string_lossy());
            }
        }

        // 装完 claude 的路径可能变了 → 清空 chat spawn 的解析缓存, 下次重新解析
        if success {
            *CLAUDE_EXE_CACHE.lock() = None;
            // 若真身 pwsh 已就位 (本次刚装好, 或本就装了), 顺手把它的目录注入 PATH(进程+用户),
            // 让本进程 spawn 的 claude 立刻用上 —— 装完 PowerShell 7 免重启即可对话。
            if let Some(fix) = ensure_pwsh_on_path() {
                if fix.ok && fix.status == "added" {
                    message.push('\n');
                    message.push_str(&fix.message);
                }
            }
        }

        // 成功后自动修 PATH (改环境变量) —— 这是「装完即可用」的关键
        if success && fix_path_after {
            let report = env_check();
            if let Some(dir) = report.claude_dir {
                let fix = ensure_dir_on_path(&dir);
                message.push('\n');
                message.push_str(&fix.message);
            }
        }

        emit(
            &app,
            EnvStreamEvent {
                req_id: req_id.clone(),
                kind: "done".into(),
                line: None,
                ok: Some(success),
                message: Some(message),
            },
        );
    });
}

// ───────────────────────── Tests ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_no_proxy_adds_loopback_to_empty() {
        let out = merge_no_proxy("");
        assert_eq!(out, "127.0.0.1,localhost,::1");
    }

    #[test]
    fn merge_no_proxy_preserves_existing_and_appends() {
        let out = merge_no_proxy("example.com, 10.0.0.5");
        assert_eq!(out, "example.com,10.0.0.5,127.0.0.1,localhost,::1");
    }

    #[test]
    fn merge_no_proxy_is_idempotent_case_insensitive() {
        // 用户已配了回环（含大小写差异）→ 不重复添加。
        let out = merge_no_proxy("LOCALHOST,127.0.0.1");
        assert_eq!(out, "LOCALHOST,127.0.0.1,::1");
        // 再并一次保持稳定（幂等）。
        assert_eq!(merge_no_proxy(&out), out);
    }

    /// 把内嵌的安装脚本原文 dump 到临时文件, 供外部用 PowerShell AST 解析器做语法校验
    /// (内嵌脚本的语法错误只会在「真正安装」时才暴露, 这里提前抓出来)。`--ignored` 手动跑。
    #[test]
    #[ignore]
    fn dump_install_scripts() {
        let dir = std::env::temp_dir();
        let node = dir.join("polaris_node_install.ps1");
        let pwsh = dir.join("polaris_pwsh_install.ps1");
        std::fs::write(&node, NODE_INSTALL_SCRIPT).unwrap();
        std::fs::write(&pwsh, PWSH_INSTALL_SCRIPT).unwrap();
        println!("NODE_SCRIPT={}", node.display());
        println!("PWSH_SCRIPT={}", pwsh.display());
    }

    #[test]
    fn path_contains_dir_is_case_and_slash_insensitive() {
        let p = r"C:\Program Files\PowerShell\7;C:\Users\mi\.local\bin";
        assert!(path_contains_dir(p, r"c:\program files\powershell\7")); // 大小写不敏感
        assert!(path_contains_dir(p, r"C:\Users\mi\.local\bin\")); // 尾斜杠归一
        assert!(!path_contains_dir(p, r"C:\Program Files\nodejs"));
        assert!(!path_contains_dir(p, "")); // 空目标不误命中
    }

    #[test]
    fn claude_dir_from_path_picks_parent_or_npm_prefix() {
        // 普通原生 exe → 取父目录
        let native = PathBuf::from(r"C:\Users\mi\.local\bin\claude.exe");
        assert_eq!(
            claude_dir_from_path(&native),
            Some(PathBuf::from(r"C:\Users\mi\.local\bin"))
        );
        // node_modules 路径 → 永不返回那个内部 bin 目录 (要么 npm 前缀, 要么至少不是 .../bin 自身的误用)
        let npmish =
            PathBuf::from(r"D:\npm\node_modules\@anthropic-ai\claude-code\bin\claude.exe");
        let dir = claude_dir_from_path(&npmish).expect("应解析出某个目录");
        assert!(
            !dir.ends_with("claude.exe"),
            "返回的应是目录而非文件: {dir:?}"
        );
    }

    /// 所有会改进程 PATH/env 的断言放进同一个测试串行跑, 避免与其他测试并发改 env 抢同一全局态。
    #[test]
    fn prime_and_prepend_behaviour() {
        // prepend 幂等: 首次真加、PATH 命中、再加返回 false
        let marker = r"Z:\polaris-test-marker-dir-do-not-exist";
        let first = prepend_process_path(marker);
        let path_now = std::env::var("PATH").unwrap_or_default();
        assert!(first, "首次应真的前插");
        assert!(path_contains_dir(&path_now, marker), "前插后 PATH 应含该目录");
        assert!(!prepend_process_path(marker), "已在 PATH 中应返回 false (幂等)");

        // resolve_claude_exe: 若本机装了 claude, 解析出的路径必须真实存在 (Windows 上偏好 .exe)
        if let Some(exe) = resolve_claude_exe() {
            assert!(exe.exists(), "解析出的 claude 路径应存在: {exe:?}");
            #[cfg(windows)]
            {
                let is_exe = exe
                    .extension()
                    .map(|e| e.eq_ignore_ascii_case("exe"))
                    .unwrap_or(false);
                // 本机同时有 .exe 与 .cmd 时, 必须挑 .exe (chat spawn 只认 .exe)
                let has_exe_alt = which_all("claude")
                    .iter()
                    .any(|p| p.extension().map(|e| e.eq_ignore_ascii_case("exe")).unwrap_or(false));
                if has_exe_alt {
                    assert!(is_exe, "存在 .exe 候选时应优先解析为 .exe, 实得: {exe:?}");
                }
            }
        }

        // 预热不应 panic; 且 (Windows) 若真身 pwsh 存在, 其目录预热后应在进程 PATH 中 → claude 能找到 shell
        prime_path_for_claude_inner();
        #[cfg(windows)]
        if let Some(pwsh) = pwsh_candidates().into_iter().find(|p| p.exists()) {
            let dir = pwsh.parent().unwrap().to_string_lossy().to_string();
            let path_after = std::env::var("PATH").unwrap_or_default();
            assert!(
                path_contains_dir(&path_after, &dir),
                "预热后真身 pwsh 目录应在进程 PATH: {dir}"
            );
        }
    }
}
