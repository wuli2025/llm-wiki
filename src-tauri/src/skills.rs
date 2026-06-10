//! Skill 系统 — MVP v0.4
//!
//! 统一目录 catalog（编译期内置 + 可安装市场）+ 用户 skill（磁盘持久化，~/Polaris/skills/）
//!
//! - 预装 skill（preinstalled=true）：开箱即用，始终 installed
//! - 市场 skill（preinstalled=false）：列在「市场精选」，点「安装」即复制到用户目录
//! - 用户自建 skill：create_skill 写盘，source = user
//! - 安装 / 创建都会立即出现在技能中心；前端负责安装后自动激活（无需额外授权步骤）

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

// ───────── 「网页演示视频」打包技能（ConardLi · Polaris 集成）─────────
// 这是个多文件技能包：安装时 git clone 原仓库整目录到用户技能目录，再叠加
// 三个 Polaris 助手脚本（这里编译期内嵌），最后拼出 skill.md。
const WVP_ID: &str = "web-video-presentation";
const WVP_REPO: &str = "https://github.com/ConardLi/garden-skills";
const WVP_ADDENDUM: &str = include_str!("templates/skills/web-video-presentation/addendum.md");
const WVP_MINIMAX_TTS: &str =
    include_str!("templates/skills/web-video-presentation/minimax-tts.mjs");
const WVP_SCAFFOLD: &str = include_str!("templates/skills/web-video-presentation/scaffold.mjs");
const WVP_BOOTSTRAP: &str = include_str!("templates/skills/web-video-presentation/bootstrap.mjs");

// ───────── 「课件视频工坊」多文件技能（Polaris 自研，编译期内嵌，启动落盘）─────────
// 支撑「生成课件类视频」UI 的基础设施技能：含 SKILL.md + 三个脚本 + 链路文档。
// 不像 WVP 走 git clone，这套全自研，所有文件编译期内嵌、启动时确保落到 ~/Polaris/skills，
// 以便：① 全新安装即可用；② 改了脚本后随 App 更新下发（靠 PVS_VERSION 比对覆盖）。
const PVS_ID: &str = "polaris-video-studio";
// 改动内嵌脚本/SKILL.md 后必须 +1，让已安装用户在下次启动时拿到更新。
const PVS_VERSION: &str = "3";
const PVS_SKILL_MD: &str = include_str!("templates/skills/polaris-video-studio/SKILL.md");
const PVS_MANIFEST: &str = include_str!("templates/skills/polaris-video-studio/manifest.json");
const PVS_INSTALL_DEPS: &str =
    include_str!("templates/skills/polaris-video-studio/scripts/install-deps.mjs");
const PVS_RUN: &str = include_str!("templates/skills/polaris-video-studio/scripts/run.mjs");
const PVS_RECORD: &str =
    include_str!("templates/skills/polaris-video-studio/scripts/pipeline/03-record.mjs");
const PVS_WORKFLOW: &str =
    include_str!("templates/skills/polaris-video-studio/references/WORKFLOW.md");

// ───────── 「演示工坊」多文件技能（PPT / 网页幻灯片，Polaris 自研，编译期内嵌，启动落盘）─────────
// 支撑「PPT 演示」「网页幻灯片」两个 UI 入口的基础设施技能：幻灯片引擎(base.css，来自
// open-design，MIT) + 17 套主题 + 自研 runtime.js + deck 模板 + PPTX 导出脚本 + SKILL.md。
// 与 PVS 同套路：全部编译期内嵌、启动时确保落到 ~/Polaris/skills（靠 DECK_VERSION 比对覆盖）。
const DECK_ID: &str = "polaris-deck-studio";
// 改动任一内嵌资源后必须 +1，让已安装用户下次启动拿到更新。
const DECK_VERSION: &str = "3";
const DECK_SKILL_MD: &str = include_str!("templates/skills/polaris-deck-studio/SKILL.md");
const DECK_LICENSE: &str = include_str!("templates/skills/polaris-deck-studio/LICENSE");
const DECK_BASE_CSS: &str = include_str!("templates/skills/polaris-deck-studio/assets/base.css");
const DECK_THEMES_CSS: &str =
    include_str!("templates/skills/polaris-deck-studio/assets/themes.css");
const DECK_RUNTIME_JS: &str =
    include_str!("templates/skills/polaris-deck-studio/assets/runtime.js");
const DECK_TEMPLATE: &str =
    include_str!("templates/skills/polaris-deck-studio/templates/deck.html");
const DECK_INSTALL_DEPS: &str =
    include_str!("templates/skills/polaris-deck-studio/scripts/install-deps.mjs");
const DECK_EXPORT_PPTX: &str =
    include_str!("templates/skills/polaris-deck-studio/scripts/export-pptx.mjs");

// ───────── 「网站生成」技能（落地页/单页站点，Polaris 自研，编译期内嵌，启动落盘）─────────
// 支撑「网站生成」UI 入口。复用 deck-studio 的 17 套主题（DECK_THEMES_CSS，不重复源文件），
// 配一套网站组件 site.css + 滚动揭示 runtime.js + 站点模板 + SKILL.md。
const WEB_ID: &str = "polaris-web-studio";
const WEB_VERSION: &str = "2";
const WEB_SKILL_MD: &str = include_str!("templates/skills/polaris-web-studio/SKILL.md");
const WEB_LICENSE: &str = include_str!("templates/skills/polaris-web-studio/LICENSE");
const WEB_SITE_CSS: &str = include_str!("templates/skills/polaris-web-studio/assets/site.css");
const WEB_RUNTIME_JS: &str =
    include_str!("templates/skills/polaris-web-studio/assets/runtime.js");
const WEB_TEMPLATE: &str =
    include_str!("templates/skills/polaris-web-studio/templates/site.html");

