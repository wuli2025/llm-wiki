# 板块⑫ 人格模块（Persona）PRD

> 状态：✅ 已实现（2026-06-02）— cargo check / vue-tsc / build 全绿
> 思想来源：WeSight（OpenClaw）的 preset agent + 右侧选人格 + 「每个项目=一个人格」。
> **不抄其代码**，用 Polaris 自研机制实现（复用既有 `claude_md` + `conv` 项目模型）。

## 0. 一句话定位

把现有隐式的「每个项目的 `CLAUDE.md` 就是它的人格」正式**产品化**成一个「人格」模块：
顶层导航出现「人格」入口（原「目录说明」改造而来并升格），用户能从**预设人格库**一键给项目套人格，
每个人格还能绑**各自的专属 wiki 知识库**，发消息时按分层注入。

## 1. 背景：Polaris 现状（已具备的地基）

| 既有能力 | 位置 | 说明 |
|---|---|---|
| 项目模型 | `conv.rs::Project{id,name,archived}` | 状态存 `~/Polaris/data/state.json` |
| 每项目一份 CLAUDE.md | `claude_md.rs` | `~/Polaris/projects/<id>/CLAUDE.md` |
| 注入链路 | `claude_md::render_for_project()` | 发消息前拼「KB 块 + 项目 CLAUDE.md 块」 |
| 编辑 UI | `ClaudeMdPanel.vue` | 已是「左列表（KB+项目）＋右编辑器」选择式 |
| 默认人格 | `conv.rs::seeded_mao` | 首启播种「毛主席」项目 + `templates/mao_persona_claude.md` |
| 知识库 | `kb.rs::kb_context_block()` | 注入结构化 wiki + 双链地图，模型自取 |

**结论**：「项目=人格」已经成立，本板块=**改名升格 + 预设库 + 选人格 UI + 人格↔KB 绑定**，不是从零造。

## 2. 人格分层注入模型（对齐 WeSight 的 L1–L6，落到 Polaris）

| 层 | 内容 | Polaris 落点 | 何时拼 |
|---|---|---|---|
| L1 全局身份 | 「你是 Polaris 北极星…」 | 新增 `templates/identity.md`（常量注入，可空） | 每次发送 |
| L2 人格 | 6 预设之一 / 毛主席 / 自定义 | 项目 `CLAUDE.md`（已存在） | 选了项目即生效 |
| L3 身份补充 | 预设里的 identity 段（并入 L2 文本） | 同 CLAUDE.md | 同上 |
| L4 团队角色 | 动态编排子代理角色 | 已由板块⑪ `dynamic-workflows` 注入 | 编排时 |
| **L5 当前时间** | 本地时间/时区/时间戳 | **新增** `chat::local_time_context()` | 每次发送 |
| **L6 记忆/知识库** | 该人格专属 wiki + 双链地图 | `kb::kb_context_block(scope)` **加 scope** | 每次发送 |

> 与现有 `render_for_project` 的关系：在其基础上**前置 L1+L5**、并把 L6 的 KB 注入从「全局 PolarisKB」改为「该人格绑定的 scope」。

## 3. 人格 ↔ 专属知识库绑定（本次新增的核心）

- `Project` 增加可选字段 `persona_id: Option<String>`（来自哪个预设，便于显示图标/更新）与 `kb_scope: Option<String>`（该人格的知识库范围）。
- `kb_scope` 语义（MVP）：**KB 根下的相对子目录**，例如毛主席 → `raw/毛主席`。为空 = 用全局 PolarisKB（向后兼容）。
- `kb::kb_context_block()` 增加可选 `scope` 参数：只把 scope 子树下的 wiki 页 + 该子树内的双链地图注入，其余不进上下文 → **不同人格看到不同知识库**。
- 知识库视图（板块②）后续可加「按人格筛选」；本板块只保证注入侧隔离。
- **务实取舍**：scope 过滤在内存索引层做前缀匹配，不动磁盘结构；用户仍可在「知识库」里手动管理各人格子目录。

## 4. 6 个预设人格（思想搬运 + 本地化改造）

