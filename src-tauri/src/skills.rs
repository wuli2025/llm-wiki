//! Skill 系统 — MVP v0.3
//!
//! 内建 skill（编译期）+ 用户 skill（磁盘持久化，~/Polaris/skills/）

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

// ═══════════════════════════════════════════════════════════════
// 内建 Skills（编译期，只读）
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize)]
pub struct BuiltInSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source: String,
    #[serde(skip)]
    pub system_prompt: &'static str,
}

fn built_in_skills() -> Vec<BuiltInSkill> {
    vec![
        BuiltInSkill {
            id: "deep-research".into(),
            name: "深度搜索".into(),
            description: "使用 LLM 大规模联网搜索相关内容，自动检索、汇总、交叉验证多来源信息".into(),
            source: "third-party".into(),
            system_prompt: include_str!("templates/skills/deep-research.md"),
        },
        BuiltInSkill {
            id: "skill-creator".into(),
            name: "Skill 创建向导".into(),
            description: "引导用户创建自定义 Skill，自动生成模板和配置文件".into(),
            source: "official".into(),
            system_prompt: include_str!("templates/skills/skill-creator.md"),
        },
    ]
}

fn find_built_in(id: &str) -> Option<BuiltInSkill> {
    built_in_skills().into_iter().find(|s| s.id == id)
}

// ═══════════════════════════════════════════════════════════════
// 用户 Skills（磁盘持久化）
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize)]
pub struct UserSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub created_at: i64,
    #[serde(skip)]
    pub system_prompt: String,
}

/// 用户 skills 根目录: ~/Polaris/skills/
fn skills_dir() -> Option<PathBuf> {
    directories::UserDirs::new().map(|u| u.home_dir().join("Polaris").join("skills"))
}

/// 扫描用户 skills 目录，返回所有用户 skill
fn scan_user_skills() -> Vec<UserSkill> {
    let Some(root) = skills_dir() else { return vec![] };
    let Ok(entries) = fs::read_dir(&root) else { return vec![] };

    let mut skills = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() { continue; }
        let skill_file = path.join("skill.md");
        if !skill_file.exists() { continue; }
        if let Ok(skill) = parse_skill_file(&skill_file) {
            skills.push(skill);
        }
    }
    skills.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    skills
}

/// 解析 skill.md 文件: YAML frontmatter + body
fn parse_skill_file(path: &Path) -> Result<UserSkill, String> {
    let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let lines: Vec<&str> = content.lines().collect();

    // 找 frontmatter 边界 ---
    if lines.len() < 3 || lines[0].trim() != "---" {
        return Err("missing frontmatter".into());
    }
    let mut end_idx = 0;
    for (i, line) in lines.iter().enumerate().skip(1) {
        if line.trim() == "---" {
            end_idx = i;
            break;
        }
    }
    if end_idx == 0 {
        return Err("unclosed frontmatter".into());
    }

    // 解析 frontmatter key: value
    let mut id = String::new();
    let mut name = String::new();
    let mut description = String::new();
    let mut author = "user".to_string();
    let mut created_at = 0i64;

    for line in &lines[1..end_idx] {
        let line = line.trim();
        if line.is_empty() { continue; }
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim();
            let v = v.trim().trim_matches('"').trim_matches('\'');
            match k {
                "id" => id = v.to_string(),
                "name" => name = v.to_string(),
                "description" => description = v.to_string(),
                "author" => author = v.to_string(),
                "created_at" => created_at = v.parse().unwrap_or(0),
                _ => {}
            }
        }
    }

    let system_prompt = lines[end_idx + 1..].join("\n").trim().to_string();

    if id.is_empty() {
        // fallback: 用目录名做 id
        id = path.parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
    }
    if name.is_empty() {
        name = id.clone();
    }

    Ok(UserSkill { id, name, description, author, created_at, system_prompt })
}

