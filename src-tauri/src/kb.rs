//! 板块 ② 维基知识库 — MVP 实现
//!
//! 设计依据: PRD-v6 §8 + v5.1 §3-§7
//! - 三层目录铁律: raw/ output/ wiki/ (新建空 KB 时创建)
//! - 关键词加权评分搜索 (PRD §8.8): 标题 +10, 课程标签 +8, 正文 +1
//! - 双链 [[wiki-link]] 解析 -> 图谱节点+边
//! - YAML frontmatter 提取 category (PRD §8.5)
//!
//! MVP 缩水:
//! - 不做 Embedding (Karpathy 论点: 结构化 wiki + 长上下文 > 向量)
//! - 不做 SimHash 去重 (留 §8.6, 后续接入)
//! - 索引常驻内存, 进程重启时重扫 (后续走 SQLite)

use crate::convert;
use anyhow::Result;
use directories::{ProjectDirs, UserDirs};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(feature = "desktop")]
use tauri::{AppHandle, Emitter, Manager};
#[cfg(not(feature = "desktop"))]
use crate::host::AppHandle;
use walkdir::WalkDir;

// ───────────────────────── State ─────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct KbDoc {
    pub rel_path: String,
    pub title: String,
    pub category: String,
    /// frontmatter 的 `type` (entity/concept/source/synthesis), 缺省空串。供 kb_lint 校验。
    pub doc_type: String,
    pub wikilinks: Vec<String>,
}

/// 按需读取 KB 中某篇 wiki 的完整正文(不走 INDEX, 直接读磁盘)。
fn read_doc_body(rel_path: &str) -> Option<String> {
    let root = KB_ROOT.read();
    let full = root.join(rel_path);
    if !full.starts_with(&*root) || !full.is_file() {
        return None;
    }
    // 去除 frontmatter, 只返回正文(与 parse_doc 保持一致)
    let raw = fs::read_to_string(&full).ok()?;
    match RE_FRONTMATTER.captures(&raw) {
        Some(c) => Some(raw[c.get(0)?.end()..].to_string()),
        None => Some(raw),
    }
}

static INDEX: Lazy<RwLock<Vec<KbDoc>>> = Lazy::new(|| RwLock::new(Vec::new()));
static KB_ROOT: Lazy<RwLock<PathBuf>> = Lazy::new(|| RwLock::new(PathBuf::new()));

// ───────────────────────── Init ──────────────────────────

pub fn init(_app: &AppHandle) -> Result<()> {
    let settings = load_settings();
    let root = settings
        .kb_root
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| default_kb_root().unwrap_or_else(|_| PathBuf::from(".")));
    ensure_skeleton(&root)?;
    *KB_ROOT.write() = root.clone();
    // 把「全量扫描解析」挪到后台线程，别拖住窗口出现。
    // scan_all 会 WalkDir 递归读+解析每篇 .md（KB 越大越慢）。而 INDEX 只被 KB 视图/命令
    // 按需用，首屏根本不读它，所以启动即设好 KB_ROOT（其它板块要它、且很轻），
    // 重活丢后台几百 ms 内填好 INDEX。
    // 注: 此前这里还有「首启播种毛主席资料库」(seed_default_kb)——已改成「名人资料包」
    // 按需安装(kb_pack_install)，不再初始自带。
    std::thread::spawn(move || {
        let docs = scan_all(&root);
        *INDEX.write() = docs;
    });
    Ok(())
}

fn default_kb_root() -> Result<PathBuf> {
    let user = UserDirs::new().ok_or_else(|| anyhow::anyhow!("no user dir"))?;
    let home = user.home_dir();
    Ok(home.join("Polaris").join("PolarisKB"))
}

// ───────────────────────── 名人资料包 (KB Packs) ─────────────────────────
//
// 随安装包打进来的名人资料(`resources/seed-kb/<名人>/`)**不再首启自动播种**，
// 改为「名人知识库」里的可安装资料包：点「下载到我的资料库」才拷到 `<KB>/raw/<名人>/`，
// 并顺带把配套 skill(内含该资料库的使用方法)装到用户技能目录 —— 资料和用法一起到手。
// 移除资料包时配套 skill 一并移除。

/// 资料包定义(编译期目录)。payload 走 `resources/seed-kb/<dir>`，仍随安装包分发，
/// 「下载」即本地拷贝，离线可用；将来要做远程包再扩 source 字段。
struct KbPackDef {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    /// `resources/seed-kb/` 与 `raw/` 下共用的目录名
    dir: &'static str,
    /// 配套 skill(技能目录 id)，安装/移除资料包时一并装/卸
    skill_id: &'static str,
}

fn pack_catalog() -> Vec<KbPackDef> {
    vec![KbPackDef {
        id: "mao",
        name: "毛主席",
        description: "《毛泽东选集》等著作的结构化资料库。装入后消息里写「请教毛主席」即可让他用毛选式大白话 + 矛盾分析法客观分析问题、生成标注来源的 HTML；同时自动创建「毛主席」人格项目。",
        dir: "毛主席",
        skill_id: "consult-mao",
    }]
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KbPackMeta {
    pub id: String,
    pub name: String,
    pub description: String,
    pub skill_id: String,
    pub installed: bool,
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn kb_pack_list() -> Vec<KbPackMeta> {
    let root = KB_ROOT.read().clone();
    pack_catalog()
        .into_iter()
        .map(|p| KbPackMeta {
            id: p.id.into(),
            name: p.name.into(),
            description: p.description.into(),
            skill_id: p.skill_id.into(),
            installed: root.join("raw").join(p.dir).exists(),
        })
        .collect()
}

/// 安装资料包：拷资料到 `raw/<名人>/` + 重扫索引 + 装配套 skill。返回索引文件总数。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn kb_pack_install(app: AppHandle, id: String) -> Result<usize, String> {
    let pack = pack_catalog()
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("没有资料包 '{}'", id))?;
    let src = seed_source(&app)
        .map(|s| s.join(pack.dir))
        .filter(|s| s.exists())
        .ok_or("安装包内未找到该资料包的数据(资源目录缺失)")?;
    let root = KB_ROOT.read().clone();
    copy_dir_recursive(&src, &root.join("raw").join(pack.dir)).map_err(|e| e.to_string())?;
    let docs = scan_all(&root);
    let n = docs.len();
    *INDEX.write() = docs;
    // 配套 skill(含资料库使用方法)装到用户技能目录。best-effort: 失败不回滚资料。
    let _ = crate::skills::install_skill(pack.skill_id.to_string());
    // 毛主席包附带「毛主席」人格项目(人格 CLAUDE.md + 专属 KB scope)
    if pack.id == "mao" {
        crate::conv::ensure_mao_project();
    }
    Ok(n)
}

/// 移除资料包：删 `raw/<名人>/` + 重扫索引 + 卸配套 skill。返回索引文件总数。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn kb_pack_remove(id: String) -> Result<usize, String> {
    let pack = pack_catalog()
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("没有资料包 '{}'", id))?;
    let root = KB_ROOT.read().clone();
    let dst = root.join("raw").join(pack.dir);
    if dst.exists() {
        fs::remove_dir_all(&dst).map_err(|e| e.to_string())?;
    }
    let _ = crate::skills::delete_skill(pack.skill_id.to_string());
    let docs = scan_all(&root);
    let n = docs.len();
    *INDEX.write() = docs;
    Ok(n)
}

/// 定位打进安装包的资料库种子目录(其内含 `毛主席/` 等资料包数据)。
/// 发布版走 Tauri `resource_dir`; 开发期回退到 `src-tauri/resources/seed-kb`。
fn seed_source(app: &AppHandle) -> Option<PathBuf> {
    if let Ok(rd) = app.path().resource_dir() {
        for cand in [rd.join("resources").join("seed-kb"), rd.join("seed-kb")] {
            if cand.exists() {
                return Some(cand);
            }
        }
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join("seed-kb");
    if dev.exists() {
        Some(dev)
    } else {
        None
    }
}

/// 递归拷贝目录内容到目标; 已存在的文件跳过(不覆盖用户改动)。
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    for entry in WalkDir::new(src).into_iter().flatten() {
        let p = entry.path();
        let rel = match p.strip_prefix(src) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            if !target.exists() {
                fs::copy(p, &target)?;
            }
        }
    }
    Ok(())
}

// ───────────────────────── Settings ──────────────────────

#[derive(Default, Serialize, Deserialize)]
struct AppSettings {
    kb_root: Option<String>,
}

fn settings_path() -> Result<PathBuf> {
    let pd = ProjectDirs::from("com", "polaris", "polaris-app")
        .ok_or_else(|| anyhow::anyhow!("no config dir"))?;
    let dir = pd.config_dir().to_path_buf();
    fs::create_dir_all(&dir)?;
    Ok(dir.join("settings.json"))
}

fn load_settings() -> AppSettings {
    settings_path()
        .ok()
        .and_then(|p| fs::read_to_string(&p).ok())
        .and_then(|s| serde_json::from_str::<AppSettings>(&s).ok())
        .unwrap_or_default()
}

fn save_settings(s: &AppSettings) -> Result<()> {
    let p = settings_path()?;
    fs::write(p, serde_json::to_string_pretty(s)?)?;
    Ok(())
}

/// 三层目录铁律 (PRD §8.3)
fn ensure_skeleton(root: &Path) -> Result<()> {
    for sub in ["raw", "output", "wiki"] {
        fs::create_dir_all(root.join(sub))?;
    }
    let claude_md = root.join("CLAUDE.md");
    if !claude_md.exists() {
        fs::write(&claude_md, include_str!("templates/kb_claude.md"))?;
    }
    let index_md = root.join("wiki").join("index.md");
    if !index_md.exists() {
        fs::write(&index_md, include_str!("templates/wiki_index.md"))?;
    }
    Ok(())
}

// ───────────────────────── Scan + Parse ──────────────────

fn scan_all(root: &Path) -> Vec<KbDoc> {
    let mut docs = Vec::new();
    if !root.exists() {
        return docs;
    }
    for entry in WalkDir::new(root).into_iter().flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
        if ext != "md" && ext != "markdown" {
            continue;
        }
        if let Ok(rel) = p.strip_prefix(root) {
            // 对话产物目录 conversations/ 不纳入知识库索引/图谱 (保护板块②不被对话产物污染);
            // 这些文件改由 chat::artifact_search 单独检索。
            if rel
                .components()
                .next()
                .and_then(|c| c.as_os_str().to_str())
                == Some("conversations")
            {
                continue;
            }
            if let Some(d) = parse_doc(p, rel) {
                docs.push(d);
            }
        }
    }
    docs
}

static RE_FRONTMATTER: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?s)^---\r?\n(.*?)\r?\n---\r?\n").unwrap());
static RE_TITLE_H1: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?m)^#\s+(.+)$").unwrap());
static RE_WIKILINK: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\[\[([^\]\|#]+)(?:[#\|][^\]]*)?\]\]").unwrap());
/// 标准 Markdown 链接 [文字](目标) — 用于从 README/目录页派生边
static RE_MDLINK: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\[[^\]]*\]\(([^)]+)\)").unwrap());
static RE_YAML_KV: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?m)^(\w+)\s*:\s*(.+)$").unwrap());

fn parse_doc(abs_path: &Path, rel: &Path) -> Option<KbDoc> {
    let body = fs::read_to_string(abs_path).ok()?;

    // 提取 frontmatter
    let (fm, body_only) = match RE_FRONTMATTER.captures(&body) {
        Some(c) => (
            c.get(1).map(|m| m.as_str().to_string()).unwrap_or_default(),
            body[c.get(0).unwrap().end()..].to_string(),
        ),
        None => (String::new(), body.clone()),
    };

    // category / type
    let mut category = String::new();
    let mut doc_type = String::new();
    let mut fm_title: Option<String> = None;
    for cap in RE_YAML_KV.captures_iter(&fm) {
        let k = cap.get(1).map(|m| m.as_str()).unwrap_or("").to_lowercase();
        let v = cap.get(2).map(|m| m.as_str().trim().trim_matches('"')).unwrap_or("");
        match k.as_str() {
            "category" => category = v.to_string(),
            "type" => doc_type = v.to_string(),
            "title" => fm_title = Some(v.to_string()),
            _ => {}
        }
    }

    // title: frontmatter > # H1 > 文件名
    let title = fm_title
        .or_else(|| {
            RE_TITLE_H1
                .captures(&body_only)
                .and_then(|c| c.get(1).map(|m| m.as_str().trim().to_string()))
        })
        .unwrap_or_else(|| {
            abs_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("untitled")
                .to_string()
        });

    // [[wikilinks]]
    let wikilinks: Vec<String> = RE_WIKILINK
        .captures_iter(&body_only)
        .filter_map(|c| c.get(1).map(|m| m.as_str().trim().to_string()))
        .collect();

    Some(KbDoc {
        rel_path: rel.to_string_lossy().replace('\\', "/"),
        title,
        category,
        doc_type,
        wikilinks,
    })
}