// ───────── 「壹伴排版优化」多文件技能（公众号排版，编译期内嵌，启动落盘）─────────
// 升级成多文件：SKILL.md（只产语义正文）+ scripts/wechat_yiban.py（壹伴样式引擎 + CloakBrowser
// 驱动）。和 deck/video studio 同套路：编译期内嵌、启动确保落到 ~/Polaris/skills（靠版本号比对覆盖），
// 这样脚本能被 spawn 的 claude agent 在磁盘上直接 `python …/wechat_yiban.py` 执行。
const WECHAT_TS_ID: &str = "wechat-md-typesetter";
// 改动 SKILL.md 或 wechat_yiban.py 后必须 +1，让已安装用户下次启动拿到更新。
// v2：修真机测出的两个 bug —— cloakbrowser API 是 headless= 非 headed=；render 改 about:blank+evaluate 避开 set_content 等 "load" 超时。
// v3：publish 健壮性 —— 跨「所有标签页×所有 frame」找编辑器（写图文常开新标签）；登录后自动点一次「写图文」入口；
//     兜底 render 复用已开 ctx 开新标签（不再另起同步 Playwright→消除 'Sync API inside asyncio loop' 崩溃）。
// v4：按真机 dump 校 SELECTORS —— editor_body 加新版 ProseMirror；title_input 改专指文章标题(避开草稿箱搜索框)；
//     new_article_entry 精确点「写图文」。注：editor_body 仍需真机在编辑器打开态确认一次。
// v5：两段解耦根治「老卡在上传」—— 注入改走「粘贴通道」(合成 ClipboardEvent，走 ProseMirror 事务，
//     和真壹伴/135editor 同路；innerHTML 硬塞会被 PM 清掉/不入草稿数据)，配三级降级+字数校验+保存回执；
//     publish 拆两段(先纯文字稳传入草稿，再套样式，样式挂了文字也已落地)；新增 --text-only 与
//     restyle 模式(对已存草稿原地换主题，normalize 后幂等不叠样式)；--timeout 可调(默认 300s)。
// v6：panel 模式(壹伴插件形态)——编辑器页面右侧注入可视化面板：7 套主题模板点选换肤 +「AI 改风格」
//     大白话定制(expose_function 桥→python→claude CLI 生成主题 JSON)+清除样式+保存草稿；
//     STYLIZE 支持主题对象/bg 整体背景/overrides 微调/plain 素颜，新增清新绿/活力橙/米纸预设。
// v7：按真机反馈三修——①背景改不了：bg 从「包 section」改「按块铺设」(每块自带 background 内联,
//     编辑器剥不掉,135editor 同法)；②换肤改「原地直改活 DOM」优先(像浏览器插件改 HTML),粘贴通道
//     降为兜底；③AI 用不了：expose_function 桥换成页面变量轮询握手(__polarisAI.pending↔__polarisAIResult)；
//     模板升到 8 套×6 种标题形态(h2Mode)，新增黛青。
// v8：长图链路(用户拍板的新路线,根治编辑器改字数/清样式)——snapshot=成品 HTML→全页截长图(@2x)
//     →段落空隙切片(下钻单子链找段落层;clip 配 full_page=True 才能裁视口外)+manifest;
//     publish-image=切片转 dataURL→File→合成 paste 贴进编辑器(原生欢迎零清洗)→等 img 落位/
//     换 mmbiz 外链→真文字导语(--intro)→填标题→存草稿。MediaOps 加「长图模式」开关(__longimg,
//     隐式带上本技能)。snapshot 已本地实测(单/多切片+段落切点目检);publish-image 待真机。
const WECHAT_TS_VERSION: &str = "8";
const WECHAT_TS_SKILL_MD: &str =
    include_str!("templates/skills/wechat-md-typesetter/SKILL.md");
const WECHAT_TS_YIBAN_PY: &str =
    include_str!("templates/skills/wechat-md-typesetter/scripts/wechat_yiban.py");

// ═══════════════════════════════════════════════════════════════
// 统一目录 Catalog（编译期，只读）
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub struct CatalogSkill {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub source: &'static str, // official | third-party
    /// true = 预装（始终可用，无需安装），false = 市场技能（需点安装）
    pub preinstalled: bool,
    pub system_prompt: &'static str,
}