/// 创建新 skill
fn create_user_skill(
    id: &str,
    name: &str,
    description: &str,
    system_prompt: &str,
) -> Result<(), String> {
    let Some(root) = skills_dir() else {
        return Err("无法获取用户目录".into());
    };
    let dir = root.join(id);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let content = format!(
        "---\nid: {}\nname: {}\ndescription: {}\nauthor: user\ncreated_at: {}\n---\n\n{}\n",
        id, name, description, now, system_prompt
    );

    fs::write(dir.join("skill.md"), content).map_err(|e| e.to_string())?;
    Ok(())
}

/// 删除 skill
fn delete_user_skill(id: &str) -> Result<(), String> {
    let Some(root) = skills_dir() else {
        return Err("无法获取用户目录".into());
    };
    let dir = root.join(id);
    if !dir.exists() {
        return Err("skill 不存在".into());
    }
    fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(())
}

/// 读取单个用户 skill 的 system_prompt
fn load_user_skill_system_prompt(id: &str) -> Option<String> {
    let root = skills_dir()?;
    let file = root.join(id).join("skill.md");
    parse_skill_file(&file).ok().map(|s| s.system_prompt)
}

// ═══════════════════════════════════════════════════════════════
// 统一接口（内建 + 用户）
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize)]
pub struct SkillMeta {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source: String,
}

/// 查找 skill（优先用户 skill，再内建）
pub fn find(id: &str) -> Option<(SkillMeta, String)> {
    // 先查用户
    for user in scan_user_skills() {
        if user.id == id {
            return Some((
                SkillMeta {
                    id: user.id,
                    name: user.name,
                    description: user.description,
                    source: "user".into(),
                },
                user.system_prompt,
            ));
        }
    }
    // 再查内建
    find_built_in(id).map(|s| (
        SkillMeta {
            id: s.id,
            name: s.name,
            description: s.description,
            source: s.source,
        },
        s.system_prompt.to_string(),
    ))
}

/// 检测用户消息是否包含创建 skill 的意图
pub fn detect_skill_creation_intent(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();
    let triggers = [
        "创建skill", "新建skill", "写skill", "做一个skill",
        "skill创建", "skill新建", "skill制作",
        "创建技能", "新建技能", "写技能",
    ];
    triggers.iter().any(|t| lower.contains(t))
}

/// 获取默认 skill（当用户意图匹配时自动激活）
pub fn default_skill_for_intent(prompt: &str) -> Option<(SkillMeta, String)> {
    if detect_skill_creation_intent(prompt) {
        find("skill-creator")
    } else {
        None
    }
}

// ═══════════════════════════════════════════════════════════════
// Tauri Commands
// ═══════════════════════════════════════════════════════════════

#[tauri::command]
pub fn list_skills() -> Vec<SkillMeta> {
    let mut list = Vec::new();

    // 用户 skills
    for user in scan_user_skills() {
        list.push(SkillMeta {
            id: user.id,
            name: user.name,
            description: user.description,
            source: "user".into(),
        });
    }

    // 内建 skills
    for bi in built_in_skills() {
        // 去重: 若用户已覆盖同名 skill，跳过内建
        if !list.iter().any(|s| s.id == bi.id) {
            list.push(SkillMeta {
                id: bi.id,
                name: bi.name,
                description: bi.description,
                source: bi.source,
            });
        }
    }

    list
}

#[tauri::command]
pub fn get_skill(id: String) -> Result<SkillMeta, String> {
    find(&id).map(|(meta, _)| meta).ok_or_else(|| format!("Skill '{}' 不存在", id))
}

#[derive(Debug, Deserialize)]
pub struct CreateSkillArgs {
    pub id: String,
    pub name: String,
    pub description: String,
    pub system_prompt: String,
}

#[tauri::command]
pub fn create_skill(args: CreateSkillArgs) -> Result<(), String> {
    // 校验 id: 只允许小写字母、数字、-、_
    if !args.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') {
        return Err("Skill ID 只能包含小写字母、数字、-、_".into());
    }
    create_user_skill(&args.id, &args.name, &args.description, &args.system_prompt)
}

#[tauri::command]
pub fn delete_skill(id: String) -> Result<(), String> {
    // 不能删除内建 skill
    if find_built_in(&id).is_some() {
        return Err("不能删除内建 skill".into());
    }
    delete_user_skill(&id)
}