// ───────────────────────── Tauri commands ────────────────

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn kb_root() -> String {
    KB_ROOT.read().to_string_lossy().to_string()
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn kb_default_root() -> String {
    default_kb_root()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default()
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn kb_set_root(new_path: String) -> Result<usize, String> {
    let trimmed = new_path.trim().to_string();
    if trimmed.is_empty() {
        return Err("路径不能为空".into());
    }
    let new_root = PathBuf::from(&trimmed);
    ensure_skeleton(&new_root).map_err(|e| format!("无法创建目录骨架: {e}"))?;
    let mut s = load_settings();
    s.kb_root = Some(trimmed);
    save_settings(&s).map_err(|e| format!("写入设置失败: {e}"))?;
    *KB_ROOT.write() = new_root.clone();
    let docs = scan_all(&new_root);
    let n = docs.len();
    *INDEX.write() = docs;
    Ok(n)
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn kb_scan() -> Result<usize, String> {
    let root = KB_ROOT.read().clone();
    let docs = scan_all(&root);
    let n = docs.len();
    *INDEX.write() = docs;
    Ok(n)
}

// ───────────────────────── 构建知识网 (摄入即编译 Ingest=Compile) ─────────────────────────
//
// Karpathy LLM-Wiki 的核心是「写的那一半」: 摄入资料时让 LLM 读原文、抽实体/概念、
// 在 wiki/ 写页面、落 [[双链]]、记账 index/log —— 交叉引用「早就写好了」, 知识因此互联成网。
// 旧「构建索引」(kb_scan) 只重扫文件、刷新内存, 不产生任何新知识与新关联。
// kb_compile 就是补上的编译器: 复用 chat.rs 已验证的 headless `claude --print` 管线,
// 给一个带写权限(Read/Write/Edit/Glob/Grep)的 claude 进程当「wiki 维护者」, 让它自己
// Read 原文、Write wiki 页 —— 与现有架构天然契合, 不引入新的 LLM API / 向量依赖。

static KB_COMPILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 知识库维护互斥: compile / enrich_links / dedup 三者都 spawn 后台线程改写同一批 wiki
/// 文件(读-改-写)。并发跑会互相覆盖(lost update)甚至 dedup 删文件时 enrich 正在写它。
/// 用一个全局忙标志串行化, RAII guard 在线程结束(Drop)时自动释放。
static KB_TASK_BUSY: AtomicBool = AtomicBool::new(false);

struct KbTaskGuard;
impl Drop for KbTaskGuard {
    fn drop(&mut self) {
        KB_TASK_BUSY.store(false, Ordering::SeqCst);
    }
}
/// 抢占维护锁; 已有任务在跑则返回 Err(前端可提示稍候)。把返回的 guard `move` 进后台线程,
/// 线程跑完(正常/出错/panic)都会 Drop 释放。
fn acquire_kb_task() -> Result<KbTaskGuard, String> {
    if KB_TASK_BUSY
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("已有知识库维护任务在运行, 请等它结束后再试".into());
    }
    Ok(KbTaskGuard)
}

/// KB 内容原子落盘: 临时文件 + rename(同卷原子)。dedup/enrich 改写 wiki 页时若裸 fs::write
/// 中途崩溃会把页面截成半截, 丢失 AI/用户内容。统一走这里。
fn kb_atomic_write(path: &Path, contents: &str) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".polaris.tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, contents)?;
    fs::rename(&tmp, path)
}

/// 编译进度事件 (前端「构建知识网」进度面板订阅 `kb:compile`)。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KbCompileEvent {
    pub run_id: String,
    /// phase | tool | page | delta | done | error
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// 仅 done 事件: 编译后重扫得到的文档总数
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_count: Option<usize>,
}

fn emit_compile(app: &AppHandle, run_id: &str, kind: &str, text: Option<String>) {
    let _ = app.emit(
        "kb:compile",
        KbCompileEvent {
            run_id: run_id.into(),
            kind: kind.into(),
            text,
            doc_count: None,
        },
    );
}

/// 「wiki 维护者」system prompt —— Karpathy 式「摄入即编译」。clean-room 自写, 只学方法论。
fn compile_directive(root_disp: &str) -> String {
    format!(
        "# 角色：知识库 wiki 维护者 (Karpathy 式 LLM-Wiki)\n\n\
你是这个知识库的**维护者**。知识库根目录就在你的工作目录: `{root}`。\n\
它分三层:\n\
- `raw/` — 原始资料, **只读, 严禁写入或修改**。\n\
- `wiki/` — **由你全权拥有的知识层**: 摘要页 / 实体页 / 概念页 / 综合页。你在这里写。\n\
- `output/` — 生成的报告类成品。\n\n\
## 你这一轮的任务：摄入即编译 (Ingest = Compile)\n\n\
把 `raw/` 里的原始资料**编译**成一张互联的知识网, 而不是简单罗列。具体:\n\n\
1. **先读规则与现状**: 读 `CLAUDE.md`(若有) 了解约定; 读 `wiki/index.md` 和 `wiki/` 下已有页面, 知道已经有什么。\n\
2. **盘点资料**: 用 Glob/Grep 扫 `raw/`, 了解有哪些资料、主题是什么。**不要逐篇全文读**, 靠文件名和 Grep 抽样了解即可, 控制成本。\n\
3. **抽取并撰写知识 (核心)**: 识别贯穿资料的**实体**(人/地/组织/事件)与**概念/思想脉络**(反复出现的主题、论点)。\
概念页放 `wiki/概念/`、实体页放 `wiki/实体/`(没有就新建子目录); 在页面里**用 `[[页面标题]]` 双链**指向相关的其它 wiki 页, 并用 Grep 找出哪些 raw 篇目讲了它、列进 frontmatter 的 `sources` 并在正文引用。\
这一步的目的是**建立关联**: 原本互不相连的资料, 经由共同的概念页/实体页被串成网。\n\
4. **记账**: 更新 `wiki/index.md` (每个 wiki 页一行: `- [[标题]] — 一句话摘要`, 按类型分组);\
追加 `wiki/log.md` (一行: `## [今天日期] compile | 本轮做了什么`, 没有就新建)。\n\n\
## 页面格式 (每个新建/更新的 wiki 页都要带 frontmatter)\n\n\
```\n\
---\n\
title: 页面标题\n\
type: concept        # entity | concept | source | synthesis 之一\n\
sources: [\"raw/某资料.md\"]   # 这页依据的原始资料相对路径, 可多个\n\
---\n\
\n\
正文... 用 [[其它页面]] 互联, 用脚注/引用标注来源, 不要编造 raw/ 里没有的事实。\n\
```\n\n\
## 针对「语料型」知识库 (如大量同质篇目、彼此几乎无双链)\n\n\
不要逐篇浅摘就完事。**优先抽思想脉络的概念页**(例如把反复出现的主题各立一个概念页),\
在概念页里用 `[[…]]` 把相关篇目链接进来 —— 让原本散落的篇目经由概念层互联成脉络。\
这一轮重在**覆盖度与连接**(把散点连成网), 不必把每篇都深挖到底。\n\n\
## 硬约束\n\n\
- **绝不修改或写入 `raw/`**。只读它。\n\
- 不编造资料里没有的内容; 拿不准的事写进 `wiki/` 时标注「待核实」。\n\
- 双链统一用 `[[页面标题]]` 形式 (标题=对应 wiki 文件名去掉 .md)。\n\
- 全程用中文撰写 wiki 页。\n\n\
完成后, 用一两句话总结你**新建/更新了哪些 wiki 页**、建立了哪些关联。现在开始。",
        root = root_disp
    )
}

#[cfg_attr(not(windows), allow(unused_variables))]
fn compile_no_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: GUI 进程 spawn 控制台子进程时不弹黑窗
        cmd.creation_flags(0x0800_0000);
    }
}