fn catalog() -> Vec<CatalogSkill> {
    vec![
        // ── 预装（开箱即用） ──
        CatalogSkill {
            id: "deep-research",
            name: "深度搜索",
            description: "使用 LLM 大规模联网搜索相关内容，自动检索、汇总、交叉验证多来源信息",
            source: "third-party",
            preinstalled: true,
            system_prompt: include_str!("templates/skills/deep-research.md"),
        },
        // ── 名人资料包配套技能（随知识库「名人资料包」一起装/卸，不单独预装） ──
        CatalogSkill {
            id: "consult-mao",
            name: "请教毛主席",
            description: "化身毛主席，用毛选式大白话+矛盾分析法客观分析问题：调毛主席资料库取证、站在未来看今天，生成标注来源的自包含 HTML。随「毛主席」名人资料包一起安装，装好后消息里写「请教毛主席」即自动激活",
            source: "official",
            preinstalled: false,
            system_prompt: include_str!("templates/skills/consult-mao.md"),
        },
        CatalogSkill {
            id: "skill-creator",
            name: "Skill 创建向导",
            description: "引导用户创建自定义 Skill，自动生成模板和配置文件",
            source: "official",
            preinstalled: true,
            system_prompt: include_str!("templates/skills/skill-creator.md"),
        },
        // ── 市场（点安装即用） ──
        CatalogSkill {
            id: "pdf",
            name: "PDF 文档处理",
            description: "提取 / 生成 / 编辑 PDF：抽取文本表格、合并拆分、Markdown 转 PDF、表单与 OCR",
            source: "official",
            preinstalled: false,
            system_prompt: include_str!("templates/skills/pdf.md"),
        },
        CatalogSkill {
            id: "xlsx",
            name: "Excel 表格",
            description: "读取分析与生成 Excel：透视统计、公式、图表、多 sheet 报表",
            source: "official",
            preinstalled: false,
            system_prompt: include_str!("templates/skills/xlsx.md"),
        },
        CatalogSkill {
            id: "pptx",
            name: "PPT 演示文稿",
            description: "把 PDF / 文档 / 数据转成有高级感的 PPT：母版配色、版式层级、图表，python-pptx 生成",
            source: "official",
            preinstalled: false,
            system_prompt: include_str!("templates/skills/pptx.md"),
        },
        CatalogSkill {
            id: "edge-tts",
            name: "语音合成 Edge-TTS",
            description: "把文本转成自然语音音频，多语言多音色，免费无需 key",
            source: "third-party",
            preinstalled: false,
            system_prompt: include_str!("templates/skills/edge-tts.md"),
        },
        CatalogSkill {
            id: "hyperframes",
            name: "视频动画 Hyperframes",
            description: "用逐帧 / 分镜方式生成短视频与动画，ffmpeg 合成，可配 Edge-TTS 旁白",
            source: "third-party",
            preinstalled: false,
            system_prompt: include_str!("templates/skills/hyperframes.md"),
        },
        CatalogSkill {
            id: "web-search",
            name: "联网搜索",
            description: "实时联网检索，基于 Tavily / Brave 等真实来源回答并交叉验证",
            source: "third-party",
            preinstalled: false,
            system_prompt: include_str!("templates/skills/web-search.md"),
        },
        CatalogSkill {
            id: "image-gen",
            name: "AI 生图",
            description: "按描述生成图片：先检测当前供应商是否真的支持生图，不支持时用中文说明并改用「很有图片质感的 HTML」兜底",
            source: "third-party",
            preinstalled: false,
            system_prompt: include_str!("templates/skills/image-gen.md"),
        },
        // ── 源自 ConardLi 教程：完整可跑的网页演示视频技能包 ──
        CatalogSkill {
            id: WVP_ID,
            name: "网页演示视频制作（ConardLi·Polaris集成）",
            description: "把文稿做成 16:9 可点击翻页的网页演示再录屏成片。安装即下载完整脚手架+23主题+音频流水线，依赖自动装；配音自动调用 Polaris 内置 MiniMax（无需 mmx-cli / 登录 / GroupId）。Windows 走 Node 版一键跑通。",
            source: "third-party",
            preinstalled: false,
            system_prompt: WVP_ADDENDUM,
        },
        // ── 源自 ConardLi 教程的两套向导 ──
        CatalogSkill {
            id: "web-video-presentation-guide",
            name: "网页演示视频制作向导",
            description: "把文稿做成 16:9 可点击翻页的网页演示再录屏成片：逐检查点告诉你此刻该做什么，并引导引入 ConardLi 的 web-video-presentation 原 skill",
            source: "third-party",
            preinstalled: false,
            system_prompt: include_str!("templates/skills/web-video-presentation-guide.md"),
        },
        CatalogSkill {
            id: "harness-practices",
            name: "Harness 工程实践向导",
            description: "把 Claude Code 调教成生产力 harness：盘点瓶颈 → 技能化/供应商切换(CC Switch)/MiniMax CLI/子代理编排，逐步告诉你现在该做什么",
            source: "third-party",
            preinstalled: false,
            system_prompt: include_str!("templates/skills/harness-practices.md"),
        },
        // ── 自媒体全链路运营（交互决策版，与「自动化」里的两条流程同源） ──
        CatalogSkill {
            id: "wechat-pipeline",
            name: "微信公众号 · 全链路运营",
            description: "选题→风格→成稿→排版出图一条龙；每个决策点先讲思考再给编号选项让你挑、也可直接输入覆盖；风格可调；支持全自动",
            source: "third-party",
            preinstalled: true,
            system_prompt: include_str!("templates/skills/wechat-pipeline.md"),
        },
        CatalogSkill {
            id: "xiaohongshu-pipeline",
            name: "小红书 · 全链路运营",
            description: "选题→风格→文案→图卡渲染一条龙；每个决策点先讲思考再给编号选项让你挑、也可直接输入覆盖；风格可调；支持全自动",
            source: "third-party",
            preinstalled: true,
            system_prompt: include_str!("templates/skills/xiaohongshu-pipeline.md"),
        },
        // ── 自媒体全链路·配套三件套（选题前置 / 数据复盘 / 社群应对，补全闭环） ──
        CatalogSkill {
            id: "hot-topic-radar",
            name: "选题雷达",
            description: "联网抓热点+对标爆文，归纳成 3-5 个选题方向、每个给 2-3 个具体选题并做爆款拆解（为什么火/适合哪个平台/时效难度），编号供勾选；读 KB 避免撞题。可独立用，也是全链路第一步",
            source: "third-party",
            preinstalled: true,
            system_prompt: include_str!("templates/skills/hot-topic-radar.md"),
        },
        CatalogSkill {
            id: "content-analytics-report",
            name: "数据复盘 · 运营周报",
            description: "把一批已发文章/笔记的数据做成运营周报：逐篇打优劣势、找「哪类选题/标题/发布时机」数据好的规律、给下轮主攻方向，并回写 KB 反哺选题",
            source: "third-party",
            preinstalled: true,
            system_prompt: include_str!("templates/skills/content-analytics-report.md"),
        },
        CatalogSkill {
            id: "community-engagement",
            name: "评论 · 社群应对",
            description: "把评论/私信分类（提问/夸赞/抬杠/求合作/负面），按账号人格逐条起草回复，标出需本人亲自处理的高敏感项，并把高频疑问提炼成选题线索回写 KB",
            source: "third-party",
            preinstalled: true,
            system_prompt: include_str!("templates/skills/community-engagement.md"),
        },
        CatalogSkill {
            id: "xhs-mao-pipeline",
            name: "小红书 · 毛选风格发布",
            description: "调毛主席知识库析毛选文风→就给定主题写小红书爆款文案→出图(HTML图卡转截图 或 AI配图)→调 post-to-xhs 浏览器自动发布;发前必人工确认、可先预览、需扫码登录",
            source: "third-party",
            preinstalled: true,
            system_prompt: include_str!("templates/skills/xhs-mao-pipeline.md"),
        },
        // ── 壹伴排版优化（公众号排版 + CloakBrowser 直送草稿，根治格式错） ──
        CatalogSkill {
            id: "wechat-md-typesetter",
            name: "壹伴排版优化",
            description: "壹伴式排版：只产出干净语义正文（零内联样式），随包壹伴脚本在 CloakBrowser 公众号编辑器 DOM 上按约定风格一键套样式（标题色块/引用卡/分割线/列表转段落全内联），填标题存草稿（绝不自动发布）——根治粘贴格式错乱",
            source: "third-party",
            preinstalled: true,
            system_prompt: WECHAT_TS_SKILL_MD,
        },
        // ── 源自 ClaudeSkills 合集的两个内容创作技能（全链路成稿/出图时调用） ──
        CatalogSkill {
            id: "gz-wechat-article-writer",
            name: "公众号文章创作（ClaudeSkills）",
            description: "微信公众号文章创作助手：风格灵活适配（企业官号/个人技术博客/活动回顾/产品评测），优化标题与结构。全链路成稿阶段的内容引擎",
            source: "third-party",
            preinstalled: true,
            system_prompt: include_str!("templates/skills/gz-wechat-article-writer.md"),
        },
        CatalogSkill {
            id: "gz-notion-infographic",
            name: "信息图 / 小红书图文（ClaudeSkills）",
            description: "按大纲自动研究并生成高质量可视化：Notion 手绘风信息图组图 / PPTX，适合小红书图文与社媒传播图。全链路渲染阶段的图卡引擎",
            source: "third-party",
            preinstalled: true,
            system_prompt: include_str!("templates/skills/gz-notion-infographic.md"),
        },
        // ── 默认浏览器插件（预装、默认开启，可随时移除） ──
        CatalogSkill {
            id: "cloak-browser",
            name: "CloakBrowser 浏览器",
            description: "Agent 默认浏览器：源码级隐身 Chromium，drop-in 替换 Playwright，过 Cloudflare / 反爬。可随时关闭移除",
            source: "third-party",
            preinstalled: true,
            system_prompt: include_str!("templates/skills/cloak-browser.md"),
        },
    ]
}