来源：WeSight `presetAgents.ts` 的 6 段 systemPrompt（股票/内容创作/备课出卷/内容总结/医疗解读/萌宠管家）。
**改造规则（关键）**：把原文里的「使用 X skill」统一改写为 Polaris 的「沿双链查知识库 + 用既有 skill 中心能力」，
并补一段统一的「## 知识库使用原则」（Read/Glob/Grep 沿 `[[...]]` 自取、先引库内结论再 web-search、区分事实与推断）。

落地：`src-tauri/src/templates/personas/` 下 6 个 `.md` + 注册表 `persona_presets.rs`（id/name/icon/desc/kbScope 建议）。
毛主席沿用既有 `mao_persona_claude.md`，作为第 7 个「内置彩蛋人格」。

| id | 名称 | icon | 建议 kb_scope |
|---|---|---|---|
| stock-expert | 股票助手 | 📈 | raw/股票 |
| content-writer | 内容创作 | ✍️ | raw/创作 |
| lesson-planner | 备课出卷 | 📚 | raw/教学 |
| content-summarizer | 内容总结 | 📋 | （空=全局） |
| health-interpreter | 医疗健康解读 | 🏥 | raw/健康 |
| pet-care | 萌宠管家 | 🐾 | raw/萌宠 |
| mao（内置） | 毛主席 | ☭ | raw/毛主席 |

## 5. 导航改版（用户可见）

**前**：顶层 `对话/知识库/图谱/自动化/技能中心/更新`；更多 `目录说明/环境/MCP/设置`
**后**：
- 顶层：`对话/知识库/图谱/自动化/技能中心/`**`人格`**（「人格」取代原「更新」位）
- 更多：**`更新`**（降入更多）`/飞书/桌面宠物/环境/MCP/设置`（「目录说明」并入「人格」，不再单列）

## 6. UI 设计（PersonaWorkshop.vue，由 ClaudeMdPanel 改造）

- 左列表：项目（=人格实例）+「知识库行为指南」一项（原 KB CLAUDE.md，保留）。每项显示人格图标 + 启用徽章。
- 右侧分两区：
  - **预设人格画廊**（仿 WeSight 右侧选人格）：7 张卡片（6 预设+毛主席），点「应用到当前项目」→ 用预设文本填充该项目 CLAUDE.md（已有内容则二次确认覆盖）+ 写入建议 kb_scope。
  - **人格正文编辑器**：复用现有 textarea（编辑 CLAUDE.md），加「启用/还原/保存」。下方加 `kb_scope` 选择（下拉 KB 子目录）。
- 顶部文案从「CLAUDE.md·主上下文」改为「人格·项目的灵魂与专属知识库」。

## 7. 后端改动清单

- `conv.rs`：`Project` 加 `persona_id`/`kb_scope`（`#[serde(default)]` 向后兼容）；新增 `conv_set_project_persona(project_id, persona_id, kb_scope)`；`conv_create_project` 可选带 persona。
- `persona_presets.rs`（新）：`include_str!` 7 个模板 + `persona_list()` 命令（返回 id/name/icon/desc/body/kbScope）+ `persona_apply(project_id, persona_id)` 命令（写 CLAUDE.md + set scope）。
- `claude_md.rs`：`render_for_project` 前置 L1（identity）+ L5（local_time）；L6 改调 `kb::kb_context_block(scope)`，scope 取自该 project 的 `kb_scope`。
- `kb.rs`：`kb_context_block` 增加 `scope: Option<&str>` 前缀过滤（默认 None=全局，保持现状）。
- `lib.rs`：注册新命令。

## 8. 验收

- `cargo check --workspace`、`vue-tsc`、`npm run build` 全绿。
- 顶层出现「人格」，更多出现「更新」，旧「目录说明」消失但功能在「人格」里可达。
- 新建项目可选预设人格；应用预设后该项目 CLAUDE.md 被填充、对话注入生效。
- 毛主席项目注入只见毛主席资料库；切到别的空 scope 人格不串味（scope 过滤生效）。
- 既有用户 state.json 升级无损（新字段默认值）。

## 9. 板块边界（沿用 §16）

人格模块不新开 crate（体量小，务实），但严守：只调 `conv`/`kb`/`claude_md` 的公开 `pub fn`，注入仍走 `render_for_project` 单一入口。
