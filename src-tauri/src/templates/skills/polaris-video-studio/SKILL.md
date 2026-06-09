---
id: polaris-video-studio
name: Polaris 视频工坊（课件/口播/演示一键成片）
description: 把文案、课件、口播稿一键做成 16:9 网页演示视频并输出 MP4。完整链路：文案→口播稿→网页开发→MiniMax 配音→逐帧对齐录屏→MP4。
source: official
author: Polaris
created_at: 0
---

# Polaris 视频工坊

> 把一段文案、课件或口播稿，一键做成带配音的 16:9 网页演示视频，输出 MP4。
>
> **核心差异**：传统 web-video-presentation 需要用户手动录屏；本工作流在 Phase 4 用
> Playwright 无头截图 + ffmpeg 逐帧对齐拼接，**零手动操作**，直接出片。

---

## 依赖清单（首次使用前必须安装）

| 工具 | 用途 | 检查命令 | 安装方式 |
|---|---|---|---|
| Node.js ≥ 18 | 脚手架 + 音频合成 + 录屏 | `node --version` | 已装（Polaris 内置 Node 环境） |
| npm ≥ 9 | 包管理 | `npm --version` | 随 Node 自带 |
| ffmpeg | 音频时长探测 + 视频合成 | `ffmpeg -version` | 自动检测，未装则引导下载 |
| Playwright | 无头截图 + 视频录制 | `npx playwright --version` | `npm install -g playwright` |
| MiniMax API Key | T2A 配音 | 环境变量 `MINIMAX_API_KEY` | Polaris 供应商坞自动提供 |

**一键安装依赖**：

```bash
node ~/Polaris/skills/polaris-video-studio/scripts/install-deps.mjs
```

此脚本会自动检测上述工具，缺什么就引导你装什么（Windows/macOS/Linux 均支持）。

---

## 可视化链路

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│   用户文案   │────▶│  口播稿 +   │────▶│  Vite+React │────▶│  MiniMax    │
│ (课件/文章) │     │   outline   │     │  网页演示   │     │   T2A 配音  │
└─────────────┘     └─────────────┘     └─────────────┘     └─────────────┘
       │                   │                   │                   │
       │            Phase 1 │            Phase 2 │            Phase 3 │
       │                   │                   │                   │
       ▼                   ▼                   ▼                   ▼
   粘贴/上传         AI 改写口播稿      脚手架 + 逐章开发    npm run synthesize
   （1 秒）          （AI 执行）         （AI 执行）         （自动，~2min）
                                                            │
                                                            ▼
                                                   ┌─────────────┐
                                                   │  13 段 mp3  │
                                                   │  逐帧对齐   │
                                                   └─────────────┘
                                                            │
                                                            ▼
                                                   ┌─────────────┐
                                                   │  Playwright │
                                                   │  无头截图   │
                                                   │  1920×1080  │
                                                   └─────────────┘
                                                            │
                                                            ▼
                                                   ┌─────────────┐
                                                   │   ffmpeg    │
                                                   │  拼接合成   │
                                                   │   MP4 输出  │
                                                   └─────────────┘
                                                            │
                                                            ▼
                                                   ┌─────────────┐
                                                   │  output.mp4 │
                                                   │  1920×1080  │
                                                   │  音画同步   │
                                                   └─────────────┘
                                                            │
                                                            ▼
                                                        用户收片
```

**链路特点**：
- **4 个 Phase**，其中 Phase 1-2 由 AI 执行（口播稿改写 + 网页开发），Phase 3-4 由脚本自动执行（配音 + 录屏合成）
- **零手动录屏**：Phase 4 用 Playwright 逐帧截图 + ffmpeg 按音频时长拼接，画面和声音精确对齐
- **一次出片**：用户只需提供文案，其余全自动化

---

## 一键执行（极速模式）

```bash
node ~/Polaris/skills/polaris-video-studio/scripts/run.mjs \
  --input="文案文件路径或纯文本" \
  --theme=midnight-press \
  --output="~/Desktop/output.mp4"