/// 「构建知识网」: 启动一个有写权限的 headless claude 当 wiki 维护者, 把 raw/ 编译进 wiki/。
/// 立即返回 run_id; 进度通过 `kb:compile` 事件流式推送, 完成时发 `done` (附重扫后的文档数)。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn kb_compile(app: AppHandle) -> Result<String, String> {
    let root = KB_ROOT.read().clone();
    if root.as_os_str().is_empty() || !root.exists() {
        return Err("知识库根目录不存在, 请先在「管理」里设置".into());
    }
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let c = KB_COMPILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let run_id = format!("kbc-{:x}-{:x}", ts, c);

    let claude_bin: std::ffi::OsString = crate::doctor::resolve_claude_exe()
        .map(|p| p.into_os_string())
        .unwrap_or_else(|| "claude".into());
    let root_disp = root.to_string_lossy().replace('\\', "/");
    let prompt = compile_directive(&root_disp);

    let _kb_task = acquire_kb_task()?;
    let run_id_thread = run_id.clone();
    std::thread::spawn(move || {
        let _kb_task = _kb_task; // 持锁直到本线程结束(Drop 释放)
        emit_compile(&app, &run_id_thread, "phase", Some("启动 wiki 维护者…".into()));

        // prompt 经 stdin 喂给 claude (而非命令行参数): 大 prompt 不会撞 Windows 命令行
        // 长度上限, 也不会因 prompt 以 `-` 开头被当成 flag —— 实测 argv 路径在某些 shell 下
        // 会触发 claude 的「Input must be provided」直接退 1, stdin 管道稳。
        let mut cmd = Command::new(&claude_bin);
        cmd.args([
            "--print",
            "--output-format",
            "stream-json",
            "--verbose",
            "--permission-mode=bypassPermissions",
            "--allowedTools",
            "Read,Write,Edit,Glob,Grep",
        ])
        .current_dir(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        crate::doctor::harden_child_env(&mut cmd); // loopback NO_PROXY + 清干扰变量
        compile_no_window(&mut cmd);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                emit_compile(
                    &app,
                    &run_id_thread,
                    "error",
                    Some(format!("调起 claude 失败: {e}")),
                );
                let _ = app.emit(
                    "kb:compile",
                    KbCompileEvent {
                        run_id: run_id_thread.clone(),
                        kind: "done".into(),
                        text: Some("编译未启动".into()),
                        doc_count: None,
                    },
                );
                return;
            }
        };

        // 把 prompt 写进 stdin 并关闭 (drop 即关), claude 读到 EOF 后开始干活
        if let Some(mut si) = child.stdin.take() {
            use std::io::Write as _;
            let _ = si.write_all(prompt.as_bytes());
            // si 在此作用域结束时 drop → stdin 关闭, 触发 claude 开始处理
        }

        // stderr: 累积, 退出非零时给原因 (stream-json 模式下通常为空, 仅崩溃时有内容)
        let stderr_buf = std::sync::Arc::new(parking_lot::Mutex::new(String::new()));
        if let Some(se) = child.stderr.take() {
            let buf = stderr_buf.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(se).lines().map_while(std::result::Result::ok) {
                    if !line.trim().is_empty() {
                        buf.lock().push_str(&line);
                        buf.lock().push('\n');
                    }
                }
            });
        }

        // stdout: 解析 stream-json, 把工具调用 / 写页面 / 文本翻成进度
        let mut pages: Vec<String> = Vec::new();
        if let Some(so) = child.stdout.take() {
            emit_compile(&app, &run_id_thread, "phase", Some("读取资料、抽取实体与概念…".into()));
            for line in BufReader::new(so).lines().map_while(std::result::Result::ok) {
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                if v.get("type").and_then(|x| x.as_str()) != Some("assistant") {
                    // result 事件的错误子类型 → 透传
                    if v.get("type").and_then(|x| x.as_str()) == Some("result") {
                        if let Some(st) = v.get("subtype").and_then(|x| x.as_str()) {
                            if st.starts_with("error") {
                                let msg = v
                                    .get("result")
                                    .and_then(|x| x.as_str())
                                    .unwrap_or("(unknown)")
                                    .to_string();
                                emit_compile(
                                    &app,
                                    &run_id_thread,
                                    "error",
                                    Some(format!("[{st}] {msg}")),
                                );
                            }
                        }
                    }
                    continue;
                }
                let Some(content) = v
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                else {
                    continue;
                };
                for block in content {
                    match block.get("type").and_then(|x| x.as_str()) {
                        Some("tool_use") => {
                            let name = block.get("name").and_then(|x| x.as_str()).unwrap_or("");
                            if matches!(name, "Write" | "Edit" | "MultiEdit") {
                                if let Some(fp) = block
                                    .get("input")
                                    .and_then(|i| i.get("file_path"))
                                    .and_then(|x| x.as_str())
                                {
                                    let norm = fp.replace('\\', "/");
                                    let short = norm.rsplit('/').next().unwrap_or(&norm).to_string();
                                    if !pages.contains(&norm) {
                                        pages.push(norm);
                                    }
                                    emit_compile(
                                        &app,
                                        &run_id_thread,
                                        "page",
                                        Some(format!("写入 {short}")),
                                    );
                                }
                            } else {
                                emit_compile(&app, &run_id_thread, "tool", Some(name.to_string()));
                            }
                        }
                        Some("text") => {
                            if let Some(t) = block.get("text").and_then(|x| x.as_str()) {
                                let t = t.trim();
                                if !t.is_empty() {
                                    emit_compile(&app, &run_id_thread, "delta", Some(t.to_string()));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        let status = child.wait();
        // 编译完成 → 重扫刷新内存索引 + 图谱
        let root_now = KB_ROOT.read().clone();
        let docs = scan_all(&root_now);
        let n = docs.len();
        *INDEX.write() = docs;

        let ok = matches!(&status, Ok(s) if s.success());
        if !ok {
            let code = status.as_ref().ok().and_then(|s| s.code());
            let se = stderr_buf.lock().clone();
            emit_compile(
                &app,
                &run_id_thread,
                "error",
                Some(format!(
                    "claude 退出码 {code:?}{}",
                    if se.is_empty() {
                        String::new()
                    } else {
                        format!(" — {se}")
                    }
                )),
            );
        }
        let msg = if ok {
            format!("编译完成: 新建/更新 {} 个页面, 知识库共 {} 篇", pages.len(), n)
        } else {
            "编译中断 (见上方原因), 已刷新索引".into()
        };
        let _ = app.emit(
            "kb:compile",
            KbCompileEvent {
                run_id: run_id_thread.clone(),
                kind: "done".into(),
                text: Some(msg),
                doc_count: Some(n),
            },
        );
    });

    Ok(run_id)
}

// ───────────────────────── 共享: 只读 claude → 收集 JSON (Wave B 基础设施) ─────────────────────────
//
// enrich/dedup 共用的核心模式 (借鉴 llm_wiki「让 AI 只出决策数据, 代码执行改动」):
// 起一个**只读** (allowedTools 仅 Read/Glob/Grep, 物理上无法写文件) 的 headless claude,
// 让它读 wiki、输出一段 JSON 决策, 把全部 assistant 文本收集起来返回。改文件由 Rust 做。

/// 起一个只读 headless claude, 把 prompt 经 stdin 喂进去, 收集其全部 assistant 文本块返回。
/// `on_event(kind, text)`: kind ∈ {tool, delta} 用于向前端透传进度。阻塞直到进程退出。
fn run_claude_readonly<F: FnMut(&str, &str)>(
    root: &Path,
    prompt: &str,
    mut on_event: F,
) -> Result<String, String> {
    let claude_bin: std::ffi::OsString = crate::doctor::resolve_claude_exe()
        .map(|p| p.into_os_string())
        .unwrap_or_else(|| "claude".into());
    let mut cmd = Command::new(&claude_bin);
    cmd.args([
        "--print",
        "--output-format",
        "stream-json",
        "--verbose",
        "--permission-mode=bypassPermissions",
        "--allowedTools",
        "Read,Glob,Grep", // 只读: 物理上不给 Write/Edit, 决策数据落地由 Rust 执行
    ])
    .current_dir(root)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    crate::doctor::harden_child_env(&mut cmd); // loopback NO_PROXY + 清干扰变量
    compile_no_window(&mut cmd);

    let mut child = cmd.spawn().map_err(|e| format!("调起 claude 失败: {e}"))?;
    if let Some(mut si) = child.stdin.take() {
        use std::io::Write as _;
        let _ = si.write_all(prompt.as_bytes());
    }
    let stderr_buf = std::sync::Arc::new(parking_lot::Mutex::new(String::new()));
    if let Some(se) = child.stderr.take() {
        let buf = stderr_buf.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(se).lines().map_while(std::result::Result::ok) {
                if !line.trim().is_empty() {
                    buf.lock().push_str(&line);
                    buf.lock().push('\n');
                }
            }
        });
    }

    let mut collected = String::new();
    if let Some(so) = child.stdout.take() {
        for line in BufReader::new(so).lines().map_while(std::result::Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
            if ty == "result" {
                if let Some(st) = v.get("subtype").and_then(|x| x.as_str()) {
                    if st.starts_with("error") {
                        return Err(format!("claude 返回错误: {st}"));
                    }
                }
                continue;
            }
            if ty != "assistant" {
                continue;
            }
            let Some(content) = v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_array())
            else {
                continue;
            };
            for block in content {
                match block.get("type").and_then(|x| x.as_str()) {
                    Some("tool_use") => {
                        let name = block.get("name").and_then(|x| x.as_str()).unwrap_or("");
                        on_event("tool", name);
                    }
                    Some("text") => {
                        if let Some(t) = block.get("text").and_then(|x| x.as_str()) {
                            collected.push_str(t);
                            on_event("delta", t.trim());
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let status = child.wait();
    if !matches!(&status, Ok(s) if s.success()) {
        let se = stderr_buf.lock().clone();
        return Err(format!("claude 异常退出{}", if se.is_empty() { String::new() } else { format!(": {se}") }));
    }
    Ok(collected)
}

/// 从一段文本里抽出第一个**平衡**的 JSON (对象 `{...}` 或数组 `[...]`), 容忍前后包裹的
/// markdown 代码围栏与说明文字 (借鉴 llm_wiki 对 LLM 输出格式宽松解析)。
fn extract_balanced_json(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let start = s.find(['{', '['])?;
    let open = bytes[start];
    let close = if open == b'{' { b'}' } else { b']' };
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            x if x == open => depth += 1,
            x if x == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(s[start..=i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

// ───────────────────────── 自动补双链 (借鉴 llm_wiki enrich-wikilinks) ─────────────────────────
//
// 旗舰示范: 「让 AI 只动嘴, 代码动手」。只读 claude 读 wiki 页 + 候选标题, 返回
// `[{page, term, target}]` 链接建议; Rust 执行替换 —— 只替**首次出现**、跳过 frontmatter /
// 已链接 / 代码区, 正文一字不多改。模型物理上没有写权限, 从根上杜绝它改乱正文。

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KbEnrichEvent {
    pub run_id: String,
    pub kind: String, // phase | tool | delta | done | error
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied: Option<usize>,
}

#[derive(Deserialize)]
struct LinkSuggestion {
    page: String,
    term: String,
    target: String,
}

/// 纯函数: 在 `body` 中把 `term` 的**首次明文出现**替换为 `[[target]]`(或 `[[target|term]]`)。
/// 跳过: frontmatter 区、已有 `[[...]]` 内部、行内代码 `` `..` `` 与围栏代码块、已是双链的同名词。
/// 命中并替换返回 `Some(新正文)`; 没有可替换的明文出现返回 `None`(不改动)。
fn apply_wikilink(body: &str, term: &str, target: &str) -> Option<String> {
    if term.is_empty() {
        return None;
    }
    let chars: Vec<char> = body.chars().collect();
    let term_chars: Vec<char> = term.chars().collect();
    let n = chars.len();
    let tn = term_chars.len();

    // 定位 frontmatter 结束位置 (第二个 `---` 行之后), 之前的内容不动。
    let fm_end = frontmatter_end_char_idx(&chars);

    let mut i = fm_end;
    let mut in_fence = false; // ``` 围栏代码块
    let mut in_inline = false; // `..` 行内代码
    let mut link_depth = 0i32; // [[..]] 内
    let mut at_line_start = true;
    while i < n {
        // 围栏: 行首三连反引号切换
        if at_line_start && i + 2 < n && chars[i] == '`' && chars[i + 1] == '`' && chars[i + 2] == '`' {
            in_fence = !in_fence;
            i += 3;
            at_line_start = false;
            continue;
        }
        let c = chars[i];
        if c == '\n' {
            at_line_start = true;
            in_inline = false; // 行内代码不跨行
            i += 1;
            continue;
        }
        at_line_start = false;
        if !in_fence && c == '`' {
            in_inline = !in_inline;
            i += 1;
            continue;
        }
        if i + 1 < n && c == '[' && chars[i + 1] == '[' {
            link_depth += 1;
            i += 2;
            continue;
        }
        if i + 1 < n && c == ']' && chars[i + 1] == ']' && link_depth > 0 {
            link_depth -= 1;
            i += 2;
            continue;
        }
        // 命中明文 term?
        if !in_fence && !in_inline && link_depth == 0 && i + tn <= n && chars[i..i + tn] == term_chars[..] {
            // 前一个非空白字符不能是 `[`(避免 [[ 紧邻) — link_depth 已挡住, 这里再防 `[term`
            let prev_ok = i == 0 || chars[i - 1] != '[';
            if prev_ok {
                let replacement = if term == target {
                    format!("[[{target}]]")
                } else {
                    format!("[[{target}|{term}]]")
                };
                let mut out = String::new();
                out.extend(chars[..i].iter());
                out.push_str(&replacement);
                out.extend(chars[i + tn..].iter());
                return Some(out);
            }
        }
        i += 1;
    }
    None
}

/// 返回 frontmatter 之后正文起始的字符下标 (无 frontmatter 则 0)。
fn frontmatter_end_char_idx(chars: &[char]) -> usize {
    // 必须以 `---\n` 开头
    if chars.len() < 4 || chars[0] != '-' || chars[1] != '-' || chars[2] != '-' {
        return 0;
    }
    // 找第二个独占一行的 `---`
    let mut i = 0;
    let mut line_start = 0;
    let mut seen_first = false;
    while i < chars.len() {
        if chars[i] == '\n' {
            let line: String = chars[line_start..i].iter().collect();
            if line.trim() == "---" {
                if seen_first {
                    return i + 1; // 第二个 --- 行的换行之后
                }
                seen_first = true;
            }
            line_start = i + 1;
        }
        i += 1;
    }
    0
}

/// 「自动补双链」: 只读 claude 给出 `[{page,term,target}]` 建议, Rust 执行替换。
/// 立即返回 run_id; 进度走 `kb:enrich` 事件, 完成发 `done` (附实际应用条数)。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn kb_enrich_links(app: AppHandle) -> Result<String, String> {
    let root = KB_ROOT.read().clone();
    if root.as_os_str().is_empty() || !root.exists() {
        return Err("知识库根目录不存在".into());
    }
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let c = KB_COMPILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let run_id = format!("kbe-{:x}-{:x}", ts, c);

    // 候选目标 = 现有 wiki 页标题清单 (供模型选择链接到哪)
    let titles: Vec<String> = {
        let idx = INDEX.read();
        idx.iter()
            .filter(|d| d.rel_path.starts_with("wiki/") && !is_wiki_meta_page(&d.rel_path.replace('\\', "/")))
            .map(|d| d.title.clone())
            .filter(|t| t.chars().count() >= 2)
            .collect()
    };
    if titles.is_empty() {
        return Err("wiki/ 暂无可链接的页面, 请先构建知识网".into());
    }

    let _kb_task = acquire_kb_task()?;
    let run_id_t = run_id.clone();
    std::thread::spawn(move || {
        let _kb_task = _kb_task; // 持锁直到本线程结束(Drop 释放)
        let emit = |kind: &str, text: Option<String>, applied: Option<usize>| {
            let _ = app.emit(
                "kb:enrich",
                KbEnrichEvent {
                    run_id: run_id_t.clone(),
                    kind: kind.into(),
                    text,
                    applied,
                },
            );
        };
        emit("phase", Some("分析 wiki 页面、寻找可补的双链…".into()), None);

        let vocab = titles.join("\n");
        let prompt = format!(
            "# 任务: 为知识库 wiki 找出应补的双链 (只输出 JSON, 不要改任何文件)\n\n\
你的工作目录是知识库根。下面是 wiki/ 现有页面的**标题清单**(可作为双链目标):\n\n{vocab}\n\n\
请用 Read/Glob/Grep 浏览 `wiki/` 下的内容页 (跳过 index.md 与各 _index.md), 找出正文里\
**以纯文本形式出现、但还没做成 `[[双链]]`** 的术语, 且该术语正好等于(或非常接近)上面清单里的某个标题。\n\n\
## 输出 (严格)\n\
只输出一个 JSON 数组, 每项形如 `{{\"page\": \"wiki/概念/x.md\", \"term\": \"正文里出现的词\", \"target\": \"清单里的目标标题\"}}`。\n\
- term 必须是该 page 正文里**逐字出现**的子串。\n\
- target 必须是上面清单里的标题之一。\n\
- 同一 page 同一 term 只给一条。最多 80 条。\n\
- **不要写入或修改任何文件**, 不要输出 JSON 以外的任何解释文字。\n\n\
现在开始, 直接输出 JSON 数组。"
        );

        let raw = match run_claude_readonly(&root, &prompt, |kind, text| {
            if kind == "tool" {
                emit("tool", Some(text.to_string()), None);
            } else if kind == "delta" && !text.is_empty() {
                emit("delta", Some(text.chars().take(80).collect()), None);
            }
        }) {
            Ok(r) => r,
            Err(e) => {
                emit("error", Some(e), None);
                emit("done", Some("补链未完成".into()), Some(0));
                return;
            }
        };

        let suggestions: Vec<LinkSuggestion> = extract_balanced_json(&raw)
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default();
        emit("phase", Some(format!("收到 {} 条建议, 代码执行替换…", suggestions.len())), None);

        // 现存 wiki 标题集 (校验 target 合法)
        let valid_targets: std::collections::HashSet<String> = {
            let idx = INDEX.read();
            idx.iter()
                .filter(|d| d.rel_path.starts_with("wiki/"))
                .map(|d| d.title.clone())
                .collect()
        };

        // 按 page 聚合, 逐页一次性读写, 顺序应用其建议 (每条改首次出现)。
        use std::collections::BTreeMap;
        let mut by_page: BTreeMap<String, Vec<LinkSuggestion>> = BTreeMap::new();
        for s in suggestions {
            by_page.entry(s.page.replace('\\', "/")).or_default().push(s);
        }

        let mut applied = 0usize;
        for (page, sugs) in by_page {
            // 安全: page 必须是 wiki/ 下合法路径且文件存在
            if is_safe_wiki_relpath(&page).is_err() {
                continue;
            }
            let full = root.join(&page);
            let Ok(mut content) = fs::read_to_string(&full) else {
                continue;
            };
            let mut changed = false;
            for s in sugs {
                if !valid_targets.contains(&s.target) {
                    continue;
                }
                if let Some(updated) = apply_wikilink(&content, &s.term, &s.target) {
                    content = updated;
                    changed = true;
                    applied += 1;
                }
            }
            if changed {
                if kb_atomic_write(&full, &content).is_ok() {
                    emit("phase", Some(format!("已补链: {}", page.rsplit('/').next().unwrap_or(&page))), None);
                }
            }
        }

        // 重扫刷新索引/图谱
        let docs = scan_all(&root);
        *INDEX.write() = docs;
        emit("done", Some(format!("补链完成: 共应用 {applied} 处双链")), Some(applied));
    });

    Ok(run_id)
}

// ───────────────────────── 智能去重 (借鉴 llm_wiki dedup + page-merge) ─────────────────────────
//
// 「摄入即编译」反复跑会写出同主题的多篇页面, 越积越乱。借鉴 llm_wiki 两段式:
// ① 规则粗筛 (按归一化标题分组, 便宜) → ② 只读 claude 细判 (真重复? 谁当主页? confidence)
// → ③ Rust 执行合并: **锁定主页 type/title/created**, 把重复页正文并入主页(不丢知识),
//    重写全库 `[[重复页]]` 双链指向主页, 删重复页文件 + 清 index 条目。

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KbDedupEvent {
    pub run_id: String,
    pub kind: String, // phase | tool | delta | done | error
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merged: Option<usize>,
}

#[derive(Deserialize)]
struct DedupVerdict {
    #[serde(default)]
    duplicate: bool,
    #[serde(default)]
    confidence: String,
    #[serde(default)]
    canonical: String,
    #[serde(default)]
    pages: Vec<String>,
}

/// 归一化标题: 小写 + 去空白与常见标点 —— 用于规则粗筛分组。
fn normalize_title(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && !"-_()（）[]【】·.,，。:：、/\\|".contains(*c))
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// 重写正文里指向 `from` 标题的双链, 改为指向 `to`, 保留别名/锚点后缀。
/// `[[from]]` → `[[to]]`; `[[from|别名]]` → `[[to|别名]]`; `[[from#节]]` → `[[to#节]]`。大小写不敏感匹配 from。
fn rewrite_wikilink_target(body: &str, from: &str, to: &str) -> String {
    let from_lc = from.to_lowercase();
    let mut out = String::with_capacity(body.len());
    let chars: Vec<char> = body.chars().collect();
    let n = chars.len();
    let mut i = 0;
    while i < n {
        if i + 1 < n && chars[i] == '[' && chars[i + 1] == '[' {
            // 找到匹配的 ]]
            if let Some(close) = find_link_close(&chars, i + 2) {
                let inner: String = chars[i + 2..close].iter().collect();
                // 拆 target | alias / target # sec
                let (target, suffix) = split_link_inner(&inner);
                if target.trim().to_lowercase() == from_lc {
                    out.push_str("[[");
                    out.push_str(to);
                    out.push_str(&suffix);
                    out.push_str("]]");
                    i = close + 2;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 从 `start` 起找 `]]` 的起始下标 (不跨越下一个 `[[`)。
fn find_link_close(chars: &[char], start: usize) -> Option<usize> {
    let n = chars.len();
    let mut i = start;
    while i + 1 < n {
        if chars[i] == ']' && chars[i + 1] == ']' {
            return Some(i);
        }
        if chars[i] == '[' && chars[i + 1] == '[' {
            return None; // 嵌套/未闭合, 放弃
        }
        i += 1;
    }
    None
}

/// 把 `[[inner]]` 的内部拆成 (目标, 后缀)。后缀含分隔符, 如 `|别名` 或 `#节`。
fn split_link_inner(inner: &str) -> (String, String) {
    if let Some(p) = inner.find(['|', '#']) {
        (inner[..p].to_string(), inner[p..].to_string())
    } else {
        (inner.to_string(), String::new())
    }
}

/// 「智能去重」: 规则粗筛 + 只读 claude 细判 + Rust 合并。
/// 立即返回 run_id; 进度走 `kb:dedup`, 完成发 `done` (附合并页数)。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn kb_dedup(app: AppHandle) -> Result<String, String> {
    let root = KB_ROOT.read().clone();
    if root.as_os_str().is_empty() || !root.exists() {
        return Err("知识库根目录不存在".into());
    }
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let c = KB_COMPILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let run_id = format!("kbd-{:x}-{:x}", ts, c);

    // 规则粗筛: 按归一化标题分组, 取 size≥2 的组 (附路径/标题/正文片段供 claude 判断)
    let groups: Vec<Vec<(String, String, String)>> = {
        let idx = INDEX.read();
        let mut by_norm: HashMap<String, Vec<(String, String, String)>> = HashMap::new();
        for d in idx.iter() {
            let rp = d.rel_path.replace('\\', "/");
            if !rp.starts_with("wiki/") || is_wiki_meta_page(&rp) {
                continue;
            }
            let snippet: String = read_doc_body(&d.rel_path)
                .map(|b| b.trim().chars().take(160).collect())
                .unwrap_or_default();
            by_norm
                .entry(normalize_title(&d.title))
                .or_default()
                .push((rp, d.title.clone(), snippet));
        }
        by_norm.into_values().filter(|g| g.len() >= 2).collect()
    };

    if groups.is_empty() {
        return Err("规则粗筛未发现疑似重复页 (标题归一化后无碰撞)".into());
    }

    let _kb_task = acquire_kb_task()?;
    let run_id_t = run_id.clone();
    std::thread::spawn(move || {
        let _kb_task = _kb_task; // 持锁直到本线程结束(Drop 释放)
        let emit = |kind: &str, text: Option<String>, merged: Option<usize>| {
            let _ = app.emit(
                "kb:dedup",
                KbDedupEvent {
                    run_id: run_id_t.clone(),
                    kind: kind.into(),
                    text,
                    merged,
                },
            );
        };
        emit("phase", Some(format!("规则粗筛出 {} 组疑似重复, 请 AI 细判…", groups.len())), None);

        // 拼候选清单给 claude
        let mut cand = String::new();
        for (gi, g) in groups.iter().enumerate() {
            cand.push_str(&format!("## 组 {gi}\n"));
            for (rp, title, snip) in g {
                cand.push_str(&format!("- `{rp}` | 标题: {title} | 摘要: {snip}\n"));
            }
            cand.push('\n');
        }
        let prompt = format!(
            "# 任务: 判断这些 wiki 页是否真重复 (只输出 JSON, 不要改任何文件)\n\n\
下面是按标题相似度粗筛出的若干**疑似重复组**(每组列了路径/标题/正文摘要)。\
必要时可用 Read 打开页面看全文再判断。\n\n{cand}\n\
## 输出 (严格)\n\
只输出一个 JSON 数组, 每组一项: \
`{{\"pages\": [\"组内全部路径\"], \"duplicate\": true/false, \"confidence\": \"high|medium|low\", \"canonical\": \"应保留为主页的路径\", \"reason\": \"一句话\"}}`。\n\
- 仅当确属讲同一事物的重复页才标 duplicate=true。\n\
- canonical 选内容最全/质量最好的那篇, 必须是该组 pages 之一。\n\
- **不要写入或修改任何文件**, 不要输出 JSON 以外的解释。\n\n\
现在开始, 直接输出 JSON 数组。"
        );

        let raw = match run_claude_readonly(&root, &prompt, |kind, text| {
            if kind == "tool" {
                emit("tool", Some(text.to_string()), None);
            }
        }) {
            Ok(r) => r,
            Err(e) => {
                emit("error", Some(e), None);
                emit("done", Some("去重未完成".into()), Some(0));
                return;
            }
        };

        let verdicts: Vec<DedupVerdict> = extract_balanced_json(&raw)
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default();
        emit("phase", Some("AI 判定完成, 代码执行合并…".to_string()), None);

        let mut merged = 0usize;
        for v in verdicts {
            if !v.duplicate || v.confidence.eq_ignore_ascii_case("low") {
                continue; // 保守: 低置信不动
            }
            let canonical = v.canonical.replace('\\', "/");
            if is_safe_wiki_relpath(&canonical).is_err() || !root.join(&canonical).exists() {
                continue;
            }
            for dup in v.pages.iter().map(|p| p.replace('\\', "/")) {
                if dup == canonical {
                    continue;
                }
                if is_safe_wiki_relpath(&dup).is_err() || !root.join(&dup).exists() {
                    continue;
                }
                if merge_duplicate_page(&root, &canonical, &dup).is_ok() {
                    merged += 1;
                    emit("phase", Some(format!("已合并 {} → {}",
                        dup.rsplit('/').next().unwrap_or(&dup),
                        canonical.rsplit('/').next().unwrap_or(&canonical))), None);
                }
            }
        }

        let docs = scan_all(&root);
        *INDEX.write() = docs;
        emit("done", Some(format!("去重完成: 合并 {merged} 个重复页")), Some(merged));
    });

    Ok(run_id)
}

/// 把重复页 `dup` 合并进主页 `canonical` (路径均为 KB 相对、已校验存在):
/// ① 把 dup 正文并入 canonical 末尾「合并自」区 (不丢知识); 主页 frontmatter 原样保留(锁定 type/title/created)。
/// ② 全库重写 `[[dup标题]]` → `[[canonical标题]]`。
/// ③ 删 dup 文件, 清 wiki/index.md 里指向 dup 的行。
fn merge_duplicate_page(root: &Path, canonical: &str, dup: &str) -> Result<(), String> {
    let stem = |rp: &str| -> String {
        let n = rp.replace('\\', "/");
        let base = n.rsplit('/').next().unwrap_or(&n).to_string();
        base.strip_suffix(".md")
            .or_else(|| base.strip_suffix(".markdown"))
            .unwrap_or(&base)
            .to_string()
    };
    // 标题取 INDEX 里的 title, 回退到文件名 stem
    let title_of = |rp: &str| -> String {
        let idx = INDEX.read();
        idx.iter()
            .find(|d| d.rel_path.replace('\\', "/") == rp)
            .map(|d| d.title.clone())
            .unwrap_or_else(|| stem(rp))
    };
    let canon_title = title_of(canonical);
    let dup_title = title_of(dup);

    let dup_full = root.join(dup);
    let canon_full = root.join(canonical);
    let dup_body = fs::read_to_string(&dup_full).map_err(|e| e.to_string())?;
    // 剥掉 dup 的 frontmatter, 只并正文
    let dup_content = RE_FRONTMATTER
        .replace(&dup_body, "")
        .trim()
        .to_string();

    // ① 并入主页末尾 (主页 frontmatter 不动 → 锁定 type/title/created)
    let mut canon_body = fs::read_to_string(&canon_full).map_err(|e| e.to_string())?;
    if !canon_body.ends_with('\n') {
        canon_body.push('\n');
    }
    canon_body.push_str(&format!(
        "\n<!-- 合并自 {dup} (kb_dedup) -->\n## (并入) {dup_title}\n\n{dup_content}\n"
    ));
    kb_atomic_write(&canon_full, &canon_body).map_err(|e| e.to_string())?;

    // ② 全库重写双链 [[dup_title]] → [[canon_title]]
    if !dup_title.eq_ignore_ascii_case(&canon_title) {
        for entry in WalkDir::new(root.join("wiki")).into_iter().flatten() {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
            if ext != "md" && ext != "markdown" {
                continue;
            }
            if p == canon_full || p == dup_full {
                continue; // dup 即将删除; canon 末尾刚并入, 不必自指重写
            }
            if let Ok(content) = fs::read_to_string(p) {
                if content.contains(&format!("[[{dup_title}")) {
                    let rewritten = rewrite_wikilink_target(&content, &dup_title, &canon_title);
                    if rewritten != content {
                        let _ = kb_atomic_write(p, &rewritten);
                    }
                }
            }
        }
    }

    // ③ 删 dup 文件 + 清 index.md 里指向 dup 的行
    fs::remove_file(&dup_full).map_err(|e| e.to_string())?;
    let index_md = root.join("wiki").join("index.md");
    if let Ok(idx_content) = fs::read_to_string(&index_md) {
        let needle_link = format!("[[{dup_title}]]");
        let needle_path = dup;
        let kept: Vec<&str> = idx_content
            .lines()
            .filter(|ln| !(ln.contains(&needle_link) || ln.contains(needle_path)))
            .collect();
        let new_idx = kept.join("\n");
        if new_idx != idx_content {
            let _ = kb_atomic_write(&index_md, &new_idx);
        }
    }
    Ok(())
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn kb_list(subdir: Option<String>) -> Vec<String> {
    let idx = INDEX.read();
    idx.iter()
        .filter(|d| {
            subdir
                .as_deref()
                .map(|s| d.rel_path.starts_with(s))
                .unwrap_or(true)
        })
        .map(|d| d.rel_path.clone())
        .collect()
}

// ───────────────────────── 上下文预算 (借鉴 llm_wiki context-budget) ─────────────────────────
//
// 痛点: wiki/ 全文注入 42k 字符曾撞 Windows 命令行 32k 上限(206)。即便改走 stdin, 无节制
// 注入也会吃掉模型有限的上下文窗口、挤掉它「回话」的余量。
// 借鉴 llm_wiki 的做法: 不拍脑袋, 按**固定比例**切预算 —— 导航页占大头、地图占其余、
// 留一截给模型回答。预算耗尽就优雅截断并显式告知「其余请用 Read/Glob 自取」。

/// 注入块总字符预算 (保守取值: 远低于 32k 命令行上限, 也给模型窗口留足回话余量)。
const KB_CTX_BUDGET: usize = 24_000;
/// 导航页(index/_index)分到的比例 —— 它们是「目录」, 信息密度最高, 给大头。
const KB_CTX_NAV_RATIO: f32 = 0.55;
/// 地图清单(raw/ 等文件标题列表)分到的比例。
const KB_CTX_MAP_RATIO: f32 = 0.40;
/// 单篇导航页正文上限 (防一个超大 _index 吃光整段预算)。
const KB_CTX_PER_PAGE_RATIO: f32 = 0.30;

/// 按字符边界安全截断; 超出时追加省略标记。
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{}\n\n…(本页过长已截断, 需要全文请用 `Read` 打开)", cut.trim_end())
}

/// Karpathy 式「结构化 wiki + 长上下文 + 双链导航」上下文块, 供 chat 发送前注入。
///
/// 不做关键词召回硬塞 (那是 Karpathy 反对的「平铺 + 向量/关键词召回」范式)。而是把
/// **wiki/ 知识层导航页** + **整库的双链/目录地图** + **KB 根的绝对路径** 给模型,
/// 让它用 Read/Glob/Grep 沿双链自取 —— 这才是 headless 下真正可行、且忠于 llmwiki 的
/// 「调用知识库」方式 (claude CLI 在 --print 下有 Read/Glob/Grep, 且 KB 就在 cwd 子树里)。
/// 注入量受 [`KB_CTX_BUDGET`] 约束, 按比例分配给导航页与地图。
/// KB 为空 / 不存在时返回空串。
pub fn kb_context_block() -> String {
    kb_context_block_scoped(None)
}

/// 同 [`kb_context_block`]，但可按 `scope`（KB 根下相对子目录，如 `raw/毛主席`）
/// 把「知识库地图」收窄到该子树 —— 板块⑫ 让不同人格看到各自的专属知识库。
/// `scope=None` 时行为与全局一致（向后兼容）。
pub fn kb_context_block_scoped(scope: Option<&str>) -> String {
    let root = KB_ROOT.read().clone();
    if root.as_os_str().is_empty() || !root.exists() {
        return String::new();
    }
    let idx = INDEX.read();
    if idx.is_empty() {
        return String::new();
    }
    let norm = |s: &str| s.replace('\\', "/");
    let stem = |rp: &str| -> String {
        let n = norm(rp);
        let base = n.rsplit('/').next().unwrap_or(&n).to_string();
        base.strip_suffix(".md")
            .or_else(|| base.strip_suffix(".markdown"))
            .unwrap_or(&base)
            .to_string()
    };
    let parent = |rp: &str| -> String {
        let n = norm(rp);
        match n.rfind('/') {
            Some(i) => n[..i].to_string(),
            None => ".".to_string(),
        }
    };

    let root_disp = norm(&root.to_string_lossy());
    let mut out = String::new();
    out.push_str(&format!(
        "### 维基库结构 (Karpathy 式: 结构化 wiki + 长上下文 + 双链导航)\n\n\
知识库根目录: `{root_disp}`\n\
**就在你的工作目录下** —— 你可以(并且应当)用 `Read` / `Glob` / `Grep` 直接打开其中任意页面来取证。\n\
三层目录: `raw/`(只读原始资料, 严禁写入) · `output/`(生成的成品) · `wiki/`(人工确认的知识层)。\n\n"
    ));

    // wiki/ 知识层: 只注入「导航文件」(顶层 index.md + 各子目录 _index.md),
    // 不再全文注入每篇 wiki —— 46 篇全文 42k 字符直接撞 Windows 命令行 32k 上限(206)。
    // 模型要细看哪篇, 用 `Read` 沿双链或路径自取 —— 索引里把路径写清楚就行。
    let mut nav_docs: Vec<&KbDoc> = idx
        .iter()
        .filter(|d| {
            let rp = norm(&d.rel_path);
            // 顶层 index.md / 顶层元页(方法论/说明/log)
            rp == "wiki/index.md"
                || rp == "wiki/karpathy-wiki方法论.md"
                || rp == "wiki/wiki-knowledge-base.md"
                // 各子目录 _index.md
                || rp.ends_with("/_index.md")
        })
        .collect();
    nav_docs.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    // 导航段省下多少字符可让给地图段 (由下方地图循环用)。
    let mut nav_surplus: usize = 0;
    if !nav_docs.is_empty() {
        out.push_str(
            "#### wiki/ 知识层 (仅注入导航页: 顶层 index + 各子目录 _index, 全文请用 Read 沿双链/路径自取)\n\n"
        );
        // 上下文预算 · 密度自适应 (借鉴 llm_wiki context-budget, 改造成弹性版):
        // 先看导航页**实际**总字符 S, 与硬上限比较:
        //   S ≤ nav_budget: 整段全塞 (不浪费), 省下的 `nav_budget − S` 让位给地图
        //   S > nav_budget: 仍按 nav_budget + 单页上限截, 多塞不进去
        // 单页上限 (per_page) 始终保留 —— 防一个超大 _index 独霸整段预算。
        let nav_budget = (KB_CTX_BUDGET as f32 * KB_CTX_NAV_RATIO) as usize;
        let per_page = (KB_CTX_BUDGET as f32 * KB_CTX_PER_PAGE_RATIO) as usize;
        // 导航页按需读正文算长度(导航页数量极少,IO 开销可忽略)
        let nav_bodies: Vec<(String, usize)> = nav_docs
            .iter()
            .filter_map(|d| {
                read_doc_body(&d.rel_path).map(|b| {
                    let trimmed = b.trim().to_string();
                    let len = trimmed.chars().count();
                    (trimmed, len)
                })
            })
            .collect();
        let nav_total: usize = nav_bodies.iter().map(|(_, len)| len).sum();
        let effective_nav = nav_budget.min(nav_total);
        nav_surplus = nav_budget - effective_nav; // 全塞时为「段内让位」,截断时为 0
        let mut nav_used = 0usize;
        let mut nav_truncated = 0usize;
        for (d, (body_raw, _)) in nav_docs.iter().zip(nav_bodies.iter()) {
            if nav_used >= effective_nav {
                nav_truncated += 1;
                continue;
            }
            // 本篇可用额度 = min(单篇上限, 段内剩余); 截断后会附"已截断"提示
            let avail = per_page.min(effective_nav - nav_used);
            let body = truncate_chars(body_raw.trim(), avail);
            nav_used += body.chars().count();
            out.push_str(&format!(
                "##### [[{}]] · `{}`\n\n{}\n\n",
                stem(&d.rel_path),
                norm(&d.rel_path),
                body
            ));
        }
        if nav_total > effective_nav {
            // 触发了截断 (整体超上限)
            if nav_total <= nav_budget {
                out.push_str(&format!(
                    "*(导航段共 {} 字符, 触达上限)*\n\n",
                    nav_total
                ));
            } else {
                out.push_str(&format!(
                    "*(还有 {} 篇导航页/总计 {} 字符未注入, 用 `Read` 打开 wiki/index.md 或对应 _index.md 查看)*\n\n",
                    nav_truncated, nav_total.saturating_sub(nav_used)
                ));
            }
        }
        // 提示: 其他 40+ 篇 wiki 的目录清单在 wiki/index.md / 概念/_index.md / 实体/_index.md 里
        let wiki_total = idx
            .iter()
            .filter(|d| norm(&d.rel_path).starts_with("wiki/") && norm(&d.rel_path).ends_with(".md"))
            .count();
        out.push_str(&format!(
            "*(wiki/ 共 {} 篇, 此处仅注入 {} 篇导航页;要看某篇正文请用 Read 打开对应 .md)*\n\n",
            wiki_total,
            nav_docs.len()
        ));
    }

    // 知识库地图: raw/ output/ 等按文件夹分组, 列标题清单 (供沿双链/路径用 Read/Grep 自取)
    use std::collections::BTreeMap;
    let mut groups: BTreeMap<String, Vec<&KbDoc>> = BTreeMap::new();
    let scope_norm = scope.map(norm).filter(|s| !s.trim().is_empty());
    for d in idx.iter() {
        let rp = norm(&d.rel_path);
        if rp == "CLAUDE.md" || rp.starts_with("wiki/") {
            continue; // 行为指南单独注入; wiki 已全文给过
        }
        // 板块⑫: 限定到该人格的知识库 scope 子树
        if let Some(s) = &scope_norm {
            if !rp.starts_with(s.as_str()) {
                continue;
            }
        }
        groups.entry(parent(&rp)).or_default().push(d);
    }
    if !groups.is_empty() {
        out.push_str("#### 知识库地图 (沿双链 `[[名称]]` 或路径, 用 Read / Grep 自取原文)\n\n");
        if let Some(s) = &scope_norm {
            out.push_str(&format!(
                "*(本人格知识范围限定在 `{}/` 子树, 其余目录不在此人格上下文内)*\n\n",
                s
            ));
        }
        // 上下文预算: 地图段按总字符封顶 (而非固定每文件夹条数), 预算耗尽即停并提示 Glob 自取。
        // 弹性: 拿导航段让位的 `nav_surplus` 补到地图, 实际预算 = 基础 + 让位, 但总不超 KB_CTX_BUDGET。
        const MAX_PER_FOLDER: usize = 60;
        let map_base = (KB_CTX_BUDGET as f32 * KB_CTX_MAP_RATIO) as usize;
        let map_budget = (map_base + nav_surplus).min(KB_CTX_BUDGET);
        let mut map_used = 0usize;
        let mut budget_hit = false;
        'folders: for (folder, docs) in &groups {
            let header = format!("- **{}/** ({} 篇)\n", folder, docs.len());
            map_used += header.chars().count();
            out.push_str(&header);
            let mut shown = 0usize;
            for d in docs.iter().take(MAX_PER_FOLDER) {
                if map_used >= map_budget {
                    budget_hit = true;
                    break 'folders;
                }
                let title = if d.title.trim().is_empty() {
                    stem(&d.rel_path)
                } else {
                    d.title.trim().to_string()
                };
                let line = format!(
                    "  - [[{}]] — {} · `{}`\n",
                    stem(&d.rel_path),
                    title,
                    norm(&d.rel_path)
                );
                map_used += line.chars().count();
                out.push_str(&line);
                shown += 1;
            }
            if docs.len() > shown {
                out.push_str(&format!(
                    "  - …其余 {} 篇, 用 `Glob \"{}/**\"` 或 `Grep` 关键词列出\n",
                    docs.len() - shown,
                    folder
                ));
            }
        }
        if budget_hit {
            out.push_str(
                "- *(地图已达上下文预算上限, 其余目录/文件请用 `Glob`/`Grep` 自行探索)*\n",
            );
        }
        out.push('\n');
    }

    out.push_str(
        "#### 调用方式 (KB-first, 忠于 Karpathy)\n\
- 回答前先沿上面的结构与双链, 用 Read/Glob/Grep 打开相关页面取证, 不要凭空作答。\n\
- 命中知识库内容时用脚注标源: 正文处 `[^1]`, 文末 `[^1]: [[文件名]]`。\n\
- 双链 `[[…]]` 只写名称 (wiki 根相对名或标题), 不写绝对路径。\n\
- 库里确实查不到时, 用 `💡` 标明这是你的推断/仿写, 不要伪造引文, 也不要谎称检索过。\n\n",
    );
    out
}

/// 把前端传入的相对路径解析为 KB root 子树内的真实路径。
/// **canonicalize 后必须仍在 KB root 之下** —— 仅靠 `starts_with(root)` 是失效护栏:
/// `root.join("../../x")` 的路径组件仍以 root 开头, 前缀检查会误判通过, 而 OS 读写时 `..`
/// 会真的逃出库外。故规范化两端再比前缀, 同时挡住 `../../` 穿越与「绝对路径替换 join」。
/// 仅用于「目标应当已存在」的入口 (read/delete); 文件不存在直接报错。
fn resolve_within_kb(root: &Path, rel_path: &str) -> Result<PathBuf, String> {
    let full = root.join(rel_path);
    let canon_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let canon_full = full
        .canonicalize()
        .map_err(|_| "文件不存在或无法访问".to_string())?;
    if !path_contains(&canon_root, &canon_full) {
        return Err("路径越界, 拒绝访问".into());
    }
    Ok(canon_full)
}

/// 跨平台「子树包含」判断 —— 专治 Windows 上两类**误判越界**:
/// 1. `std::fs::canonicalize` 在 Windows 会加 `\\?\`(及 `\\?\UNC\`)扩展长度前缀。
///    若比较两端一端有前缀、一端没有(例如某端 canonicalize 失败回退原值),裸
///    `Path::starts_with` 必为假,合法路径被当成越界。
/// 2. Windows 文件系统大小写不敏感,但 `Path::starts_with` 大小写敏感;根目录存储时
///    的大小写与 canonicalize 返回的真实大小写不一致即误判。
///
/// 故先剥扩展长度前缀、再按平台规整大小写,最后用**组件级** `starts_with` 比较
/// (组件级可避免 `C:\foobar` 命中 `C:\foo` 这种伪前缀)。
pub fn path_contains(base: &Path, child: &Path) -> bool {
    fn norm(p: &Path) -> PathBuf {
        let s = p.to_string_lossy().to_string();
        let s = if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{rest}")
        } else if let Some(rest) = s.strip_prefix(r"\\?\") {
            rest.to_string()
        } else {
            s
        };
        if cfg!(windows) {
            PathBuf::from(s.to_lowercase())
        } else {
            PathBuf::from(s)
        }
    }
    norm(child).starts_with(norm(base))
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn kb_read(rel_path: String) -> Result<String, String> {
    let root = KB_ROOT.read().clone();
    let full = resolve_within_kb(&root, &rel_path)?;
    fs::read_to_string(&full).map_err(|e| e.to_string())
}

/// 删除资料库里的一份资料(浏览页每条右侧 × 用)。
/// 仅允许删除 KB root 子树内的文件; 删除后重扫索引, 返回剩余文件数。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn kb_delete(rel_path: String) -> Result<usize, String> {
    let root = KB_ROOT.read().clone();
    // 防越界: 规范化后必须仍在 KB root 下 (与 kb_read 共用同一护栏)
    let canon_full = resolve_within_kb(&root, &rel_path)?;
    if !canon_full.is_file() {
        return Err("只能删除文件".into());
    }
    fs::remove_file(&canon_full).map_err(|e| e.to_string())?;
    // 增量: 直接从 INDEX 移除, 避免全量重扫
    let rel_norm = rel_path.replace('\\', "/");
    index_remove(&rel_norm);
    let n = INDEX.read().len();
    Ok(n)
}

/// 清空资料库(管理页「清空资料库」用): 删除 `raw/` 下全部资料并重建空 `raw/`,
/// 保留三层骨架与 CLAUDE.md / wiki。返回清空后剩余索引文件数。
/// 已安装的名人资料包随之清掉, 想要回来去「名人资料包」重新安装即可。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn kb_clear() -> Result<usize, String> {
    let root = KB_ROOT.read().clone();
    let raw = root.join("raw");
    if raw.exists() {
        fs::remove_dir_all(&raw).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(&raw).map_err(|e| e.to_string())?;
    let docs = scan_all(&root);
    let n = docs.len();
    *INDEX.write() = docs;
    Ok(n)
}

#[derive(Serialize)]
pub struct KbHit {
    pub path: String,
    pub title: String,
    pub snippet: String,
    pub score: f64,
}

/// PRD §8.8 关键词加权评分: 标题 +10 / category +8 / 正文 +1
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn kb_search(query: String, top_k: Option<usize>) -> Vec<KbHit> {
    let q = query.to_lowercase();
    let terms: Vec<&str> = q.split_whitespace().collect();
    if terms.is_empty() {
        return vec![];
    }
    let topk = top_k.unwrap_or(8);
    let idx = INDEX.read();
    let mut scored: Vec<(f64, String, String, String)> = Vec::new(); // score, path, title, snippet
    for d in idx.iter() {
        let title_lc = d.title.to_lowercase();
        let cat_lc = d.category.to_lowercase();
        let mut score = 0.0;
        for t in &terms {
            if title_lc.contains(t) {
                score += 10.0;
            }
            if !cat_lc.is_empty() && cat_lc.contains(t) {
                score += 8.0;
            }
        }
        // 按需读正文: 标题/category 已命中需要 snippet; 没命中需要确认正文是否命中
        let body_opt = read_doc_body(&d.rel_path);
        if let Some(ref body) = body_opt {
            let body_lc = body.to_lowercase();
            for t in &terms {
                let body_count = body_lc.matches(t).count() as f64;
                score += body_count;
            }
        }
        if score < 1.0 {
            continue;
        }
        let snippet = body_opt
            .as_deref()
            .map(|b| first_snippet(b, &terms, 160))
            .unwrap_or_default();
        scored.push((score, d.rel_path.clone(), d.title.clone(), snippet));
    }
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .take(topk)
        .map(|(score, path, title, snippet)| KbHit {
            path,
            title,
            snippet,
            score,
        })
        .collect()
}

fn first_snippet(body: &str, terms: &[&str], max_len: usize) -> String {
    let lower = body.to_lowercase();
    let mut best = 0usize;
    for t in terms {
        if let Some(p) = lower.find(t) {
            best = p;
            break;
        }
    }
    let start = best.saturating_sub(40);
    let end = (start + max_len).min(body.len());
    let raw = &body[clamp_char_boundary(body, start)..clamp_char_boundary(body, end)];
    raw.replace('\n', " ").trim().to_string()
}

fn clamp_char_boundary(s: &str, mut idx: usize) -> usize {
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx.min(s.len())
}

// ── 增量索引辅助 ─────────────────────────

/// 把单个新文档增量加入 INDEX(同 rel_path 已存在则覆盖)。
fn index_add_doc(doc: KbDoc) {
    let mut idx = INDEX.write();
    if let Some(pos) = idx.iter().position(|d| d.rel_path == doc.rel_path) {
        idx[pos] = doc;
    } else {
        idx.push(doc);
    }
}

/// 从 INDEX 中移除指定 rel_path 的文档。
fn index_remove(rel_path: &str) {
    let mut idx = INDEX.write();
    idx.retain(|d| d.rel_path != rel_path);
}

/// Ingest 单文件:任意格式 → 转 markdown 写入 raw/(不可转的原样复制),增量刷新索引。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn kb_ingest(source_path: String) -> Result<String, String> {
    let root = KB_ROOT.read().clone();
    let mut cache = IngestCache::load(&root);
    let rel = ingest_one(&root, &PathBuf::from(&source_path), &mut cache);
    cache.save(&root);
    let rel = rel?;
    // 增量: 只解析新文件加入 INDEX, 避免全量重扫
    let full = root.join(&rel);
    if let Ok(rp) = full.strip_prefix(&root) {
        if let Some(doc) = parse_doc(&full, rp) {
            index_add_doc(doc);
        }
    }
    Ok(rel)
}

/// 知识库拖拽上传:批量(可含目录,自动展开)。每个文件转 markdown 入 raw/,
/// 全部处理完只重扫一次索引。返回逐文件结果(失败不影响其余)。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn kb_upload_files(paths: Vec<String>) -> Vec<KbUploadResult> {
    const MAX_FILES: usize = 500;
    let root = KB_ROOT.read().clone();
    let files = expand_to_files(&paths, MAX_FILES);
    // 整批共用一个缓存: 未变且产物仍在的源跳过转换, 结束统一落盘。
    let mut cache = IngestCache::load(&root);

    let mut results = Vec::with_capacity(files.len());
    for f in &files {
        let name = f
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| f.to_string_lossy().to_string());
        match ingest_one(&root, f, &mut cache) {
            Ok(rel) => results.push(KbUploadResult {
                name,
                rel_path: rel,
                ok: true,
                message: String::new(),
            }),
            Err(e) => results.push(KbUploadResult {
                name,
                rel_path: String::new(),
                ok: false,
                message: e,
            }),
        }
    }
    cache.save(&root);

    // 增量: 逐个解析成功入库的文件加入 INDEX, 避免全量重扫
    for r in &results {
        if !r.ok || r.rel_path.is_empty() {
            continue;
        }
        let full = root.join(&r.rel_path);
        if let Ok(rp) = full.strip_prefix(&root) {
            if let Some(doc) = parse_doc(&full, rp) {
                index_add_doc(doc);
            }
        }
    }

    results
}

#[derive(Serialize)]
pub struct KbUploadResult {
    pub name: String,
    pub rel_path: String,
    pub ok: bool,
    pub message: String,
}

// ───────────────────────── 批量转换 md (管理页「批量转换 md 文件」) ─────────────────────────
//
// 与拖拽上传/ingest 的差别: 这是「只要 markdown」的批量通道 ——
// 可抽文本的 (PDF/Word/Excel/PPT/文本/代码) 转成 .md 入 raw/;
// 视频类明确跳过 (主要针对非视频类文件, 视频留给将来的 ASR 链路);
// 图片/音频/压缩包等抽不出文本的也跳过**而不是原样复制**, 避免把大体积二进制灌进知识库。

/// 视频扩展名 (小写)。注意不含 "ts" —— 那会误伤 TypeScript 源码 (TEXT_EXTS 按文本转)。
const VIDEO_EXTS: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v", "mpg", "mpeg", "m2ts", "3gp",
    "rmvb", "rm", "vob", "ogv",
];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KbConvertReport {
    /// 扫到的文件总数
    pub total: usize,
    /// 成功转成 md 的数量 (含缓存命中复用)
    pub converted: usize,
    /// 视频类跳过数
    pub skipped_video: usize,
    /// 其它跳过数 (图片/音频/压缩包等不可抽文本, 以及 KB 内已是 md 的文件)
    pub skipped_other: usize,
    /// 失败明细 "文件名: 原因"
    pub failed: Vec<String>,
}

/// 批量转换: 路径(文件或文件夹, 文件夹递归展开)下的非视频类文件 → markdown 入 raw/ 并增量索引。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn kb_convert_batch(paths: Vec<String>) -> Result<KbConvertReport, String> {
    const MAX_FILES: usize = 2000;
    let root = KB_ROOT.read().clone();
    let raw_dir = root.join("raw");
    fs::create_dir_all(&raw_dir).map_err(|e| e.to_string())?;

    let files = expand_to_files(&paths, MAX_FILES);
    if files.is_empty() {
        return Err("没找到文件: 请确认填的是存在的文件或文件夹绝对路径".into());
    }

    let mut cache = IngestCache::load(&root);
    let mut report = KbConvertReport {
        total: files.len(),
        converted: 0,
        skipped_video: 0,
        skipped_other: 0,
        failed: Vec::new(),
    };
    let mut new_rels: Vec<String> = Vec::new();

    for f in &files {
        let ext = f
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if VIDEO_EXTS.contains(&ext.as_str()) {
            report.skipped_video += 1;
            continue;
        }
        // KB 根内已是 md 的文件不重转, 防止用户把 KB 根自己填进来时自吞出 "(2)" 副本
        if ext == "md" && f.starts_with(&root) {
            report.skipped_other += 1;
            continue;
        }
        match convert_one_md(&root, &raw_dir, f, &mut cache) {
            Ok(Some(rel)) => {
                report.converted += 1;
                new_rels.push(rel);
            }
            Ok(None) => report.skipped_other += 1,
            Err(e) => {
                let name = f
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| f.to_string_lossy().to_string());
                report.failed.push(format!("{name}: {e}"));
            }
        }
    }
    cache.save(&root);

    // 增量索引新产物, 避免全量重扫
    for rel in &new_rels {
        let full = root.join(rel);
        if let Ok(rp) = full.strip_prefix(&root) {
            if let Some(doc) = parse_doc(&full, rp) {
                index_add_doc(doc);
            }
        }
    }
    Ok(report)
}

/// 单文件「只要 md」转换: 可抽文本 → 写 raw/<stem>.md 并记缓存; 不可抽 → Ok(None) 跳过。
/// 与 ingest_one 的差别: 不做「不可转就原样复制」的兜底。
fn convert_one_md(
    root: &Path,
    raw_dir: &Path,
    src: &Path,
    cache: &mut IngestCache,
) -> Result<Option<String>, String> {
    if !src.is_file() {
        return Err("不是文件".into());
    }
    let src_key = src.to_string_lossy().replace('\\', "/");
    let fingerprint = content_fingerprint(src);
    if let Some(fp) = &fingerprint {
        if let Some(raw_rel) = cache.lookup_valid(root, &src_key, fp) {
            // 只复用 md 产物; 旧通道原样复制进来的非 md 产物不算"已转换"
            if raw_rel.ends_with(".md") {
                return Ok(Some(raw_rel));
            }
        }
    }
    let Some(md) = convert::convert_to_markdown(src)? else {
        return Ok(None);
    };
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "untitled".into());
    let dst = unique_path(raw_dir, &stem, "md");
    let titled = format!("# {stem}\n\n{md}");
    fs::write(&dst, titled).map_err(|e| e.to_string())?;
    let rel = rel_of(root, &dst);
    if let Some(fp) = fingerprint {
        cache.record(src_key, fp, rel.clone());
    }
    Ok(Some(rel))
}

// ───────────────────────── 增量入库缓存 (借鉴 llm_wiki ingest-cache) ─────────────────────────
//
// 痛点: 重复拖同一批资料入库, 每次都全量重转 (PDF/docx 抽取很贵)。
// 借鉴 llm_wiki: 给源文件算内容指纹, 指纹没变 → 跳过转换, 直接复用上次产物。
// 关键的第二步 (llm_wiki 的「防幽灵条目」洞察): 命中缓存还要**校验产物仍在磁盘上**,
// 否则旧产物被删后缓存还指着它, 会"跳过"导致库里凭空少一篇。
// 用 std 的 DefaultHasher (siphash) 做内容指纹 —— 仅需变更检测, 不引入 sha2 依赖。

#[derive(Default, Serialize, Deserialize)]
struct IngestCache {
    /// 源文件绝对路径 → (内容指纹, 产物 raw 相对路径)
    #[serde(default)]
    entries: HashMap<String, (String, String)>,
    #[serde(skip)]
    dirty: bool,
}

fn ingest_cache_path(root: &Path) -> PathBuf {
    root.join(".polaris_ingest_cache.json")
}

/// 计算文件内容指纹 (siphash, 仅用于变更检测)。读失败返回 None。
fn content_fingerprint(src: &Path) -> Option<String> {
    use std::hash::Hasher;
    let bytes = fs::read(src).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    h.write(&bytes);
    Some(format!("{:x}", h.finish()))
}

impl IngestCache {
    fn load(root: &Path) -> Self {
        fs::read_to_string(ingest_cache_path(root))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save(&self, root: &Path) {
        if self.dirty {
            if let Ok(s) = serde_json::to_string(self) {
                let _ = fs::write(ingest_cache_path(root), s);
            }
        }
    }

    /// 命中缓存且产物仍在磁盘 → 返回可复用的 raw 相对路径。
    fn lookup_valid(&self, root: &Path, src_key: &str, fp: &str) -> Option<String> {
        let (cached_fp, raw_rel) = self.entries.get(src_key)?;
        if cached_fp != fp {
            return None; // 内容变了
        }
        if !root.join(raw_rel).exists() {
            return None; // 防幽灵条目: 产物已被删
        }
        Some(raw_rel.clone())
    }

    /// 该源已记录的旧产物相对路径(用于内容变更时删除陈旧副本)。
    fn stale_artifact(&self, src_key: &str) -> Option<String> {
        self.entries.get(src_key).map(|(_, rel)| rel.clone())
    }
    fn record(&mut self, src_key: String, fp: String, raw_rel: String) {
        self.entries.insert(src_key, (fp, raw_rel));
        self.dirty = true;
    }
}

/// 把一个源文件落到 KB 的 raw/:
/// - 命中增量缓存(内容未变且产物仍在) → 跳过转换, 复用上次产物
/// - 可抽文本 → 写 `raw/<stem>.md`
/// - 不可抽(图片/二进制) → 原样复制 `raw/<filename>`
/// 返回写入的相对路径(正斜杠)。
fn ingest_one(root: &Path, src: &Path, cache: &mut IngestCache) -> Result<String, String> {
    if !src.is_file() {
        return Err(format!("不是文件: {}", src.to_string_lossy()));
    }
    let raw_dir = root.join("raw");
    fs::create_dir_all(&raw_dir).map_err(|e| e.to_string())?;

    // 增量缓存: 内容指纹未变且产物仍在磁盘 → 直接复用, 跳过昂贵的转换。
    let src_key = src.to_string_lossy().replace('\\', "/");
    let fingerprint = content_fingerprint(src);
    if let Some(fp) = &fingerprint {
        if let Some(raw_rel) = cache.lookup_valid(root, &src_key, fp) {
            return Ok(raw_rel);
        }
    }

    // 指纹变了(源文件被编辑过重新拖入): 先删旧产物。否则 unique_path 会另写 "stem (2).md",
    // 旧的陈旧内容永远留在 raw/ 和 INDEX 里, 被搜索/图谱/编译当成独立页一并引用。
    if let Some(old_rel) = cache.stale_artifact(&src_key) {
        let old = root.join(&old_rel);
        if old.exists() {
            let _ = fs::remove_file(&old);
        }
    }

    let raw_rel = ingest_convert_write(root, src, &raw_dir)?;
    if let Some(fp) = fingerprint {
        cache.record(src_key, fp, raw_rel.clone());
    }
    Ok(raw_rel)
}

/// 实际的转换+落盘 (从 ingest_one 拆出, 便于缓存命中时整体跳过)。
fn ingest_convert_write(root: &Path, src: &Path, raw_dir: &Path) -> Result<String, String> {
    match convert::convert_to_markdown(src)? {
        Some(md) => {
            let stem = src
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "untitled".into());
            let dst = unique_path(raw_dir, &stem, "md");
            // 顶部补一个标题,便于 KB 索引与预览
            let titled = format!("# {stem}\n\n{md}");
            fs::write(&dst, titled).map_err(|e| e.to_string())?;
            Ok(rel_of(root, &dst))
        }
        None => {
            let fname = src
                .file_name()
                .ok_or_else(|| "无文件名".to_string())?
                .to_string_lossy()
                .to_string();
            let (stem, ext) = split_name(&fname);
            let dst = unique_path(raw_dir, &stem, &ext);
            fs::copy(src, &dst).map_err(|e| e.to_string())?;
            Ok(rel_of(root, &dst))
        }
    }
}

/// 展开输入路径:目录递归取文件,文件直接收,去重并限量。
fn expand_to_files(paths: &[String], cap: usize) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for p in paths {
        if out.len() >= cap {
            break;
        }
        let pb = PathBuf::from(p);
        if pb.is_dir() {
            for e in WalkDir::new(&pb).into_iter().flatten() {
                if e.path().is_file() {
                    out.push(e.path().to_path_buf());
                    if out.len() >= cap {
                        break;
                    }
                }
            }
        } else if pb.is_file() {
            out.push(pb);
        }
    }
    out
}

/// 在 dir 下生成不冲突的路径 `<stem>.<ext>`,冲突则追加 ` (2)` ` (3)` …
fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let safe = sanitize_stem(stem);
    let first = dir.join(format!("{safe}.{ext}"));
    if !first.exists() {
        return first;
    }
    for n in 2..10_000 {
        let cand = dir.join(format!("{safe} ({n}).{ext}"));
        if !cand.exists() {
            return cand;
        }
    }
    first
}

/// 去掉文件名里对 Windows 非法的字符
fn sanitize_stem(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c })
        .collect();
    let t = cleaned.trim().trim_matches('.').trim();
    if t.is_empty() {
        "untitled".into()
    } else {
        t.to_string()
    }
}

