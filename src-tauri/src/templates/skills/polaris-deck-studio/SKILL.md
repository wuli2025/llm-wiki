---
id: polaris-deck-studio
name: Polaris 演示工坊（PPT / 网页幻灯片）
description: 把文案或文档做成有设计感的幻灯片。一套引擎两种交付：自包含可翻页的网页 deck(.html)，或像素级还原主题的 .pptx。内置 17 套主题(借力 open-design)，键盘翻页/演讲者备注/打印 PDF。
source: official
author: Polaris
created_at: 0
---

# Polaris 演示工坊

> 输入一段文案或一份文档 → 选一套主题 → 输出一份**好看**的演示。
> 同一套 HTML 引擎，两种交付物：
> - **网页幻灯片**：一个自包含 `.html`，可翻页、可全屏、可打印为 PDF、可直接分享。
> - **PPT**：把网页 deck 逐页截图，做成像素级还原主题的 `.pptx`（视觉 = 网页，文字不可编辑）。

技能资源目录（已随 App 落盘）：`~/Polaris/skills/polaris-deck-studio/`
```
assets/base.css      幻灯片引擎 + 设计 token（来自 open-design，MIT）
assets/themes.css    17 套主题（[data-theme] 属性选择器）
assets/runtime.js    翻页 / 主题切换(T) / 概览(O) / 全屏(F) / 打印(P) / #/N 深链
templates/deck.html  起始模板（含 5 页示例 + 动画用法）
scripts/install-deps.mjs   装 playwright + pptxgenjs（仅 PPT 导出需要）
scripts/export-pptx.mjs    deck.html → .pptx（逐页截图，整版图嵌入）
```

---

## 调用方式（前端会传一段「制作配置」）

「演示工坊」面板会在提示词里给出：
- **输出模式**：`html`（网页幻灯片）或 `pptx`（PPT）
- **主题 id**：见下表（或 `auto` = 你自行挑最合适的）
- **页数上限 / 画幅比例 / 信息密度**
- **正文**：直接粘贴的文案，或上传文件的绝对路径（先 `Read` 它们）
- **产物目录**：最终文件要保存到这里，并在回答末尾列出绝对路径

没有上述配置时（用户在普通对话里直接说「做个 PPT/网页演示」），用合理默认：主题走 **`auto`（高级感）**、16:9、≤12 页、中等密度、输出 `html`。

### ★ 主题 = `auto`（即 UI 的「AI 自由发挥」）= 默认高级感
`auto` **不是**「随便挑一个」，而是**默认做出一眼高级、有感染力的观感**：
- **优先深色 / 质感主题**，**不要默认白底**。首选：`aurora`（极光渐变辉光）、`glassmorphism`（毛玻璃）、`pitch-deck-vc`（融资路演）、`vaporwave`（蒸汽波）、`cyberpunk-neon`（赛博霓虹）、`tokyo-night`（东京夜）。
- 配方：**深底 + 渐变强调色（`.gradient-text` 用在关键词上）+ 超大标题（封面 `.h1` 可到 110–160px）+ 克制留白 + 大数字金句页**。少字、字大、一页一事。
- 仅当内容**明显属于**学术 / 公文 / 财报 / 法务等需要素白严肃的场景，才退回浅色主题（如 `academic-paper`、`corporate-clean`、`minimal-white`）。
- 用户填了「自定义风格补充」时以其为准（如「黑金高级」→ 在深色主题上叠加金色强调）。

---

## 主题（36 套，data-theme 取值）

| 分组 | id |
|---|---|
| 高级感首选（深色/质感） | `aurora` `glassmorphism` `pitch-deck-vc` `vaporwave` `cyberpunk-neon` `tokyo-night` |
| 深色 | `dracula` `nord` `terminal-green` `blueprint` `catppuccin-mocha` `gruvbox-dark` `retro-tv` `rose-pine` |
| 浅色 | `minimal-white` `editorial-serif` `swiss-grid` `magazine-bold` `japanese-minimal` `xiaohongshu-white` `academic-paper` `corporate-clean` `soft-pastel` `arctic-cool` `bauhaus` `catppuccin-latte` `engineering-whiteprint` `midcentury` `news-broadcast` `sharp-mono` `solarized-light` `sunset-warm` |
| 特色 | `neo-brutalism` `memphis-pop` `rainbow-gradient` `y2k-chrome` |