fn find_catalog(id: &str) -> Option<CatalogSkill> {
    catalog().into_iter().find(|c| c.id == id)
}

// ═══════════════════════════════════════════════════════════════
// 用户 Skills（磁盘持久化）
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize)]
pub struct UserSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    /// 来源：用户自建为 "user"；从市场安装时保留原始 source（official / third-party）
    pub source: String,
    pub author: String,
    pub created_at: i64,
    #[serde(skip)]
    pub system_prompt: String,
}

/// 用户 skills 根目录: ~/Polaris/skills/
fn skills_dir() -> Option<PathBuf> {
    directories::UserDirs::new().map(|u| u.home_dir().join("Polaris").join("skills"))
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 扫描用户 skills 目录，返回所有用户 skill
fn scan_user_skills() -> Vec<UserSkill> {
    let Some(root) = skills_dir() else {
        return vec![];
    };
    let Ok(entries) = fs::read_dir(&root) else {
        return vec![];
    };

    let mut skills = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let skill_file = path.join("skill.md");
        if !skill_file.exists() {
            continue;
        }
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
    let mut source = "user".to_string();
    let mut author = "user".to_string();
    let mut created_at = 0i64;

    for line in &lines[1..end_idx] {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim();
            let v = v.trim().trim_matches('"').trim_matches('\'');
            match k {
                "id" => id = v.to_string(),
                "name" => name = v.to_string(),
                "description" => description = v.to_string(),
                "source" => source = v.to_string(),
                "author" => author = v.to_string(),
                "created_at" => created_at = v.parse().unwrap_or(0),
                _ => {}
            }
        }
    }

    let system_prompt = lines[end_idx + 1..].join("\n").trim().to_string();

    if id.is_empty() {
        // fallback: 用目录名做 id
        id = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
    }
    if name.is_empty() {
        name = id.clone();
    }

    Ok(UserSkill {
        id,
        name,
        description,
        source,
        author,
        created_at,
        system_prompt,
    })
}

/// 把一份 skill.md 写到用户目录（创建 / 安装共用）
fn write_skill_file(
    id: &str,
    name: &str,
    description: &str,
    source: &str,
    author: &str,
    system_prompt: &str,
) -> Result<(), String> {
    let Some(root) = skills_dir() else {
        return Err("无法获取用户目录".into());
    };
    let dir = root.join(id);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let content = format!(
        "---\nid: {}\nname: {}\ndescription: {}\nsource: {}\nauthor: {}\ncreated_at: {}\n---\n\n{}\n",
        id,
        name,
        description,
        source,
        author,
        now_secs(),
        system_prompt
    );

    fs::write(dir.join("skill.md"), content).map_err(|e| e.to_string())?;
    Ok(())
}

/// 删除用户目录里的 skill 副本（= 卸载 / 删除）
fn remove_user_skill(id: &str) -> Result<(), String> {
    let Some(root) = skills_dir() else {
        return Err("无法获取用户目录".into());
    };
    let dir = root.join(id);
    if !dir.exists() {
        return Err("技能不存在".into());
    }
    fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(())
}

// ═══════════════════════════════════════════════════════════════
// 统一接口（catalog + 用户）
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize)]
pub struct SkillMeta {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source: String,
    /// 是否已拥有可用（预装 / 已安装 / 用户自建）
    pub installed: bool,
    /// 是否可删除（物理存在于用户目录，可卸载 / 删除）
    pub removable: bool,
}

/// 查找 skill（优先用户目录副本，再 catalog），返回元信息 + system_prompt
pub fn find(id: &str) -> Option<(SkillMeta, String)> {
    // 先查用户目录（允许覆盖同名 catalog skill）
    for user in scan_user_skills() {
        if user.id == id {
            return Some((
                SkillMeta {
                    id: user.id,
                    name: user.name,
                    description: user.description,
                    source: user.source,
                    installed: true,
                    removable: true,
                },
                user.system_prompt,
            ));
        }
    }
    // 再查 catalog
    find_catalog(id).map(|c| {
        (
            SkillMeta {
                id: c.id.into(),
                name: c.name.into(),
                description: c.description.into(),
                source: c.source.into(),
                installed: c.preinstalled,
                removable: false,
            },
            c.system_prompt.to_string(),
        )
    })
}

/// 检测用户消息是否包含创建 skill 的意图
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

/// 检测是否是"需要浏览器 / 网页自动化"的任务
pub fn detect_browser_intent(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();
    let triggers = [
        // URL / 英文
        "http://", "https://", "www.", "browser", "scrape", "scraping", "crawl",
        "playwright", "selenium", "puppeteer", "captcha", "cloudflare",
        // 中文
        "网页", "网站", "浏览器", "打开链接", "打开网址", "抓取", "爬取", "爬虫",
        "登录网", "网页截图", "网页自动化", "填表单", "网上下单", "自动化操作网页",
    ];
    triggers.iter().any(|t| lower.contains(t))
}

/// 检测是否是「做 PPT / 演示文稿」的任务。命中即自动激活 pptx 技能，
/// 不再要求用户先去技能中心安装 / 在对话框点选 —— 这是「无法产出 PPT」的首要原因。
pub fn detect_pptx_intent(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();
    let triggers = [
        // 英文
        "ppt", "pptx", "powerpoint", "slide deck", "slides", "keynote", "presentation",
        // 中文
        "幻灯片", "演示文稿", "演示文档", "做个演示", "做一个演示", "做份演示",
        "汇报材料", "路演", "宣讲", "述职", "答辩",
    ];
    triggers.iter().any(|t| lower.contains(t))
}