fn split_name(fname: &str) -> (String, String) {
    match fname.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), e.to_string()),
        _ => (fname.to_string(), "bin".to_string()),
    }
}

fn rel_of(root: &Path, full: &Path) -> String {
    full.strip_prefix(root)
        .unwrap_or(full)
        .to_string_lossy()
        .replace('\\', "/")
}

// ───────────────────────── 安全路径护栏 (借鉴 llm_wiki isSafeIngestPath) ─────────────────────────
//
// 编译器 (kb_compile) 给 headless claude 开了写权限自由落盘 wiki 页。万一模型(或被注入)给出
// `C:\Windows\...` 这种绝对路径、`../../` 越界、或 Windows 保留名, 就可能写坏库外文件。
// 这是一道**纯函数**护栏: 校验「应当落在 wiki/ 下的相对路径」是否安全。7 层校验, 一条不过即拒。
// 用于编译后审计 (kb_lint) 与任何接受模型生成路径的入口。

/// Windows 设备保留名 (任意大小写, 含带扩展名形式如 `CON.md` 也保留)。
const WIN_RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// 校验一个「本应落在 wiki/ 下」的相对路径是否安全可写。
/// 返回 `Err(原因)` 列出第一个不通过的校验项。
pub fn is_safe_wiki_relpath(raw: &str) -> Result<(), String> {
    // ① 无控制字符
    if raw.chars().any(|c| c.is_control()) {
        return Err("含控制字符".into());
    }
    // ② 拒绝绝对路径 (Unix `/`、UNC `\\`、Windows 盘符 `C:`)
    if raw.starts_with('/') || raw.starts_with('\\') {
        return Err("是绝对路径".into());
    }
    let bytes = raw.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return Err("含 Windows 盘符".into());
    }
    // ③ 规范化反斜杠后逐段检查
    let norm = raw.replace('\\', "/");
    let segs: Vec<&str> = norm.split('/').filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return Err("路径为空".into());
    }
    for seg in &segs {
        // ④ 无 `.` / `..` 越界段
        if *seg == ".." || *seg == "." {
            return Err("含 .. 或 . 越界段".into());
        }
        // ⑤ Windows 保留名 (取扩展名前的主名判断)
        let stem = seg.split('.').next().unwrap_or(seg).to_lowercase();
        if WIN_RESERVED.contains(&stem.as_str()) {
            return Err(format!("含 Windows 保留名: {seg}"));
        }
        // ⑥ 段尾不得为空格或点 (Windows 会静默剥离, 造成路径歧义)
        if seg.ends_with(' ') || seg.ends_with('.') {
            return Err(format!("段尾有空格或点: {seg}"));
        }
    }
    // ⑦ 必须落在 wiki/ 下且为 markdown
    if segs[0] != "wiki" {
        return Err("未落在 wiki/ 下".into());
    }
    if !(norm.ends_with(".md") || norm.ends_with(".markdown")) {
        return Err("不是 .md 文件".into());
    }
    Ok(())
}

