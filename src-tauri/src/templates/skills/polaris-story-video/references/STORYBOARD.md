# storyboard.json 数据契约

故事视频整条流水线（生图 / 配音 / 合成）只读这一份文件。AI 在「规划」阶段产出它。

## 顶层字段

| 字段 | 类型 | 说明 |
|---|---|---|
| `title` | string | 标题（仅记录用） |
| `aspect` | string | 画幅：`9:16`(竖) `16:9`(横) `1:1`(方) `4:3` `3:4` `3:2` `2:3` |
| `style` | string | 美术风格后缀，拼进**每条**生图 prompt，保证全片画风统一 |
| `voice` | string | 全局默认音色 voice_id（每镜可覆盖） |
| `speed` | number | 全局默认语速（MiniMax voice_setting.speed，1.0=正常） |
| `language_boost` | string | 配音语言增强，如 `Chinese` `Chinese,Yue` `English` `Japanese` |
| `bgm` | string | 背景音乐绝对路径，可空 |
| `bgmVolume` | number | BGM 相对人声音量 0–1（默认 0.18） |
| `burnSubs` | bool | 是否把字幕烧进画面（默认 true） |
| `output` | string | 成片默认输出路径（可被 --output 覆盖） |
| `characters` | array | 角色表（见下） |
| `shots` | array | 分镜表（见下） |

## characters[]

| 字段 | 说明 |
|---|---|
| `id` | 角色标识（英文短名，分镜用它引用） |
| `name` | 角色名 |
| `prompt` | 外观描述：年龄/脸型/发型/服饰/气质，越具体跨镜越稳 |
| `ref` | 设定图相对路径（默认 `assets/characters/<id>.png`，脚本自动生成） |

## shots[]

| 字段 | 说明 |
|---|---|
| `id` | 镜号（1,2,3…） |
| `narration` | 旁白文本（配音用；时长由它决定） |
| `subtitle` | 字幕文本，可空=用 narration |
| `image_prompt` | 画面描述：角色动作 + 环境 + 镜头语言（**不要重复写风格词**，风格在顶层 style） |
| `characters` | 出场角色 id 数组；`characters[0]` 作为人物参考图（跨镜一致） |
| `motion` | 运镜：`zoom-in` `zoom-out` `pan-left` `pan-right` `static` |
| `voice` / `speed` / `language_boost` | 覆盖该镜配音（可省，用全局） |
| `duration` | 无旁白镜头的停留秒数（有旁白时忽略，按音频时长） |
| `image` / `audio` | 产物相对路径（默认 `assets/shots/<id>.png` / `assets/audio/<id>.mp3`） |

## 约定

- 路径相对 storyboard.json 所在目录；绝对路径原样使用。
- 分镜数建议 8–24；旁白每镜 1–3 句，口语化、适合朗读。
- 全片只用一种 `style`；多角色镜头主角放 `characters[0]`，配角写进 `image_prompt`。
