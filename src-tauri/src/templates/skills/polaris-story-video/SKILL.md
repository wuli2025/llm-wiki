---
id: polaris-story-video
name: Polaris 故事视频（AI 生图 · 旁白 · 一键成片）
description: 把文案/主题做成「带 AI 高清插画」的故事短视频并输出 MP4。完整链路：文案→分镜脚本→角色设定→MiniMax 生图（人物+环境，跨镜一致）→MiniMax 配音→ffmpeg 运镜/字幕/BGM 合成。竖屏 9:16 / 横屏 16:9 / 方图 1:1。
source: official
author: Polaris
created_at: 0
---

# Polaris 故事视频

> 把一段文案或一个主题，做成**带 AI 生成插画**的故事短视频（抖音/视频号竖屏式），输出 MP4。
>
> **和「课件视频」的根本区别**：课件视频是"网页 PPT → 录屏"；故事视频是
> **"AI 生成人物+环境高清图 → Ken-Burns 运镜 → 旁白+字幕+BGM → 合成"**。核心是真生图。

---

## 依赖（首次使用前自检）

| 工具 | 用途 | 安装 |
|---|---|---|
| Node.js ≥ 18 | 脚本运行 | Polaris 内置 |
| ffmpeg / ffprobe | 运镜·字幕·混音·合成 | 缺则按提示装；**无需 Playwright/浏览器** |
| MiniMax key | 生图 image-01 + 配音 T2A | Polaris 供应商坞自动提供（同一个 key） |

```bash
node ~/Polaris/skills/polaris-story-video/scripts/install-deps.mjs
```

---

## 数据契约：storyboard.json

整条链路只认一份 `storyboard.json`（角色 + 分镜）。schema 见 `references/STORYBOARD.md`，要点：

```jsonc
{
  "title": "标题",
  "aspect": "9:16",                 // 9:16 竖屏 | 16:9 横屏 | 1:1 方图
  "style": "<美术风格后缀，拼进每条生图 prompt，保证全片画风统一>",
  "voice": "audiobook_male_1",      // 全局默认音色（每镜可覆盖）
  "speed": 1.0,                      // 全局默认语速
  "language_boost": "Chinese",
  "bgm": "/绝对路径/bgm.mp3",         // 可空
  "bgmVolume": 0.18,
  "burnSubs": true,                  // 是否烧录字幕
  "characters": [
    { "id": "hero", "name": "小满",
      "prompt": "8岁中国女孩，圆脸，齐刘海黑色短发，红色棉袄，眼神明亮",
      "ref": "assets/characters/hero.png" }
  ],
  "shots": [
    { "id": 1,
      "narration": "从前，在雪山脚下的小村庄里，住着一个叫小满的女孩。",
      "subtitle": "雪山脚下的小村庄",          // 可空=用 narration
      "image_prompt": "雪山脚下的中国北方小村庄，清晨，炊烟，小满站在木屋门口张望，电影感广角",
      "characters": ["hero"],                  // 第一个角色作为人物参考(跨镜一致)
      "motion": "zoom-in",                      // zoom-in|zoom-out|pan-left|pan-right|static
      "voice": "female-tianmei", "speed": 1.0,
      "image": "assets/shots/1.png", "audio": "assets/audio/1.mp3" }
  ]
}
```

**人物一致性**：每个角色先生成一张「设定图」(`characters[].ref`)，之后该角色出现的分镜把它作为
`subject_reference` 传给生图，使其跨镜长相一致。MiniMax 单图主体参考一次一人——一个镜头里多个角色时，
把**主角**放 `characters[0]`（作参考），其余角色靠 `image_prompt` 文字描述。

---

## 工作流

### 第一步 · 规划（AI 执行，产出两份文件）

1. 读用户文案/主题（含上传素材）。
2. 设计**角色表**：每个反复出现的人物写清外观（年龄/脸型/发型/服饰/气质），便于设定图与跨镜一致。
3. 把故事切成 **8–24 个分镜**，每镜写：旁白(`narration`)、字幕(`subtitle`)、画面描述(`image_prompt`，
   含角色动作+环境+镜头语言)、出场角色、运镜(`motion`)。旁白节奏按目标时长配。
4. 产出两份文件到产物目录（文件名严格如下）：
   - `分镜脚本.md` —— 给人看的：角色表 + 逐镜「旁白/画面/运镜」。
   - `storyboard.json` —— 给脚本跑的机器数据（上面的 schema）。
5. 产出后**停下等确认**（除非全自动模式）。

> 把 UI 选定的 **画幅 / 美术风格 / 音色 / 语速 / 语言 / 字幕 / BGM** 如实写进 storyboard.json，
> 不要自行更改用户的选择。

### 第二步 · 出片（脚本自动）

```bash
node ~/Polaris/skills/polaris-story-video/scripts/run.mjs \
  --storyboard=<storyboard.json 绝对路径> \
  --output=<成片 mp4 绝对路径>
```

`run.mjs` 依次跑：生图(`minimax-image.mjs --batch`) → 配音(`minimax-tts.mjs --batch`) →
合成(`compose.mjs`)。也可分步单独跑（便于只重做某一步，加 `--force` 覆盖重算）：

```bash
node scripts/minimax-image.mjs --batch --storyboard=<sb>     # 角色设定图 + 逐镜高清图
node scripts/minimax-tts.mjs   --batch --storyboard=<sb>     # 逐镜旁白 mp3
node scripts/compose.mjs --storyboard=<sb> --output=<mp4>    # 运镜+字幕+BGM 合成
```

生成的图落在 storyboard 同目录的 `assets/characters/` 与 `assets/shots/`，旁白在 `assets/audio/`，
便于你逐张检查、对不满意的镜头删图后 `--force` 重生。

---

## 美术风格建议（写进 storyboard.style）

| 内容 | 推荐风格关键词 |
|---|---|
| 治愈/童话 | 温暖治愈系绘本插画，柔和光线，水彩质感 |
| 国风/历史 | 中国水墨动画风，留白，写意，淡彩 |
| 励志/金句 | 电影感写实，戏剧光影，暗调，大气 |
| 科普/未来 | 半写实 3D，皮克斯质感，明亮 |
| 悬疑/奇幻 | 暗黑奇幻插画，浓郁色彩，体积光 |

全片**只用一种风格**写进 `style`，逐镜 `image_prompt` 只描述内容不重复风格词，画风才统一。

---

## 故障排查

| 现象 | 解决 |
|---|---|
| 生图/配音报 key 错 | 供应商坞启用 MiniMax，或设 `MINIMAX_API_KEY` |
| 同一角色每镜长相不一 | 确认该角色有 `ref` 设定图、且分镜 `characters[0]` 指向它 |
| 字幕乱码/不显示 | 装中文字体；可设 `STORY_SUB_FONT="思源黑体"` 等已装字体名 |
| 找不到 ffmpeg | 跑 install-deps.mjs；或设 env `FFMPEG`/`FFPROBE` |
| 想保留中间产物排查 | 设 `STORY_KEEP_TMP=1`，`.compose/` 不清理 |

---

## 相关资源
- 依赖自检：`scripts/install-deps.mjs`
- 一键出片：`scripts/run.mjs`
- 生图：`scripts/minimax-image.mjs`　配音：`scripts/minimax-tts.mjs`　合成：`scripts/compose.mjs`
- 数据契约：`references/STORYBOARD.md`