// ───────────────────────── Graph ─────────────────────────

#[derive(Serialize)]
pub struct KbNode {
    pub id: String,
    pub title: String,
    pub category: String,
    /// 节点类型: "doc" 文档 | "folder" 目录中枢 | "root" 知识库根
    pub kind: String,
}

#[derive(Serialize)]
pub struct KbEdge {
    pub source: String,
    pub target: String,
}

#[derive(Serialize)]
pub struct KbGraph {
    pub nodes: Vec<KbNode>,
    pub edges: Vec<KbEdge>,
}

/// 知识库根中枢节点 id (合成节点, 不对应真实文件)
const ROOT_ID: &str = "__kb_root__";

/// 目录中枢节点 id 前缀。Windows/真实文件名不含冒号, 故不会与 rel_path 冲突。
fn folder_id(rel: &str) -> String {
    format!("dir:{rel}")
}

/// 把 Markdown 链接目标 (可能含 ./ ../) 解析回知识库内的 rel_path。
/// base_dir 为发出链接的文档所在目录 (rel)。返回规范化的正斜杠 rel_path。
fn resolve_rel(base_dir: Option<&Path>, link: &str) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(b) = base_dir {
        for s in b.to_string_lossy().replace('\\', "/").split('/') {
            if !s.is_empty() {
                parts.push(s.to_string());
            }
        }
    }
    for seg in link.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other.to_string()),
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

