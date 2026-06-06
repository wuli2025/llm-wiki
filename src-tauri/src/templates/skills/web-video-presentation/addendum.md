# 网页演示视频（Polaris 集成版）

> 这是 ConardLi `web-video-presentation` 原 skill 的**完整安装**，并由 Polaris
> 做了三处增强，让它在本软件里开箱即跑（尤其是 Windows）。**下面的 Polaris
> 规则优先于原文**，其余流程/方法论一律照原 SKILL 正文执行（见本文件下半部分）。

## 本技能包根目录（绝对路径，下文用 `PKG` 指代）

```
PKG = __PKG_DIR__
```

`PKG/references/`、`PKG/themes/`（23 套主题）、`PKG/templates/`、`PKG/scripts/`
全部已在本地，原 SKILL 正文里所有「参见 references/xxx.md」都是 `PKG/references/xxx.md`，
用 Read 直接读，不是死链。

## 增强一 · 脚手架走 Node（跨平台，免 bash/WSL）

原 SKILL 的 `bash scripts/scaffold.sh` 在 Windows 上依赖 git-bash/WSL，常跑不通。
**本机一律改用 Node 版脚手架**（同样产出 Vite+React+TS 项目）：

```bash
# 列主题
node "PKG/polaris/scaffold.mjs" --list-themes
# 建项目（在用户当前工作目录下）
node "PKG/polaris/scaffold.mjs" ./presentation --theme=<用户选的主题id>
```

（mac/Linux 也可继续用原 `bash PKG/scripts/scaffold.sh`，二者等价。）
脚手架会自动 `npm install` + 装 `tsx` + 跑 typecheck —— **项目级依赖自动装好**。

## 增强二 · 配音自动调用 Polaris 的 MiniMax（无需 mmx-cli / 不用登录 / 不要 GroupId）

原 SKILL 的默认 provider 用 `mmx-cli`（要 `npm i -g mmx-cli` + `mmx auth login`）。
**本集成已替换为直连 MiniMax T2A 的 Node 合成器**，它**自动**从 Polaris 供应商坞
（`~/Polaris/data/providers.json` 的 `minimax` 供应商）取 key —— 用户只要在供应商坞
启用过「MiniMax（粉丝福利）」，配音就零配置可用（已实测：sk-cp- key 直接通过 T2A
鉴权，返回 mp3）。

脚手架已把项目的 `npm run synthesize-audio` 接到这个合成器。Phase 3 音频合成：

```bash
cd presentation
npm run extract-narrations      # 扫所有 narrations.ts → audio-segments.json（先让用户扫一眼）
npm run synthesize-audio        # 调 MiniMax T2A，逐段写 public/audio/<id>/<step>.mp3（断点续合）
npm run synthesize-audio -- --force   # 强制重合成
```

调音色 / 模型（可选）：
- 音色：`MINIMAX_TTS_VOICE=female-shaonv npm run synthesize-audio`
- 高质量模型：`MINIMAX_TTS_MODEL=speech-02-hd npm run synthesize-audio`
- 想换 key：设环境变量 `MINIMAX_API_KEY=...` 覆盖（优先级高于供应商坞）

> 如果用户没启用 MiniMax 供应商，合成器会报「找不到 MiniMax key」。这时引导用户
> 去 Polaris 左下角供应商坞启用 MiniMax，或设 `MINIMAX_API_KEY`，再重跑。
> 也可回退原 SKILL 的 openai provider（`PRESENTATION_TTS=openai`，走 bash 版）。

## 增强三 · 依赖自检

安装本技能后已自动跑过 `node PKG/polaris/bootstrap.mjs`（自检 node/npm/ffmpeg/MiniMax key）。
ffmpeg 只在「校验音频时长」时用到，缺了不影响合成与录屏。

## 录屏（Phase 4，用户动手）

合成完音频后，打开 `http://localhost:5173/?auto=1`（或 5174）→ 按一次 `SPACE`
→ 整片按音频自动播放并翻页 → 用屏幕录制软件 16:9 全屏录 → 停录裁头尾即成片。
详见 `PKG/references/RECORDING.md`。

---

> 以下为 ConardLi 原 `SKILL.md` 正文（方法论 + 协作流程）。**凡涉及 scaffold 命令
> 与音频 provider，以上面 Polaris 规则为准**；其余照此执行。

