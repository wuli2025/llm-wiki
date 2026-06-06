# 板块 ② 维基知识库 · 增强 PRD(借鉴 llm_wiki 思想)

> 状态:✅ 后端已实现 + 前端已接线(本轮)
> 来源:研究参考项目 `llm_wiki`(GPL,仅学思想不抄代码)后,落地其第一、二梯队可借鉴点。
> 报告原件:`C:\Users\mi\Desktop\Polaris可借鉴思想报告.html`

## 0. 核心哲学(贯穿全部增强)

> **让 AI 只动嘴,代码动手。** AI 只产出「决策数据」(JSON:该补哪条链、哪两页重复),
> 所有对用户文件的改动交给确定性 Rust 代码执行。系统因此可预测、可回滚、对数据安全。

实现上:enrich / dedup 起的都是 **只读 claude**(`--allowedTools Read,Glob,Grep`,
物理上拿不到 Write/Edit),它无法改文件,只能输出 JSON;落地全在 `kb.rs`。

---

## 1. 上下文预算(治 32k 撞墙)· tier-1

**问题**:wiki/ 全文注入曾达 42k 字符,撞 Windows 命令行 32k 上限(报 206);即便走 stdin,
无节制注入也吃光模型上下文窗口、挤掉回话余量。

**做法**(`kb_context_block_scoped`,纯 Rust):按固定比例切预算,而非拍脑袋。
- 总预算 `KB_CTX_BUDGET = 24_000` 字符(远低于 32k,且给模型回话留足)。
- 导航页(index/_index)占 `55%`,地图清单占 `40%`,余量留给标题/说明。
- 单篇导航页正文上限 `30%`,防一个超大 `_index` 吃光预算。
- 超出即优雅截断,显式提示「其余请用 Read/Glob 自取」。

---

## 2. 安全路径护栏 · tier-2

**问题**:`kb_compile` 给 headless claude 开了写权限自由落盘 wiki 页。万一模型(或被注入)
给出 `C:\Windows\…` 绝对路径、`../../` 越界、或 Windows 保留名,可能写坏库外文件。

**做法**:`is_safe_wiki_relpath()` 纯函数,7 层校验(单测覆盖):
无控制字符 → 拒绝绝对路径/盘符/UNC → 规范化反斜杠 → 拒绝 `.`/`..` 段 →
拒绝 Windows 保留名(CON/NUL/COM1…)→ 段尾不得空格或点 → 必须落在 `wiki/` 下且为 `.md`。
用于 `kb_lint` 审计与 enrich/dedup 落盘前校验。

---

## 3. 增量入库缓存 · tier-2

**问题**:重复拖同一批资料,每次全量重转(PDF/docx 抽取很贵)。

**做法**(`IngestCache`,`.polaris_ingest_cache.json`):源文件内容指纹(std `DefaultHasher`,
不引入 sha2 依赖)未变 **且产物仍在磁盘** → 跳过转换、复用上次产物。
第二步的「存在性校验」是 llm_wiki 的关键洞察 —— 防「幽灵条目」(旧产物被删后缓存还指着它,
导致库里凭空少一篇)。`kb_upload_files` 整批共用一个缓存,结束统一落盘。

---

## 4. wiki 质量检查 `kb_lint` · tier-2(sweep 之眼)

纯规则扫一遍内存索引,把问题列清楚(作为后台巡检的「眼睛」):
- **死双链**:`[[X]]` 指向不存在页面。
- **缺 type**:内容页 frontmatter 漏 `type`。
- **孤儿页**:既不链别人也没人链它,未接入知识网。
- **不安全路径**:rel_path 不通过护栏。

返回 `KbLintReport { totalPages, deadLinks, missingType, orphans, unsafePaths, issues[] }`。
为支持 type 校验,`KbDoc` 新增 `doc_type` 字段(从 frontmatter `type` 解析)。

---

## 5. 自动补双链 `kb_enrich_links` · tier-1(旗舰)

**「AI 出决策、代码执行」的最佳示范。**
- 只读 claude 拿到「现有 wiki 标题清单」,浏览内容页,返回
  `[{page, term, target}]` —— 哪个 page 正文里哪个明文词 `term` 该链到哪个已有页 `target`。
- Rust 的 `apply_wikilink()` 纯函数执行替换(单测覆盖):
  - 只替 **首次出现**;
  - 跳过 frontmatter 区、已有 `[[…]]` 内部、行内代码 `` `..` `` 与围栏代码块;
  - `term==target` 写 `[[X]]`,否则写别名形式 `[[target|term]]`;
  - 落盘前过 `is_safe_wiki_relpath` + `target` 必须是现存标题。
- 进度走 `kb:enrich` 事件,完成回报实际应用条数。

---

## 6. 智能去重 `kb_dedup` · tier-2 + 页面合并锁定字段 tier-1

两段式(便宜的规则在前、贵的 AI 在后):
1. **规则粗筛**:按 `normalize_title`(小写+去标点空白)分组,取 size≥2 的疑似重复组。
   无碰撞直接早退(llm_wiki:0 重复 → 跳过)。
2. **AI 细判**:只读 claude 看路径/标题/摘要(必要时 Read 全文),返回
   `[{pages, duplicate, confidence, canonical, reason}]`。**保守:低置信不动。**
3. **Rust 执行合并**(`merge_duplicate_page`):
   - **锁定主页 frontmatter**(不碰 `type/title/created`)——改写会断双链、毁一次性时间戳;
   - 把重复页正文并入主页末尾「合并自」区(不丢知识);
   - **重写全库** `[[重复页标题]]` → `[[主页标题]]`(`rewrite_wikilink_target`,保留别名/锚点后缀);
   - 删重复页文件 + 清 `wiki/index.md` 里指向它的行。
- 进度走 `kb:dedup` 事件,完成回报合并页数。

---

## 7. 新增 Tauri 命令一览

| 命令 | 类型 | 说明 |
|------|------|------|
| `kb_lint` | 同步 | 返回 `KbLintReport` |
| `kb_enrich_links` | 异步 | 返回 runId,进度 `kb:enrich` |
| `kb_dedup` | 异步 | 返回 runId,进度 `kb:dedup` |

均已注册到 `lib.rs` invoke_handler。前端 `tauri.ts` 加 `kb.lint/enrichLinks/dedup` +
类型 `KbLintReport/KbMaintainEvent`;`stores/kb.ts` 加 `startMaintain/runLint`(复用构建进度 UI);
`WikiBrowse.vue` 新增「维护知识网」卡片(补双链 / 去重 / 质量检查三按钮 + lint 报告)。

## 8. 测试

`cargo test --lib kb::tests` —— 11 个单测全绿:安全路径(3)、补链(4)、去重纯函数(2)、
JSON 容错抽取(1)、上下文截断(1)。`npx vue-tsc --noEmit` 前端零报错。

## 9. 未采用 / 留待

- **向量/RRF 混合检索**(tier-3):Polaris 刻意走 Read/Glob/Grep 长上下文路线,暂不引入 embedding。
- **后台定时巡检 / 剪贴板入库 / 图片理解 / 本地 API server**(tier-3):看产品走向再定。
- dedup 的正文「智能合并」目前是安全的「并入末尾」而非语义融合;语义融合留待后续。