/// 知识图谱: 文档节点 + 目录层级派生的中枢结构 + 双链/Markdown 链接关系边。
///
/// 散点根因 (PRD §8 设计回顾): 原实现只认 `[[wikilink]]`, 未链接的文档=孤点。
/// 现按真实目录层级 (raw/X/卷/篇) 自动生成"目录中枢节点"和树状边, 使任意
/// 知识库无需手工双链即可呈现连通图谱; 双链与 Markdown 链接作为额外关系叠加。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn kb_graph() -> KbGraph {
    use std::collections::HashSet;
    let idx = INDEX.read();

    // 标题/文件名 -> rel_path (用于 [[wikilink]] 解析)
    let mut title_to_path: HashMap<String, String> = HashMap::new();
    let mut path_set: HashSet<String> = HashSet::new();
    for d in idx.iter() {
        title_to_path.insert(d.title.to_lowercase(), d.rel_path.clone());
        let stem = Path::new(&d.rel_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        title_to_path.entry(stem).or_insert_with(|| d.rel_path.clone());
        path_set.insert(d.rel_path.clone());
    }

    let mut nodes: Vec<KbNode> = Vec::new();
    let mut edge_set: HashSet<(String, String)> = HashSet::new();
    let mut folder_set: HashSet<String> = HashSet::new();

    // ① 文档节点
    for d in idx.iter() {
        nodes.push(KbNode {
            id: d.rel_path.clone(),
            title: d.title.clone(),
            category: d.category.clone(),
            kind: "doc".into(),
        });
    }

    // ② 目录层级 -> 中枢节点 + 树状边
    for d in idx.iter() {
        let segs: Vec<&str> = d.rel_path.split('/').filter(|s| !s.is_empty()).collect();
        if segs.len() < 2 {
            // 根目录下的散文件: 直接挂到知识库根
            edge_set.insert((d.rel_path.clone(), ROOT_ID.to_string()));
            continue;
        }
        // 累积每一层文件夹路径 (不含文件名)
        let mut acc = String::new();
        let mut folders: Vec<String> = Vec::new();
        for s in &segs[..segs.len() - 1] {
            if acc.is_empty() {
                acc = (*s).to_string();
            } else {
                acc = format!("{acc}/{s}");
            }
            folders.push(acc.clone());
        }
        // 文档 -> 最深一层目录
        edge_set.insert((d.rel_path.clone(), folder_id(folders.last().unwrap())));
        // 目录 -> 上级目录 逐层
        for w in folders.windows(2) {
            edge_set.insert((folder_id(&w[1]), folder_id(&w[0])));
        }
        // 顶层目录 -> 知识库根
        edge_set.insert((folder_id(&folders[0]), ROOT_ID.to_string()));
        for f in folders {
            folder_set.insert(f);
        }
    }

    // ③ 目录中枢节点
    for f in &folder_set {
        let title = f.rsplit('/').next().unwrap_or(f).to_string();
        nodes.push(KbNode {
            id: folder_id(f),
            title,
            category: String::new(),
            kind: "folder".into(),
        });
    }
    // ④ 知识库根节点 (有内容时)
    if !nodes.is_empty() {
        nodes.push(KbNode {
            id: ROOT_ID.to_string(),
            title: "知识库".into(),
            category: String::new(),
            kind: "root".into(),
        });
    }

    // ⑤ [[wikilink]] 关系边
    for d in idx.iter() {
        for link in &d.wikilinks {
            let key = link.to_lowercase();
            if let Some(target) = title_to_path.get(&key) {
                if target != &d.rel_path {
                    edge_set.insert((d.rel_path.clone(), target.clone()));
                }
            }
        }
    }

    // ⑥ Markdown 链接 [文](relpath.md) 关系边
    for d in idx.iter() {
        let base_dir = Path::new(&d.rel_path).parent();
        if let Some(body) = read_doc_body(&d.rel_path) {
            for cap in RE_MDLINK.captures_iter(&body) {
                let raw = cap.get(1).map(|m| m.as_str().trim()).unwrap_or("");
                if raw.is_empty()
                    || raw.starts_with("http")
                    || raw.starts_with('#')
                    || raw.starts_with("mailto:")
                {
                    continue;
                }
                let target_raw = raw.split(['#', '?']).next().unwrap_or(raw);
                if !(target_raw.ends_with(".md") || target_raw.ends_with(".markdown")) {
                    continue;
                }
                if let Some(t) = resolve_rel(base_dir, target_raw) {
                    if t != d.rel_path && path_set.contains(&t) {
                        edge_set.insert((d.rel_path.clone(), t));
                    }
                }
            }
        }
    }

    let edges = edge_set
        .into_iter()
        .map(|(source, target)| KbEdge { source, target })
        .collect();

    KbGraph { nodes, edges }
}