/// 检测是否是「生成图片 / 文生图 / AI 绘画」的任务（仅针对写实照片、AI 绘画类**位图**，
/// 不含图表 / 流程图 / SVG —— 那些可由代码生成，不受供应商生图能力限制）。
/// 命中即自动激活 image-gen 技能，让它先把「当前供应商能不能真的生图」讲清楚。
pub fn detect_image_intent(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();
    let triggers = [
        // 英文
        "generate an image", "generate image", "create an image", "make an image",
        "draw me", "text-to-image", "an illustration", "a poster", "ai art",
        // 中文 · 动词类
        "生图", "生成图片", "生成图像", "生成一张图", "生成一幅", "文生图", "ai 作图",
        "ai作图", "ai 画", "ai画", "画一张", "画一幅", "画个", "画张", "画幅",
        "帮我画", "给我画", "做张图", "做一张图", "做个图", "来张图", "出张图",
        // 中文 · 名词类（强烈暗示位图绘制）
        "配图", "海报", "插画", "插图", "封面图", "宣传图", "壁纸", "头像图",
    ];
    triggers.iter().any(|t| lower.contains(t))
}

/// 检测是否是「请教毛主席」的任务（原对话框开关，现改为技能 + 意图自动激活：
/// 消息里写「请教毛主席」等说法即注入毛选式分析指令，无需任何按钮）。
pub fn detect_mao_consult_intent(prompt: &str) -> bool {
    let triggers = [
        "请教毛主席",
        "请教一下毛主席",
        "问问毛主席",
        "问一下毛主席",
        "毛主席怎么看",
        "毛主席会怎么",
        "以毛主席的视角",
        "用毛主席的视角",
        "从毛主席的视角",
        "用毛选分析",
        "毛选式分析",
    ];
    triggers.iter().any(|t| prompt.contains(t))
}

/// 按任务意图自动激活的 skill（不依赖用户在对话框点选）。可返回多个。
/// 创建技能意图 → skill-creator；网页/浏览器自动化 → cloak-browser；
/// 做 PPT → pptx；生成图片 → image-gen；请教毛主席 → consult-mao。
pub fn auto_skills_for_intent(prompt: &str) -> Vec<(SkillMeta, String)> {
    let mut out = Vec::new();
    if detect_skill_creation_intent(prompt) {
        if let Some(s) = find("skill-creator") {
            out.push(s);
        }
    }
    if detect_mao_consult_intent(prompt) {
        // 只在已安装时注入(skill 随「毛主席」名人资料包一起装到用户技能目录;
        // 没装资料包就注入指令只会让模型对着空目录瞎找)
        if let Some(s) = find("consult-mao") {
            if s.0.installed {
                out.push(s);
            }
        }
    }
    if detect_browser_intent(prompt) {
        if let Some(s) = find("cloak-browser") {
            out.push(s);
        }
    }
    if detect_pptx_intent(prompt) {
        if let Some(s) = find("pptx") {
            out.push(s);
        }
    }
    if detect_image_intent(prompt) {
        if let Some(s) = find("image-gen") {
            out.push(s);
        }
    }
    out
}

// ═══════════════════════════════════════════════════════════════
// Tauri Commands
// ═══════════════════════════════════════════════════════════════

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn list_skills() -> Vec<SkillMeta> {
    let user = scan_user_skills();
    let user_ids: HashSet<String> = user.iter().map(|s| s.id.clone()).collect();

    let cat = catalog();
    let cat_ids: HashSet<&str> = cat.iter().map(|c| c.id).collect();

    let mut list = Vec::new();

    // 1. 目录技能（市场 + 预装）
    for c in &cat {
        let in_user_dir = user_ids.contains(c.id);
        list.push(SkillMeta {
            id: c.id.into(),
            name: c.name.into(),
            description: c.description.into(),
            source: c.source.into(),
            installed: c.preinstalled || in_user_dir,
            removable: in_user_dir,
        });
    }

    // 2. 纯用户自建技能（不在目录里的）
    for u in &user {
        if !cat_ids.contains(u.id.as_str()) {
            list.push(SkillMeta {
                id: u.id.clone(),
                name: u.name.clone(),
                description: u.description.clone(),
                source: u.source.clone(),
                installed: true,
                removable: true,
            });
        }
    }

    list
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn get_skill(id: String) -> Result<SkillMeta, String> {
    find(&id)
        .map(|(meta, _)| meta)
        .ok_or_else(|| format!("Skill '{}' 不存在", id))
}

#[derive(Debug, Deserialize)]
pub struct CreateSkillArgs {
    pub id: String,
    pub name: String,
    pub description: String,
    pub system_prompt: String,
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn create_skill(args: CreateSkillArgs) -> Result<(), String> {
    // 校验 id: 只允许小写字母、数字、-、_
    if !args
        .id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err("Skill ID 只能包含小写字母、数字、-、_".into());
    }
    write_skill_file(
        &args.id,
        &args.name,
        &args.description,
        "user",
        "user",
        &args.system_prompt,
    )
}

/// 从市场安装一个目录技能：复制模板到用户目录，保留原始 source。
/// 安装即拥有，立即出现在技能中心（前端负责自动激活）。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn install_skill(id: String) -> Result<(), String> {
    // 多文件打包技能走专用安装路径（clone 整包 + 叠加 Polaris 助手 + 拼 skill.md）。
    if id == WVP_ID {
        return install_web_video_presentation();
    }
    let c = find_catalog(&id).ok_or_else(|| format!("市场中没有技能 '{}'", id))?;
    write_skill_file(
        c.id,
        c.name,
        c.description,
        c.source,
        "registry",
        c.system_prompt,
    )
}

