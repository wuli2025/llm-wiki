//! 板块⑫ 人格模块 — 预设人格注册表 + 应用到项目
//!
//! 思想来源: WeSight 的 preset agent（右侧选人格、每个项目=一个人格）。
//! Polaris 自研实现: 人格正文 = 项目的 `CLAUDE.md`（复用既有注入链路 `claude_md::render_for_project`），
//! 本模块只负责「预设库」与「一键应用到当前项目」+「绑定该人格的专属知识库 scope」。

use directories::UserDirs;
use serde::Serialize;
use std::fs;
use std::path::PathBuf;

/// 一个预设人格（对外给前端画廊用）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PersonaPreset {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub description: String,
    /// 建议绑定的知识库范围（KB 根下相对子目录，空=全局）
    pub kb_scope: String,
    /// 人格正文（写入项目 CLAUDE.md 的内容）
    pub body: String,
}

// 预设正文（编译期内嵌）。毛主席沿用既有模板，作为内置彩蛋人格。
const STOCK: &str = include_str!("templates/personas/stock-expert.md");
const WRITER: &str = include_str!("templates/personas/content-writer.md");
const LESSON: &str = include_str!("templates/personas/lesson-planner.md");
const SUMMARY: &str = include_str!("templates/personas/content-summarizer.md");
const HEALTH: &str = include_str!("templates/personas/health-interpreter.md");
const PET: &str = include_str!("templates/personas/pet-care.md");
const MAO: &str = include_str!("templates/mao_persona_claude.md");

fn presets() -> Vec<PersonaPreset> {
    let mk = |id: &str, name: &str, icon: &str, desc: &str, scope: &str, body: &str| PersonaPreset {
        id: id.into(),
        name: name.into(),
        icon: icon.into(),
        description: desc.into(),
        kb_scope: scope.into(),
        body: body.into(),
    };
    vec![
        mk("stock-expert", "股票助手", "📈", "A 股深度分析 / 公告监控 / 行情查询，数据驱动客观分析。", "raw/股票", STOCK),
        mk("content-writer", "内容创作", "✍️", "公众号/自媒体写手：选题、撰写、5 种风格、排版钩子。", "raw/创作", WRITER),
        mk("lesson-planner", "备课出卷", "📚", "K12 教案/试卷/答案解析，难度分布可控，输出 docx/xlsx。", "raw/教学", LESSON),
        mk("content-summarizer", "内容总结", "📋", "网页/文档/会议纪要的结构化摘要：一句话→要点→详细→行动项。", "", SUMMARY),
        mk("health-interpreter", "医疗健康解读", "🏥", "体检报告/化验单通俗解读，分级标注，附免责声明。", "raw/健康", HEALTH),
        mk("pet-care", "萌宠管家", "🐾", "猫狗行为/健康/营养，温暖亲切，安全禁忌优先。", "raw/萌宠", PET),
        mk("mao", "毛主席", "☭", "毛选式客观分析：矛盾分析、实事求是、同志称呼、引用克制。", "raw/毛主席", MAO),
    ]
}

/// 项目工作目录的 CLAUDE.md 路径（须与 conv::write_mao_persona / claude_md 一致）。
fn project_claude_md_path(project_id: &str) -> Option<PathBuf> {
    // 安全闸: 防 project_id 走 `..` 越出 projects 根写任意 CLAUDE.md(见 conv::is_safe_project_id)。
    if !crate::conv::is_safe_project_id(project_id) {
        return None;
    }
    let user = UserDirs::new()?;
    Some(
        user.home_dir()
            .join("Polaris")
            .join("projects")
            .join(project_id)
            .join("CLAUDE.md"),
    )
}

// ───────────────────────── Tauri commands ─────────────────────────

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn persona_list() -> Vec<PersonaPreset> {
    presets()
}

/// 把某预设人格应用到指定项目：写入该项目 CLAUDE.md + 绑定建议的知识库 scope。
/// `overwrite=false` 且已有非占位内容时拒绝覆盖（交前端二次确认后再 true）。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn persona_apply(
    project_id: String,
    persona_id: String,
    overwrite: bool,
) -> Result<(), String> {
    let preset = presets()
        .into_iter()
        .find(|p| p.id == persona_id)
        .ok_or_else(|| format!("未知人格预设: {}", persona_id))?;

    let path = project_claude_md_path(&project_id).ok_or("无法确定项目路径")?;
    if !overwrite && path.exists() {
        let existing = fs::read_to_string(&path).unwrap_or_default();
        if !existing.trim().is_empty()
            && !existing.contains(crate::claude_md::PLACEHOLDER_MARKER)
        {
            return Err("该项目已有人格内容，确认覆盖请重试。".into());
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(&path, &preset.body).map_err(|e| e.to_string())?;

    // 绑定该人格的专属知识库 scope（空字符串=全局）
    let scope = if preset.kb_scope.trim().is_empty() {
        None
    } else {
        Some(preset.kb_scope.clone())
    };
    crate::conv::set_project_persona(&project_id, Some(persona_id), scope);
    Ok(())
}