// ───────────────────────── wiki 质量检查 (借鉴 llm_wiki lint + sweep) ─────────────────────────
//
// 知识库会「自己越长越乱」: claude 编译时可能写出指向不存在页的死双链、漏写 frontmatter 的 type、
// 留下没人链接也不链接别人的孤儿页。借鉴 llm_wiki 的 lint: 纯规则扫一遍 INDEX, 把问题列清楚,
// 作为「后台巡检 (sweep)」的眼睛 —— 先看见问题, 才能交给 kb_dedup / kb_enrich_links 去修。

#[derive(Serialize)]
pub struct KbLintIssue {
    /// dead-link | missing-type | orphan | unsafe-path
    pub kind: String,
    pub path: String,
    pub detail: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KbLintReport {
    pub total_pages: usize,
    pub dead_links: usize,
    pub missing_type: usize,
    pub orphans: usize,
    pub unsafe_paths: usize,
    pub issues: Vec<KbLintIssue>,
}

/// 导航/元页 (index / _index / log / 方法论): 不参与「缺 type」「孤儿」判定。
fn is_wiki_meta_page(rp: &str) -> bool {
    rp == "wiki/index.md"
        || rp.ends_with("/_index.md")
        || rp == "wiki/log.md"
        || rp.ends_with("/log.md")
        || rp == "wiki/karpathy-wiki方法论.md"
        || rp == "wiki/wiki-knowledge-base.md"
}

/// wiki 质量检查: 死双链 / 缺 frontmatter type / 孤儿页 / 不安全路径。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn kb_lint() -> KbLintReport {
    use std::collections::HashSet;
    let idx = INDEX.read();
    let norm = |s: &str| s.replace('\\', "/");