应用主题 = 在 `<html data-theme="aurora">`。运行时按 `T` 可循环切换预览。

---

## 制作步骤

### 1. 规划内容 → 分页
把正文拆成「一页一个信息点」的结构。好演示的铁律：**每页只讲一件事，字少、字大、留白多**。封面 / 要点列表 / 大数字金句 / 两栏对比 / 结尾，是最常用的页型。演讲者要说但观众不该看到的内容，放进 `<div class="notes">…</div>`（默认隐藏，按 `S` 在演讲者视图看）。

### 2. 用引擎写 deck.html
照 `templates/deck.html` 的骨架写。核心约定（全在 `base.css` 里）：
- 容器 `<div class="deck">`，每页一个 `<section class="slide" data-title="...">`
- 版式原语：`.grid .g2/.g3/.g4`、`.row`、`.card`/`.card-accent`/`.card-hover`、`.pill`、`.lede`、`.kicker`、`.gradient-text`、`.center`
- 标题：`.h1`/`.h2`/`h1.title`/`h2.title`/`.h3`
- 动画：元素加 `class="anim-fade-up"`（或 `anim-fade/anim-zoom/anim-slide-left/anim-slide-right`）；列表容器加 `anim-stagger-list`，子项设 `style="--i:0/1/2…"` 做错峰入场
- 页脚/进度/概览：`<div class="deck-footer"><span class="slide-number"></span></div>`、`<div class="progress-bar"><span></span></div>`、`<div class="overview"></div>`

### 3. ★ 做成自包含单文件（两种模式都这么做）
**把 `assets/base.css` 与 `assets/themes.css` 的内容内联进 `<style>`，把 `assets/runtime.js` 内联进 `<script>`**，删掉对 `../assets/*` 的外链。这样产出的 `deck.html` 是**单文件**，可独立分享、可被截图导出、不依赖技能目录。读取这三个文件：
```bash
cat ~/Polaris/skills/polaris-deck-studio/assets/base.css
cat ~/Polaris/skills/polaris-deck-studio/assets/themes.css
cat ~/Polaris/skills/polaris-deck-studio/assets/runtime.js
```
把 deck.html 存到**产物目录**（文件名如 `演示-<主题>.html`）。

### 4a. 模式 = html（网页幻灯片）
到此就完成了。在回答末尾给出 `deck.html` 的绝对路径，并说明：双击用浏览器打开；`←/→/空格` 翻页、`F` 全屏、`O` 概览、`T` 换主题、`P`/`Ctrl+P` 导出 PDF。

### 4b. 模式 = pptx（PPT）
先装一次导出依赖（仅首次需要、幂等）：
```bash
node ~/Polaris/skills/polaris-deck-studio/scripts/install-deps.mjs
```
再导出（用上一步那份自包含 deck.html）：
```bash
node ~/Polaris/skills/polaris-deck-studio/scripts/export-pptx.mjs \
  --deck="<产物目录>/演示-<主题>.html" \
  --out="<产物目录>/演示-<主题>.pptx" \
  --width=1920 --height=1080
```
脚本逐页截图（自动加 `?export=1` 关闭入场动画求干净静帧）→ 用 pptxgenjs 把每页整版图铺满一张 16:9 幻灯片 → 输出 `.pptx`。回答末尾给出 `.pptx`（和源 `.html`）的绝对路径。

---

## 兜底（依赖缺失也不能卡死）
- `npm`/`playwright` 装不上 → 改让用户用浏览器打开 deck.html 后 **`Ctrl+P` → 另存为 PDF**（`base.css` 已含 `@media print` 分页，每页一张）。
- 需要**可编辑文本**的 PPT（而非整版图）→ 改用 `pptx`（python-pptx）技能按内容重排为原生文本框，告知用户这会牺牲主题的精确视觉。
- 始终给出已经成功产出的那份文件的绝对路径，别让用户两手空空。

## 画幅
默认 16:9（导出用 1920×1080）。若用户要 4:3，截图用 `--width=1440 --height=1080`，并把 `export-pptx.mjs` 里 `defineLayout`/`addImage` 的 13.333×7.5 改为 10×7.5（脚本注释处）。