/// 递归复制目录树。
fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            fs::copy(&from, &to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// 去掉 markdown 开头的 YAML frontmatter（`---\n…\n---\n`），返回正文。
fn strip_frontmatter(s: &str) -> String {
    let s = s.trim_start_matches('\u{feff}');
    if let Some(rest) = s.strip_prefix("---") {
        if let Some(idx) = rest.find("\n---") {
            // 跳过 "\n---" 后到该行结尾
            let after = &rest[idx + 4..];
            let after = after.strip_prefix('\n').unwrap_or(after);
            return after.trim_start_matches('\r').trim_start().to_string();
        }
    }
    s.to_string()
}

/// 安装「网页演示视频」打包技能：clone 原仓库整目录 → 叠加 Polaris 助手脚本
/// → 拼 skill.md（Polaris 增强说明 + 原 SKILL 正文）→ 跑一次依赖自检。
fn install_web_video_presentation() -> Result<(), String> {
    let Some(root) = skills_dir() else {
        return Err("无法获取用户目录".into());
    };
    let dest = root.join(WVP_ID);

    // 1. clone 到临时目录
    let tmp = make_temp_dir()?;
    let repo = tmp.join("repo");
    let repo_s = repo.to_string_lossy();
    let clone_res = run_cmd(
        "git",
        &["clone", "--depth", "1", WVP_REPO, repo_s.as_ref()],
    );
    if let Err(e) = clone_res {
        let _ = fs::remove_dir_all(&tmp);
        return Err(format!("下载技能包失败（需要 git + 联网）：{}", e));
    }
    let src = repo.join("skills").join("web-video-presentation");
    if !src.exists() {
        let _ = fs::remove_dir_all(&tmp);
        return Err("仓库中未找到 skills/web-video-presentation".into());
    }

    // 2. 全新覆盖目标目录并复制整包
    if dest.exists() {
        fs::remove_dir_all(&dest).map_err(|e| e.to_string())?;
    }
    let copy_res = copy_dir_all(&src, &dest);
    let _ = fs::remove_dir_all(&tmp);
    copy_res?;

    // 3. 叠加 Polaris 助手脚本（编译期内嵌）
    let pol = dest.join("polaris");
    fs::create_dir_all(&pol).map_err(|e| e.to_string())?;
    fs::write(pol.join("minimax-tts.mjs"), WVP_MINIMAX_TTS).map_err(|e| e.to_string())?;
    fs::write(pol.join("scaffold.mjs"), WVP_SCAFFOLD).map_err(|e| e.to_string())?;
    fs::write(pol.join("bootstrap.mjs"), WVP_BOOTSTRAP).map_err(|e| e.to_string())?;

    // 4. 拼 skill.md = frontmatter + Polaris 增强说明(替换 PKG 路径) + 原 SKILL 正文
    let orig = fs::read_to_string(dest.join("SKILL.md")).map_err(|e| e.to_string())?;
    let body = strip_frontmatter(&orig);
    let pkg_str = dest.to_string_lossy().to_string();
    let addendum = WVP_ADDENDUM.replace("__PKG_DIR__", &pkg_str);
    let fm = format!(
        "---\nid: {}\nname: 网页演示视频制作（ConardLi·Polaris集成）\ndescription: 把文稿做成16:9可点击翻页的网页演示再录屏成片，配音自动调用 Polaris 内置 MiniMax。完整脚手架+23主题+音频流水线，Windows 走 Node 版一键跑通。\nsource: third-party\nauthor: ConardLi/Polaris\ncreated_at: {}\n---\n\n",
        WVP_ID,
        now_secs()
    );
    let skill = format!("{}{}\n\n{}", fm, addendum, body);
    fs::write(dest.join("skill.md"), skill).map_err(|e| e.to_string())?;

    // 5. 依赖自检（best-effort，不阻断安装）
    let _ = Command::new("node").arg(pol.join("bootstrap.mjs")).output();

    Ok(())
}

/// 启动时确保「课件视频工坊」技能在 ~/Polaris/skills 落盘（多文件，含可执行脚本）。
///
/// 这是支撑「生成课件类视频」UI 的基础设施技能，所以是「确保存在」而非「尊重删除」：
/// - 目录缺失（含被用户删除）→ 重新落盘
/// - 已落盘但版本旧（`.polaris_version` < `PVS_VERSION`）→ 覆盖更新（让脚本修复随更新下发）
/// - 已是最新 → 跳过
///
/// best-effort：任何失败都只是让该 UI 功能暂不可用，不应阻断 App 启动。
pub fn seed_video_studio_skill() {
    let Some(root) = skills_dir() else {
        return;
    };
    let dest = root.join(PVS_ID);
    let ver_file = dest.join(".polaris_version");
    let stored = fs::read_to_string(&ver_file).unwrap_or_default();
    let present = dest.join("skill.md").exists();
    if present && stored.trim() == PVS_VERSION {
        return; // 已是最新，无需重写
    }
    if write_video_studio_files(&dest).is_ok() {
        let _ = fs::write(&ver_file, PVS_VERSION);
    }

    // 顺带刷新已安装的 web-video-presentation 里的 Polaris 助手 minimax-tts.mjs，
    // 让「多语言配音」(language_boost) 的引擎修复随同一次版本更新下发——
    // 不动 ConardLi 原包文件，只覆盖我们自己叠加的助手脚本，且仅在它已存在时。
    let wvp_tts = root.join(WVP_ID).join("polaris").join("minimax-tts.mjs");
    if wvp_tts.exists() {
        let _ = fs::write(&wvp_tts, WVP_MINIMAX_TTS);
    }
}

/// 启动时确保「演示工坊」技能在 ~/Polaris/skills 落盘（多文件，含资源 + 导出脚本）。
///
/// 与 `seed_video_studio_skill` 同策略：目录缺失 / 版本旧（`.polaris_version` < `DECK_VERSION`）
/// 就（重）写；已是最新则跳过。best-effort，失败只让该 UI 暂不可用，不阻断启动。
pub fn seed_deck_studio_skill() {
    let Some(root) = skills_dir() else {
        return;
    };
    let dest = root.join(DECK_ID);
    let ver_file = dest.join(".polaris_version");
    let stored = fs::read_to_string(&ver_file).unwrap_or_default();
    let present = dest.join("skill.md").exists();
    if present && stored.trim() == DECK_VERSION {
        return;
    }
    if write_deck_studio_files(&dest).is_ok() {
        let _ = fs::write(&ver_file, DECK_VERSION);
    }
}

/// 把内嵌的「演示工坊」全部文件写到目标目录（建好子目录树）。
/// 技能正文写成小写 `skill.md`，与扫描约定一致。
fn write_deck_studio_files(dest: &Path) -> Result<(), String> {
    let assets = dest.join("assets");
    let templates = dest.join("templates");
    let scripts = dest.join("scripts");
    fs::create_dir_all(&assets).map_err(|e| e.to_string())?;
    fs::create_dir_all(&templates).map_err(|e| e.to_string())?;
    fs::create_dir_all(&scripts).map_err(|e| e.to_string())?;
    fs::write(dest.join("skill.md"), DECK_SKILL_MD).map_err(|e| e.to_string())?;
    fs::write(dest.join("LICENSE"), DECK_LICENSE).map_err(|e| e.to_string())?;
    fs::write(assets.join("base.css"), DECK_BASE_CSS).map_err(|e| e.to_string())?;
    fs::write(assets.join("themes.css"), DECK_THEMES_CSS).map_err(|e| e.to_string())?;
    fs::write(assets.join("runtime.js"), DECK_RUNTIME_JS).map_err(|e| e.to_string())?;
    fs::write(templates.join("deck.html"), DECK_TEMPLATE).map_err(|e| e.to_string())?;
    fs::write(scripts.join("install-deps.mjs"), DECK_INSTALL_DEPS).map_err(|e| e.to_string())?;
    fs::write(scripts.join("export-pptx.mjs"), DECK_EXPORT_PPTX).map_err(|e| e.to_string())?;
    Ok(())
}

/// 启动时确保「网站生成」技能在 ~/Polaris/skills 落盘。策略同上（版本号比对覆盖）。
pub fn seed_web_studio_skill() {
    let Some(root) = skills_dir() else {
        return;
    };
    let dest = root.join(WEB_ID);
    let ver_file = dest.join(".polaris_version");
    let stored = fs::read_to_string(&ver_file).unwrap_or_default();
    let present = dest.join("skill.md").exists();
    if present && stored.trim() == WEB_VERSION {
        return;
    }
    if write_web_studio_files(&dest).is_ok() {
        let _ = fs::write(&ver_file, WEB_VERSION);
    }
}

/// 把内嵌的「网站生成」全部文件写到目标目录。themes.css 复用 deck-studio 的同一份内容。
fn write_web_studio_files(dest: &Path) -> Result<(), String> {
    let assets = dest.join("assets");
    let templates = dest.join("templates");
    fs::create_dir_all(&assets).map_err(|e| e.to_string())?;
    fs::create_dir_all(&templates).map_err(|e| e.to_string())?;
    fs::write(dest.join("skill.md"), WEB_SKILL_MD).map_err(|e| e.to_string())?;
    fs::write(dest.join("LICENSE"), WEB_LICENSE).map_err(|e| e.to_string())?;
    fs::write(assets.join("site.css"), WEB_SITE_CSS).map_err(|e| e.to_string())?;
    fs::write(assets.join("themes.css"), DECK_THEMES_CSS).map_err(|e| e.to_string())?;
    fs::write(assets.join("runtime.js"), WEB_RUNTIME_JS).map_err(|e| e.to_string())?;
    fs::write(templates.join("site.html"), WEB_TEMPLATE).map_err(|e| e.to_string())?;
    Ok(())
}

/// 把内嵌的「课件视频工坊」全部文件写到目标目录（建好子目录树）。
/// 技能正文写成小写 `skill.md` —— 与 `scan_user_skills` / `write_skill_file` 的约定一致，
/// 避免大小写敏感文件系统（Linux/macOS 构建）下扫描不到。
fn write_video_studio_files(dest: &Path) -> Result<(), String> {
    let scripts = dest.join("scripts");
    let pipeline = scripts.join("pipeline");
    let refs = dest.join("references");
    fs::create_dir_all(&pipeline).map_err(|e| e.to_string())?;
    fs::create_dir_all(&refs).map_err(|e| e.to_string())?;
    fs::write(dest.join("skill.md"), PVS_SKILL_MD).map_err(|e| e.to_string())?;
    fs::write(dest.join("manifest.json"), PVS_MANIFEST).map_err(|e| e.to_string())?;
    fs::write(scripts.join("install-deps.mjs"), PVS_INSTALL_DEPS).map_err(|e| e.to_string())?;
    fs::write(scripts.join("run.mjs"), PVS_RUN).map_err(|e| e.to_string())?;
    fs::write(pipeline.join("03-record.mjs"), PVS_RECORD).map_err(|e| e.to_string())?;
    fs::write(refs.join("WORKFLOW.md"), PVS_WORKFLOW).map_err(|e| e.to_string())?;
    Ok(())
}

/// 老用户迁移：早期版本首启会自动播种毛主席资料库、且 consult-mao 的前身(对话框开关 /
/// 预装技能)开箱即用；改版后 skill 随「毛主席」名人资料包安装。已被播种过资料
/// (`<KB>/raw/毛主席` 在盘上)但技能目录里没有 consult-mao 的老用户，启动时补装一次，
/// 避免升级后「请教毛主席」失效。资料不在(从未播种/已移除)则不动。best-effort。
pub fn migrate_consult_mao_for_seeded_kb() {
    let kb_raw_mao = std::path::PathBuf::from(crate::kb::kb_root())
        .join("raw")
        .join("毛主席");
    if !kb_raw_mao.exists() {
        return;
    }
    let Some(root) = skills_dir() else {
        return;
    };
    if root.join("consult-mao").join("skill.md").exists() {
        return;
    }
    let _ = install_skill("consult-mao".to_string());
}

/// 启动时确保「壹伴排版优化」技能在 ~/Polaris/skills 落盘（多文件，含 wechat_yiban.py 可执行脚本）。
///
/// 与 deck/video studio 同策略：目录缺失 / 版本旧（`.polaris_version` < `WECHAT_TS_VERSION`）就（重）写；
/// 已是最新则跳过。脚本必须真落到磁盘，spawn 的 claude agent 才能 `python …/wechat_yiban.py` 跑它。
/// best-effort：失败只让「壹伴直送草稿」暂不可用，不阻断 App 启动。
pub fn seed_wechat_typesetter_skill() {
    let Some(root) = skills_dir() else {
        return;
    };
    let dest = root.join(WECHAT_TS_ID);
    let ver_file = dest.join(".polaris_version");
    let stored = fs::read_to_string(&ver_file).unwrap_or_default();
    let present = dest.join("skill.md").exists();
    if present && stored.trim() == WECHAT_TS_VERSION {
        return;
    }
    if write_wechat_typesetter_files(&dest).is_ok() {
        let _ = fs::write(&ver_file, WECHAT_TS_VERSION);
    }
}

/// 把内嵌的「壹伴排版优化」文件写到目标目录。技能正文写成小写 `skill.md`，与扫描约定一致。
fn write_wechat_typesetter_files(dest: &Path) -> Result<(), String> {
    let scripts = dest.join("scripts");
    fs::create_dir_all(&scripts).map_err(|e| e.to_string())?;
    fs::write(dest.join("skill.md"), WECHAT_TS_SKILL_MD).map_err(|e| e.to_string())?;
    fs::write(scripts.join("wechat_yiban.py"), WECHAT_TS_YIBAN_PY).map_err(|e| e.to_string())?;
    Ok(())
}

// ═══════════════════════════════════════════════════════════════
// 外部导入 / 下载（不限来源，鼓励从外面拿）
//   本地：.md 文件 / .zip 压缩包 / 技能目录
//   远程：http(s) 的 .md 或 .zip / git 仓库 URL（可装整套技能合集）
// ═══════════════════════════════════════════════════════════════

/// 把任意来源的 skill 导入用户目录，返回导入成功的 skill id 列表（供前端自动激活）。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn import_skill(source: String) -> Result<Vec<String>, String> {
    let src = source.trim();
    if src.is_empty() {
        return Err("来源为空".into());
    }

    let is_remote = src.starts_with("http://")
        || src.starts_with("https://")
        || src.starts_with("git@")
        || src.ends_with(".git");

    if is_remote {
        import_from_remote(src)
    } else {
        import_from_local(Path::new(src))
    }
}