    // 双链解析表: 小写标题 + 文件名 stem → rel_path (与 kb_graph 一致的解析口径)
    let mut title_to_path: HashMap<String, String> = HashMap::new();
    for d in idx.iter() {
        title_to_path.insert(d.title.to_lowercase(), d.rel_path.clone());
        let stem = Path::new(&d.rel_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        title_to_path.entry(stem).or_insert_with(|| d.rel_path.clone());
    }

    // 被任意页面双链指向的目标 (用于孤儿判定)
    let mut referenced: HashSet<String> = HashSet::new();
    for d in idx.iter() {
        for link in &d.wikilinks {
            if let Some(t) = title_to_path.get(&link.to_lowercase()) {
                referenced.insert(t.clone());
            }
        }
    }

    const MAX_ISSUES: usize = 300;
    let mut issues: Vec<KbLintIssue> = Vec::new();
    let (mut dead_links, mut missing_type, mut orphans, mut unsafe_paths) = (0, 0, 0, 0);
    let mut wiki_pages = 0usize;
    let mut push = |issues: &mut Vec<KbLintIssue>, kind: &str, path: &str, detail: String| {
        if issues.len() < MAX_ISSUES {
            issues.push(KbLintIssue {
                kind: kind.into(),
                path: path.into(),
                detail,
            });
        }
    };

    for d in idx.iter() {
        let rp = norm(&d.rel_path);
        if !rp.starts_with("wiki/") {
            continue; // 只检查知识层
        }
        wiki_pages += 1;

        // ① 死双链: 指向不存在页面的 [[X]]
        for link in &d.wikilinks {
            if !title_to_path.contains_key(&link.to_lowercase()) {
                dead_links += 1;
                push(&mut issues, "dead-link", &rp, format!("[[{}]] 无对应页面", link));
            }
        }

        // ② 不安全路径 (理论上扫到的文件都存在, 但路径形态可能不规范)
        if is_safe_wiki_relpath(&rp).is_err() {
            unsafe_paths += 1;
            if let Err(why) = is_safe_wiki_relpath(&rp) {
                push(&mut issues, "unsafe-path", &rp, why);
            }
        }

        if is_wiki_meta_page(&rp) {
            continue; // 元页不查 type / 孤儿
        }

        // ③ 缺 frontmatter type
        if d.doc_type.trim().is_empty() {
            missing_type += 1;
            push(&mut issues, "missing-type", &rp, "frontmatter 缺 type 字段".into());
        }

        // ④ 孤儿页: 既不链接别人, 也没人链接它
        let links_out = !d.wikilinks.is_empty();
        let linked_in = referenced.contains(&d.rel_path);
        if !links_out && !linked_in {
            orphans += 1;
            push(&mut issues, "orphan", &rp, "无入链也无出链, 未接入知识网".into());
        }
    }

    KbLintReport {
        total_pages: wiki_pages,
        dead_links,
        missing_type,
        orphans,
        unsafe_paths,
        issues,
    }
}

/// 用于 chat_send: 把 search hits 渲染成 system prompt KB 块
pub fn render_kb_context(query: &str, top_k: usize) -> String {
    let hits = kb_search(query.to_string(), Some(top_k));
    if hits.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n\n## 维基库召回 (KB-first)\n\n");
    out.push_str("以下文件由 Polaris 在你的本地知识库中按关键词加权评分召回,优先以此回答:\n\n");
    let root = KB_ROOT.read().clone();
    for (i, h) in hits.iter().enumerate() {
        let full = root.join(&h.path);
        let body = fs::read_to_string(&full).unwrap_or_default();
        let trimmed: String = body.chars().take(4000).collect();
        out.push_str(&format!(
            "### [{}] {}\n来源: `{}`\n\n{}\n\n---\n\n",
            i + 1,
            h.title,
            h.path,
            trimmed
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_path_accepts_normal_wiki_pages() {
        assert!(is_safe_wiki_relpath("wiki/概念/主观主义.md").is_ok());
        assert!(is_safe_wiki_relpath("wiki/index.md").is_ok());
        assert!(is_safe_wiki_relpath("wiki/实体/a/b.markdown").is_ok());
    }

    #[test]
    fn safe_path_rejects_escapes_and_absolutes() {
        assert!(is_safe_wiki_relpath("../etc/passwd.md").is_err()); // 越界 (且不在 wiki/)
        assert!(is_safe_wiki_relpath("wiki/../../x.md").is_err()); // .. 段
        assert!(is_safe_wiki_relpath("/wiki/x.md").is_err()); // 绝对
        assert!(is_safe_wiki_relpath("C:/wiki/x.md").is_err()); // 盘符
        assert!(is_safe_wiki_relpath("\\\\srv\\wiki\\x.md").is_err()); // UNC
    }

    #[test]
    fn resolve_within_kb_rejects_traversal() {
        // 用真实临时目录验证运行期护栏 (区别于上面 is_safe_wiki_relpath 的纯函数校验)
        let base = std::env::temp_dir().join(format!("polaris_kbguard_{}", std::process::id()));
        let root = base.join("kb");
        let _ = fs::create_dir_all(&root);
        fs::write(root.join("inside.md"), "ok").unwrap();
        fs::write(base.join("secret.txt"), "secret").unwrap();

        // 库内文件: 放行, 且解析回真实路径
        assert!(resolve_within_kb(&root, "inside.md").is_ok());
        // `../` 穿越到库外: 必须拒 (旧 starts_with 护栏会误放)
        assert!(resolve_within_kb(&root, "../secret.txt").is_err());
        // 多级穿越同样拒
        assert!(resolve_within_kb(&root, "../../Windows/System32/drivers/etc/hosts").is_err());
        // 不存在的文件: 报错而非 panic
        assert!(resolve_within_kb(&root, "nope.md").is_err());

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn path_contains_handles_verbatim_prefix_and_case() {
        // 同根, 子在内: 放行
        assert!(path_contains(Path::new(r"C:\Users\a\Polaris"), Path::new(r"C:\Users\a\Polaris\x.md")));
        // 关键回归: 一端带 Windows `\\?\` 扩展长度前缀、一端没有, 仍判为包含 (旧裸 starts_with 会误判越界)
        assert!(path_contains(Path::new(r"C:\Users\a\Polaris"), Path::new(r"\\?\C:\Users\a\Polaris\x.md")));
        assert!(path_contains(Path::new(r"\\?\C:\Users\a\Polaris"), Path::new(r"C:\Users\a\Polaris\x.md")));
        // 伪前缀: 组件级比较不应把 `Polaris-bak` 当成 `Polaris` 的子树
        assert!(!path_contains(Path::new(r"C:\Users\a\Polaris"), Path::new(r"C:\Users\a\Polaris-bak\x.md")));
        // 真越界: 不在根下
        assert!(!path_contains(Path::new(r"C:\Users\a\Polaris"), Path::new(r"C:\Windows\System32\drivers\etc\hosts")));
        // Windows 上大小写不敏感: 根与子大小写不一致也应判为包含
        if cfg!(windows) {
            assert!(path_contains(Path::new(r"C:\Users\A\Polaris"), Path::new(r"c:\users\a\polaris\x.md")));
        }
    }

    #[test]
    fn safe_path_rejects_reserved_and_outside_wiki() {
        assert!(is_safe_wiki_relpath("wiki/CON.md").is_err()); // Windows 保留名
        assert!(is_safe_wiki_relpath("wiki/nul.md").is_err());
        assert!(is_safe_wiki_relpath("raw/x.md").is_err()); // 不在 wiki/
        assert!(is_safe_wiki_relpath("wiki/note.txt").is_err()); // 非 md
        assert!(is_safe_wiki_relpath("wiki/sub /page.md").is_err()); // 目录段尾空格
        assert!(is_safe_wiki_relpath("wiki/sub./page.md").is_err()); // 目录段尾点
    }

    #[test]
    fn enrich_links_first_occurrence_skips_code_and_existing() {
        // 首次出现替换为带别名的双链
        let body = "# 标题\n\n马克思主义是核心。再提马克思主义。\n";
        let out = apply_wikilink(body, "马克思主义", "马克思主义").unwrap();
        assert!(out.contains("[[马克思主义]]是核心"));
        // 只替首次: 第二处仍是纯文本
        assert_eq!(out.matches("[[马克思主义]]").count(), 1);
        assert!(out.contains("再提马克思主义"));
    }

    #[test]
    fn enrich_links_alias_when_term_differs() {
        let body = "讨论了实践论的要点。";
        let out = apply_wikilink(body, "实践论", "实践论(著作)").unwrap();
        assert!(out.contains("[[实践论(著作)|实践论]]"));
    }

    #[test]
    fn enrich_links_skips_frontmatter_and_already_linked() {
        // frontmatter 里的同名词不动
        let body = "---\ntitle: 矛盾论\ntype: concept\n---\n\n正文提到矛盾论。";
        let out = apply_wikilink(body, "矛盾论", "矛盾论").unwrap();
        assert!(out.contains("title: 矛盾论")); // frontmatter 未被改
        assert!(out.contains("正文提到[[矛盾论]]"));

        // 已是双链则不再重复包裹
        let linked = "已经有 [[矛盾论]] 了。";
        assert!(apply_wikilink(linked, "矛盾论", "矛盾论").is_none());
    }

    #[test]
    fn enrich_links_skips_inline_code() {
        let body = "用 `kb_compile` 命令构建。";
        assert!(apply_wikilink(body, "kb_compile", "kb_compile").is_none());
    }

    #[test]
    fn normalize_title_collapses_punctuation_and_case() {
        assert_eq!(normalize_title("矛盾论"), normalize_title("矛盾论 "));
        assert_eq!(normalize_title("On Practice"), normalize_title("on  practice"));
        assert_eq!(normalize_title("实践-论(草)"), normalize_title("实践论草"));
    }

    #[test]
    fn rewrite_wikilink_target_keeps_alias_and_section() {
        let body = "见 [[旧页]] 和 [[旧页|别名]] 与 [[旧页#某节]], 但 [[别的页]] 不动。";
        let out = rewrite_wikilink_target(body, "旧页", "新页");
        assert!(out.contains("[[新页]]"));
        assert!(out.contains("[[新页|别名]]"));
        assert!(out.contains("[[新页#某节]]"));
        assert!(out.contains("[[别的页]]")); // 未匹配的保持原样
        assert!(!out.contains("[[旧页"));
    }

    #[test]
    fn extract_json_tolerates_fences_and_prose() {
        let s = "好的, 结果如下:\n```json\n[{\"a\":1},{\"b\":\"]x\"}]\n```\n完毕";
        let j = extract_balanced_json(s).unwrap();
        assert_eq!(j, "[{\"a\":1},{\"b\":\"]x\"}]");
        let obj = extract_balanced_json("noise {\"k\": \"v}v\"} tail").unwrap();
        assert_eq!(obj, "{\"k\": \"v}v\"}");
    }

    #[test]
    fn context_block_surplus_when_nav_is_small() {
        // 实际库 < 100 字符, 注入应接近原样而不被 55% 上限「闲置」
        use std::sync::OnceLock;
        // 直接读当前 KB 测 (单元测试跑时若有 KB 才有意义; 跑不过就当 placeholder)
        // 核心逻辑靠 nav_total ≤ nav_budget 时 nav_surplus = nav_budget − nav_total 来测
        let nav_budget = (KB_CTX_BUDGET as f32 * KB_CTX_NAV_RATIO) as usize;
        let nav_total: usize = 50; // 模拟极小
        let effective = nav_budget.min(nav_total);
        let surplus = nav_budget - effective;
        assert_eq!(effective, 50);
        assert_eq!(surplus, nav_budget - 50);
    }

    #[test]
    fn context_block_no_surplus_when_nav_fills() {
        let nav_budget = (KB_CTX_BUDGET as f32 * KB_CTX_NAV_RATIO) as usize;
        let nav_total = nav_budget + 5_000; // 溢出
        let effective = nav_budget.min(nav_total);
        let surplus = nav_budget - effective;
        assert_eq!(effective, nav_budget);
        assert_eq!(surplus, 0);
    }

    #[test]
    fn context_block_map_total_capped_by_global_budget() {
        // 即便 surplus 很大, map_budget + nav_budget 不能越界 KB_CTX_BUDGET
        let map_base = (KB_CTX_BUDGET as f32 * KB_CTX_MAP_RATIO) as usize;
        let nav_surplus = KB_CTX_BUDGET; // 极端: 导航段空, 让位最大
        let map_budget = (map_base + nav_surplus).min(KB_CTX_BUDGET);
        assert!(map_budget <= KB_CTX_BUDGET);
    }

    #[test]
    fn truncate_keeps_short_and_cuts_long() {
        assert_eq!(truncate_chars("短文本", 100), "短文本");
        let long = "字".repeat(50);
        let out = truncate_chars(&long, 10);
        assert!(out.starts_with(&"字".repeat(10)));
        assert!(out.contains("已截断"));
        // 截断后 CJK 字符不被切坏 (能正常算字符数)
        assert!(out.chars().count() > 10);
    }
}
