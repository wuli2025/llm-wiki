//! 最小 Skill 系统 — MVP v0.2
//!
//! Skill = 预设 system prompt，激活时注入到用户 prompt 顶部。
//! 当前全部内建，不存磁盘；Phase 3 再扩展为可用户自定义。

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source: String, // "official" | "third-party"
    #[serde(skip)]
    pub system_prompt: &'static str,
}

/// 所有内建 skill
pub fn built_in_skills() -> Vec<Skill> {
    vec![
        Skill {
            id: "deep-research".into(),
            name: "深度搜索".into(),
            description: "使用 LLM 大规模联网搜索相关内容，自动检索、汇总、交叉验证多来源信息".into(),
            source: "third-party".into(),
            system_prompt: include_str!("templates/skills/deep-research.md"),
        },
        Skill {
            id: "skill-creator".into(),
            name: "Skill 创建向导".into(),
            description: "引导用户创建自定义 Skill，自动生成模板和配置文件".into(),
            source: "official".into(),
            system_prompt: include_str!("templates/skills/skill-creator.md"),
        },
    ]
}

/// 根据 ID 查找 skill（返回完整信息，含 system_prompt）
pub fn find(id: &str) -> Option<Skill> {
    built_in_skills().into_iter().find(|s| s.id == id)
}

/// 检测用户消息是否包含创建 skill 的意图
/// 触发词："创建skill"、"新建skill"、"写skill"、"做一个skill"、"skill创建"
pub fn detect_skill_creation_intent(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();
    let triggers = [
        "创建skill",
        "新建skill",
        "写skill",
        "做一个skill",
        "skill创建",
        "skill新建",
        "skill制作",
        "创建技能",
        "新建技能",
        "写技能",
    ];
    triggers.iter().any(|t| lower.contains(t))
}

/// 获取默认 skill（当用户意图匹配时自动激活）
pub fn default_skill_for_intent(prompt: &str) -> Option<Skill> {
    if detect_skill_creation_intent(prompt) {
        find("skill-creator")
    } else {
        None
    }
}

// ───────────────────────── Tauri Commands ─────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct SkillMeta {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source: String,
}

#[tauri::command]
pub fn list_skills() -> Vec<SkillMeta> {
    built_in_skills()
        .into_iter()
        .map(|s| SkillMeta {
            id: s.id,
            name: s.name,
            description: s.description,
            source: s.source,
        })
        .collect()
}

#[tauri::command]
pub fn get_skill(id: String) -> Result<Skill, String> {
    find(&id).ok_or_else(|| format!("Skill '{}' 不存在", id))
}