fn import_from_remote(src: &str) -> Result<Vec<String>, String> {
    let tmp = make_temp_dir()?;
    let lower = src.to_lowercase();

    let result = if lower.ends_with(".md") {
        let md = tmp.join("skill.md");
        download(src, &md)?;
        import_one_md(&md, "imported").map(|id| vec![id])
    } else if lower.ends_with(".zip") {
        let zip = tmp.join("download.zip");
        download(src, &zip)?;
        let out = tmp.join("unzipped");
        fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        unzip(&zip, &out)?;
        import_from_dir(&out)
    } else {
        // .git 结尾、git@、或 github/gitlab 等仓库 URL → clone 后扫描全部技能
        let dest = tmp.join("repo");
        let dest_s = dest.to_string_lossy();
        run_cmd("git", &["clone", "--depth", "1", src, dest_s.as_ref()])?;
        import_from_dir(&dest)
    };

    let _ = fs::remove_dir_all(&tmp);
    result
}

fn import_from_local(path: &Path) -> Result<Vec<String>, String> {
    if !path.exists() {
        return Err(format!("路径不存在: {}", path.display()));
    }
    if path.is_dir() {
        return import_from_dir(path);
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "md" => import_one_md(path, "imported").map(|id| vec![id]),
        "zip" => {
            let tmp = make_temp_dir()?;
            let out = tmp.join("unzipped");
            fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            unzip(path, &out)?;
            let r = import_from_dir(&out);
            let _ = fs::remove_dir_all(&tmp);
            r
        }
        other => Err(format!("不支持的文件类型: .{}", other)),
    }
}

