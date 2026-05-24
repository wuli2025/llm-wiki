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

use anyhow::Result;
use directories::{ProjectDirs, UserDirs};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use walkdir::WalkDir;

// ───────────────────────── State ─────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct KbDoc {
    pub rel_path: String,
    pub title: String,
    pub category: String,
    pub wikilinks: Vec<String>,
    pub body: String,
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
    let docs = scan_all(&root);
    *INDEX.write() = docs;
    Ok(())
}

fn default_kb_root() -> Result<PathBuf> {
    let user = UserDirs::new().ok_or_else(|| anyhow::anyhow!("no user dir"))?;
    let home = user.home_dir();
    Ok(home.join("Polaris").join("PolarisKB"))
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

    // category
    let mut category = String::new();
    let mut fm_title: Option<String> = None;
    for cap in RE_YAML_KV.captures_iter(&fm) {
        let k = cap.get(1).map(|m| m.as_str()).unwrap_or("").to_lowercase();
        let v = cap.get(2).map(|m| m.as_str().trim().trim_matches('"')).unwrap_or("");
        match k.as_str() {
            "category" => category = v.to_string(),
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
        wikilinks,
        body: body_only,
    })
}

// ───────────────────────── Tauri commands ────────────────

#[tauri::command]
pub fn kb_root() -> String {
    KB_ROOT.read().to_string_lossy().to_string()
}

#[tauri::command]
pub fn kb_default_root() -> String {
    default_kb_root()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default()
}

#[tauri::command]
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

#[tauri::command]
pub fn kb_scan() -> Result<usize, String> {
    let root = KB_ROOT.read().clone();
    let docs = scan_all(&root);
    let n = docs.len();
    *INDEX.write() = docs;
    Ok(n)
}

#[tauri::command]
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

#[tauri::command]
pub fn kb_read(rel_path: String) -> Result<String, String> {
    let root = KB_ROOT.read().clone();
    let full = root.join(&rel_path);
    if !full.starts_with(&root) {
        return Err("path escapes KB root".into());
    }
    fs::read_to_string(&full).map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct KbHit {
    pub path: String,
    pub title: String,
    pub snippet: String,
    pub score: f64,
}

/// PRD §8.8 关键词加权评分: 标题 +10 / category +8 / 正文 +1
#[tauri::command]
pub fn kb_search(query: String, top_k: Option<usize>) -> Vec<KbHit> {
    let q = query.to_lowercase();
    let terms: Vec<&str> = q.split_whitespace().collect();
    if terms.is_empty() {
        return vec![];
    }
    let topk = top_k.unwrap_or(8);
    let idx = INDEX.read();
    let mut scored: Vec<(f64, &KbDoc, String)> = idx
        .iter()
        .filter_map(|d| {
            let title_lc = d.title.to_lowercase();
            let cat_lc = d.category.to_lowercase();
            let body_lc = d.body.to_lowercase();
            let mut score = 0.0;
            for t in &terms {
                if title_lc.contains(t) {
                    score += 10.0;
                }
                if !cat_lc.is_empty() && cat_lc.contains(t) {
                    score += 8.0;
                }
                let body_count = body_lc.matches(t).count() as f64;
                score += body_count;
            }
            if score < 1.0 {
                return None;
            }
            // snippet around first term hit
            let snippet = first_snippet(&d.body, &terms, 160);
            Some((score, d, snippet))
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .take(topk)
        .map(|(score, d, snippet)| KbHit {
            path: d.rel_path.clone(),
            title: d.title.clone(),
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

/// MVP ingest: 把外部文件复制到 raw/<filename> 并刷新索引
#[tauri::command]
pub fn kb_ingest(source_path: String) -> Result<String, String> {
    let src = PathBuf::from(&source_path);
    if !src.exists() {
        return Err(format!("source not found: {}", source_path));
    }
    let ext = src.extension().and_then(|s| s.to_str()).unwrap_or("");
    if ext != "md" && ext != "markdown" && ext != "txt" {
        return Err(format!(
            "MVP 仅支持 .md/.markdown/.txt,当前: .{}",
            ext
        ));
    }
    let root = KB_ROOT.read().clone();
    let dst_dir = root.join("raw");
    fs::create_dir_all(&dst_dir).map_err(|e| e.to_string())?;
    let fname = src
        .file_name()
        .ok_or_else(|| "no file name".to_string())?
        .to_owned();
    let dst = dst_dir.join(&fname);
    fs::copy(&src, &dst).map_err(|e| e.to_string())?;

    // 刷新索引
    let docs = scan_all(&root);
    *INDEX.write() = docs;

    Ok(format!("raw/{}", fname.to_string_lossy()))
}

// ───────────────────────── Graph ─────────────────────────

#[derive(Serialize)]
pub struct KbNode {
    pub id: String,
    pub title: String,
    pub category: String,
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

#[tauri::command]
pub fn kb_graph() -> KbGraph {
    let idx = INDEX.read();
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
    let nodes: Vec<KbNode> = idx
        .iter()
        .map(|d| KbNode {
            id: d.rel_path.clone(),
            title: d.title.clone(),
            category: d.category.clone(),
        })
        .collect();
    let mut edges: Vec<KbEdge> = Vec::new();
    for d in idx.iter() {
        for link in &d.wikilinks {
            let key = link.to_lowercase();
            if let Some(target) = title_to_path.get(&key) {
                if target != &d.rel_path {
                    edges.push(KbEdge {
                        source: d.rel_path.clone(),
                        target: target.clone(),
                    });
                }
            }
        }
    }
    KbGraph { nodes, edges }
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