```

参数说明：
- `--input`: 文案文件（.md/.txt）或直接写文本
- `--theme`: 视觉主题（默认 midnight-press，共 23 套可选）
- `--output`: 输出 MP4 路径（默认 ~/Desktop/output.mp4）
- `--skip-scaffold`: 如果项目已存在，跳过脚手架

---

## 分 Phase 执行（精细控制）

### Phase 1 · 内容（AI 执行）

把用户文案转成口播稿（script.md）+ 开发计划（outline.md）。

**执行方式**：
1. 读取用户输入（文案文件或直接文本）
2. 分析文案结构，提取关键信息
3. 生成口播稿（口语化、有停顿标记、适合配音）
4. 生成 outline（章节切分、步数、信息池、素材清单）

**输出**：`work/script.md` + `work/outline.md`

### Phase 2 · 开发（AI 执行）

脚手架 + 逐章开发网页演示。

**执行方式**：
1. `node scaffold.mjs ./work/presentation --theme=<主题>`
2. 删除默认 demo 章节
3. 按 outline 逐章开发：
   - 每章：`Chapter.tsx` + `Chapter.css` + `narrations.ts`
   - 注册到 `chapters.ts`
4. `npx tsc --noEmit` 类型检查
5. `npm run build` 构建

**输出**：`work/presentation/`（完整可运行的 Vite+React 项目）

### Phase 3 · 配音（脚本自动）

```bash
cd work/presentation
npm run extract-narrations   # 扫 narrations.ts → audio-segments.json
npm run synthesize-audio     # MiniMax T2A 逐段合成 mp3
```

**输出**：`work/presentation/public/audio/*/*.mp3`

**多语言配音**：narrations.ts 里的台词用**目标语言**书写（英语视频就写英语口播稿、粤语就写粤语…），
并启用 MiniMax `language_boost` 提升该语言的发音准确度。两种设法：

- 全局：合成前设环境变量，如 `MINIMAX_LANGUAGE_BOOST="English"`（Windows: `$env:MINIMAX_LANGUAGE_BOOST="English"`）。
- 按段：在 `audio-segments.json` 每段加 `"language_boost": "Chinese,Yue"`（按段覆盖全局）。

常用 `language_boost` 取值：`Chinese`、`Chinese,Yue`（粤语）、`English`、`Japanese`、`Korean`、
`Spanish`、`French`、`German`、`Russian`、`Portuguese`、`Italian`、`Arabic`、`Hindi`、`Thai`、
`Vietnamese`、`Indonesian`、`auto`。

### Phase 4 · 出片（脚本自动）

```bash
node ~/Polaris/skills/polaris-video-studio/scripts/pipeline/03-record.mjs \
  --project=<presentation 目录的绝对路径> \
  --output=~/Desktop/output.mp4 \
  --subtitles=zh-Hans,en --burn=zh-Hans,en   # 可选：多语言字幕
```

**多语言字幕**（可选）：

1. 配音、`extract-narrations` 之后，给 `audio-segments.json` **每一段**补 `subtitles` 字段，
   把该段台词译成各目标语言：
   ```json
   { "chapter": "intro", "step": 1, "text": "……", "audio": "intro/1.mp3",
     "subtitles": { "zh-Hans": "简体文本", "zh-Hant": "繁體文本", "en": "English text" } }
   ```
   缺某语言时该段会回退用 `text`。
2. 给 `03-record.mjs` 传 `--subtitles=<逗号分隔语言>`：
   - 每种语言在 MP4 旁生成同名 `.srt`，并作为**可切换软字幕轨**嵌入 MP4；
   - `--burn=<语言>` 把其中 1–2 种叠成（双语）**硬字幕烧进画面**（任何播放器都能看到）；
     不传 `--burn` 默认烧前 2 种，`--no-burn` 则一种都不烧。
   - 字幕时间轴按每段音频的精确时长自动对齐，无需手填时间码。

字幕语言代码：`zh-Hans`(简) `zh-Hant`(繁) `en` `yue`(粤) `ja` `ko` `es` `fr` `de` `ru` `pt`
`it` `ar` `hi` `th` `vi` `id`。

> ⚠️ `--project` 必须指向 Phase 2 脚手架生成的 presentation 项目目录（含 `package.json`），
> **不是** skill 自己的目录。脚本会从该目录读 `audio-segments.json` 自动发现章节/步骤结构，
> 不再写死任何章节数 —— 任意结构的 PPT 都能录。

**执行流程**（全自动，无需手动改脚本）：
1. 预清占用端口的残留进程 → 启动 dev server（`--strictPort` 锁端口，默认 5174，可加 `--port=` 改）
2. Playwright 无头打开 `http://localhost:<port>/`
3. 按 `audio-segments.json` 的有序段落逐步截图（每步一张 1920×1080 PNG）
4. ffprobe 读取每段 mp3 精确时长
5. ffmpeg 把「PNG 冻结 + MP3」合成每步独立视频片段
6. ffmpeg concat 拼接所有片段为完整 MP4
7. 杀掉 dev server **进程树**（不留孤儿进程占端口）并清理临时文件

> 前置条件：跑此脚本前，presentation 目录必须已完成 Phase 3 配音
> （即已有 `audio-segments.json` 与 `public/audio/<章节>/<步>.mp3`）。

**输出**：`~/Desktop/output.mp4`（1920×1080，H.264 + AAC，音画同步）

---

## 主题选择

本 skill 复用 `web-video-presentation` 的 23 套主题。推荐搭配：

| 内容类型 | 推荐主题 | 理由 |
|---|---|---|
| 课件/教学 | chalk-garden、paper-press | 清晰易读，教育感 |
| 数据/报告 | midnight-press、swiss-ikb | 暗底突出数字，专业感 |
| 产品发布 | bold-signal、bauhaus-bold | 强视觉冲击，宣言感 |
| 品牌故事 | dark-botanical、vintage-editorial | 高级质感，有温度 |

---

## 故障排查

| 现象 | 原因 | 解决 |
|---|---|---|
| 视频没声音 | Playwright recordVideo 不录音频 | 本 skill 的 Phase 4 已改用 ffmpeg 拼接方案，不依赖 Playwright 录屏 |
| 音频合成失败 | MiniMax key 无效或额度用完 | 检查 `MINIMAX_API_KEY` 环境变量，或去 Polaris 供应商坞启用 MiniMax |
| 构建报错 | TypeScript 类型错误 | 检查各章节 `narrations.ts` 长度是否与 `.tsx` 中的 step 数匹配 |
| 录屏超时 | dev server 端口被占用 / 上次留下孤儿进程 | 脚本启动前会自动清掉占用端口的进程并杀进程树；如仍超时，看脚本打印的 dev server 日志定位（多半是 npm install 没装全） |
| 找不到 audio-segments.json | 没先跑 Phase 3 配音 | 先在 presentation 目录 `npm run extract-narrations && npm run synthesize-audio` |
| 画面和音频不同步 | 自动播放模式下 step 切换延迟 | Phase 4 的 ffmpeg 方案按每段音频精确时长拼接，天然同步 |

---

## 相关资源

- 依赖安装脚本：`scripts/install-deps.mjs`
- 一键执行脚本：`scripts/run.mjs`
- 录屏合成脚本：`scripts/pipeline/03-record.mjs`
- 可视化链路文档：`references/WORKFLOW.md`
- 原 web-video-presentation skill：`~/Polaris/skills/web-video-presentation/`