/// 递归扫描目录里所有 SKILL.md / skill.md，逐个导入（支持技能合集）
fn import_from_dir(dir: &Path) -> Result<Vec<String>, String> {
    let mut ids = Vec::new();
    for entry in walkdir::WalkDir::new(dir).into_iter().flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if fname.eq_ignore_ascii_case("skill.md") {
            if let Ok(id) = import_one_md(p, "imported") {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
    }
    if ids.is_empty() {
        return Err("未在来源中找到任何 SKILL.md / skill.md".into());
    }
    Ok(ids)
}

/// 导入单个 md：有 frontmatter 按字段解析，无 frontmatter 则整篇即正文。
/// 规范化后写到 ~/Polaris/skills/<id>/skill.md。
fn import_one_md(md: &Path, default_source: &str) -> Result<String, String> {
    let raw = fs::read_to_string(md).map_err(|e| e.to_string())?;

    let (id_raw, name_raw, description, src) = if let Ok(s) = parse_skill_file(md) {
        (s.id, s.name, s.description, s.source)
    } else {
        // 无 frontmatter：用所在目录名（退而求其次文件名）当 id，正文 = 全文
        let base = md
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .filter(|s| !["unzipped", "repo", "skills", ""].contains(s))
            .map(|s| s.to_string())
            .or_else(|| md.file_stem().and_then(|n| n.to_str()).map(|s| s.to_string()))
            .unwrap_or_else(|| "imported-skill".to_string());
        (base.clone(), base, String::new(), "user".to_string())
    };

    // 正文：parse 成功用其 system_prompt，否则用去掉 frontmatter 的全文
    let body = match parse_skill_file(md) {
        Ok(s) => s.system_prompt,
        Err(_) => raw.trim().to_string(),
    };

    let id = {
        let cleaned: String = id_raw
            .to_lowercase()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        let cleaned = cleaned.trim_matches('-').to_string();
        if cleaned.is_empty() {
            "imported-skill".to_string()
        } else {
            cleaned
        }
    };
    let name = if name_raw.trim().is_empty() {
        id.clone()
    } else {
        name_raw
    };
    let source = if src == "user" {
        default_source.to_string()
    } else {
        src
    };

    write_skill_file(&id, &name, &description, &source, "imported", &body)?;
    Ok(id)
}

// ── 外部工具封装（用系统自带 git / curl / tar，免新增 Rust 依赖） ──

fn make_temp_dir() -> Result<PathBuf, String> {
    let base = std::env::temp_dir().join(format!("polaris-skill-import-{}", now_secs()));
    fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    Ok(base)
}

fn run_cmd(cmd: &str, args: &[&str]) -> Result<(), String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("无法执行 {}：{}（请确认系统已安装 {}）", cmd, e, cmd))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("{} 执行失败：{}", cmd, err.trim()));
    }
    Ok(())
}

fn download(url: &str, dest: &Path) -> Result<(), String> {
    let dest_s = dest.to_string_lossy();
    run_cmd("curl", &["-L", "--fail", "-s", "-o", dest_s.as_ref(), url])
}

fn unzip(zip: &Path, dest: &Path) -> Result<(), String> {
    // Win11 / macOS / Linux 自带 bsdtar 可解 .zip
    let zip_s = zip.to_string_lossy();
    let dest_s = dest.to_string_lossy();
    run_cmd("tar", &["-xf", zip_s.as_ref(), "-C", dest_s.as_ref()])
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn delete_skill(id: String) -> Result<(), String> {
    // 安全闸: id 直接拼进 remove_dir_all 的路径, 必须挡掉 `..` / 路径分隔符 / 盘符,
    // 否则前端(或被注入的 webview 脚本)能传 `..\..\Docs` 或绝对路径删任意目录。
    if id.is_empty()
        || id.contains('/')
        || id.contains('\\')
        || id.contains("..")
        || id.contains(':')
    {
        return Err("非法技能 ID".into());
    }
    let Some(root) = skills_dir() else {
        return Err("无法获取用户目录".into());
    };
    // 物理存在于用户目录 → 直接移除（用户自建 / 已安装市场技能都走这里）
    if root.join(&id).exists() {
        return remove_user_skill(&id);
    }
    // 不在用户目录：可能是预装技能（不可删）或根本不存在
    if find_catalog(&id).map(|c| c.preinstalled).unwrap_or(false) {
        return Err("预装技能不可删除".into());
    }
    Err("技能不存在".into())
}
