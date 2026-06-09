# Polaris · Docker 版

把 Polaris（原 Tauri 桌面 AI 工作台）跑成**浏览器访问的容器服务**。
核心架构：**保留全部 Rust 引擎，用 axum HTTP/WS 外壳替代 Tauri 桌面外壳**。
桌面版与 Docker 版**共用同一份源码**——这是「Windows 更新后能快速更新 Docker」的根基。

---

## 一、快速开始

```bash
# 1) 准备环境变量
cp .env.example .env
#   编辑 .env，至少填一种鉴权：
#   - ANTHROPIC_API_KEY=sk-ant-...           （Claude 官方）
#   - 或 ANTHROPIC_BASE_URL + ANTHROPIC_AUTH_TOKEN（智谱/Kimi/DeepSeek/聚合站）
#   - 公网部署务必设 POLARIS_AUTH_TOKEN=<一串口令>

# 2) 一键构建 + 拉起
docker compose up -d --build

# 3) 浏览器打开
#   http://localhost:8080
#   若设了口令：http://localhost:8080/?token=<你的口令>
```

健康检查：`curl http://localhost:8080/api/health` → `ok`。

---

## 二、它是怎么接起来的（架构）

```
浏览器 (Vue3 前端，与桌面版同一份)
   │  src/tauri.ts 适配层：非 Tauri 环境自动改走 ↓
   ├── invoke(cmd,args)  ──HTTP──▶  POST /api/invoke   （≈75 个引擎命令分发）
   ├── listen(topic,cb)  ──WS────▶  GET  /ws           （emit 事件广播）
   └── 文件上传          ──multipart▶ POST /api/upload  （替代原生文件对话框）
                                          │
                              polaris-server (Rust · axum)
                                          │  src/host.rs 的 shim AppHandle
                                          │  把 app.emit() 转成 WS 广播
                              ┌───────────┴───────────┐
                              │ kb / chat / conv /     │  ← 桌面版同款 .rs，未改业务逻辑
                              │ provider / skills /... │     仅顶部 import + 命令宏 cfg 门控
                              └───────────┬───────────┘
                                  spawn   │  stdin 喂 prompt，解析 stream-json
                              ┌───────────▼───────────┐
                              │   claude CLI（镜像预装）│──▶ 各 LLM 供应商
                              └───────────────────────┘
```

关键实现（都在 `src-tauri/src/`）：

| 文件 | 作用 |
|---|---|
| `host.rs` | server 模式的 `AppHandle` 替身：`emit→broadcast`、`path().resource_dir()→/app/resources` |
| `server.rs` | axum 服务：`/api/invoke` 分发、`/ws` 推流、`/api/upload`、静态托管、可选口令鉴权 |
| `bin/polaris-server.rs` | server 二进制入口 |
| 各引擎模块 | `use tauri::AppHandle` → `#[cfg]` 门控双导入；`#[tauri::command]` → `#[cfg_attr(feature="desktop", tauri::command)]` |

`Cargo.toml`：`tauri` 等设为 **optional**，`default = ["desktop"]`，新增 `server` feature。
- 桌面构建：`cargo build`（默认 desktop）—— 一切照旧。
- Docker 构建：`cargo build --bin polaris-server --no-default-features --features server` —— 不拉 Tauri，Linux 无需 webkit2gtk。

---

## 三、⭐ Windows 更新后，如何快速同步到 Docker

**因为是同一份源码，更新只需重建镜像，无需任何移植。**

```bash
# 在 Windows 上正常改完代码、提交后（桌面版照常 cargo build / 发版）：
git pull              # 或把最新源码同步到部署机
docker compose up -d --build
```

`Dockerfile` 做了**依赖缓存分层**：第三方依赖（axum/tokio/解析库等）单独成层，
只要 `Cargo.toml`/`Cargo.lock` 没变，重建时**不会重编依赖**，
通常 1–3 分钟即可出新镜像。前端同理（`package-lock.json` 不变则复用 `npm ci` 层）。

> 维护纪律：改后端时若新增了 `#[tauri::command]`，记得在 `src/server.rs` 的
> `dispatch_sync` 里加一条对应分发（一行）。其余业务逻辑改动**两端自动共享**。

---

## 四、数据持久化

| 卷 | 容器内路径 | 内容 |
|---|---|---|
| `polaris-data` | `/root/Polaris` | 知识库 `PolarisKB/`、对话历史、项目、产物、技能 |
| `polaris-claude` | `/root/.claude` | claude 凭证、`settings.json`（供应商切换/OAuth 登录态） |
| `polaris-config` | `/root/.config` | KB 设置等 XDG 配置 |

容器重建（`up --build`）数据不丢。备份直接备份这三个卷即可。

---

## 五、鉴权说明

- **API Key 模式（推荐，最稳）**：`.env` 里设 `ANTHROPIC_API_KEY` 或第三方
  `ANTHROPIC_BASE_URL`+`ANTHROPIC_AUTH_TOKEN`。容器把这些环境变量传给 spawn 的 claude。
- **供应商坞切换**：进入 App 内「供应商」面板切换/新增，会写入 `/root/.claude/settings.json`（持久化）。
- **OAuth 订阅（Claude Pro / Codex）**：无头容器难走设备码流程。变通：把已登录的
  `~/.claude` 内容拷进 `polaris-claude` 卷复用。本期主推 API Key。
- **访问口令**：设 `POLARIS_AUTH_TOKEN` 后，`/api/*` 需 `Authorization: Bearer <口令>`，
  WS 需 `?token=<口令>`。前端用 `http://host:8080/?token=<口令>` 访问会自动记住口令。

---

## 六、特性存活矩阵（容器版）

| 板块 | 状态 | 说明 |
|---|---|---|
| 对话 / 流式 / 工具调用 | ✅ 保留 | WS 推流，体验等价 |
| 知识库 KB（扫描/图谱/检索/编译/上传） | ✅ 保留 | 纯逻辑，卷持久化 |
| 技能 / 人格 / CLAUDE.md / 供应商 / 用量 / Codex 代理 | ✅ 保留 | 文件落盘到卷 |
| 文件上传 | ✅ 保留 | 拖拽 → `/api/upload` multipart |
| 产物预览 / 成品编辑器 | ✅ 保留 | `artifact_read` 返回正文/dataUrl，iframe 预览 |
| 飞书 / 企微网关 | ⚠ 可用 | 长连接服务端更合适；OAuth 回调 URL 需公网可达 |
| PPT / 网页 / 视频工坊 | ⚠ 多数保留 | 视频需镜像加 ffmpeg/playwright（按需扩镜像） |
| 可运行项目（一键起前后端） | ⚠ 受限 | 容器内嵌套起服务受限，list/status 可用 |
| Docker 沙箱板块 | ⛔ 降级 | Docker-in-Docker 风险高，返回 stub |
| 环境医生（安装 claude/node） | ⛔ 简化 | 镜像已预装，安装类命令返回提示 |
| 自动更新 / 托盘 / 宠物窗 | ⛔ 删除 | 桌面专属；更新走 `docker pull` / `up --build` |

---

## 七、常用运维

```bash
docker compose logs -f polaris      # 看日志
docker compose restart polaris      # 重启
docker compose down                 # 停（保留卷）
docker compose down -v              # 停并删数据卷（慎用）
docker exec -it polaris-web bash    # 进容器排查（claude --version 等）
```

## 八、稳健性：单轮对话看门狗

容器内偶发：个别极简 prompt 会让 claude 触发子代理（`claude --print`，其 cwd 落在 `/`）
对文件系统做无界扫描而长时间不返回，既拖死本轮、又占住 OAuth 订阅的并发槽拖垮后续消息。

对策：`POLARIS_CHAT_TIMEOUT_SECS`（默认 180s）。超时仍未结束则杀掉整个 claude 进程组，
stdout 关闭 → 正常 emit error+done，系统自愈、释放并发槽。设 0 关闭。
桌面版默认关闭（保持原行为），仅容器启用。

> 实测：实质性问题（联网检索、生成 PPT/网页、写文件、KB 取证）均正常；
> 仅「只回复两个字」这类极简多轮 prompt 偶发触发上述扫描，看门狗保证不会无限挂死。

## 九、扩展为「全功能镜像」（媒体/视频）

在 `Dockerfile` 阶段3 的 apt 安装里加 `ffmpeg`，并按需装 Playwright/Chromium
（`npx playwright install --with-deps chromium`），compose 里加 `shm_size: 1gb`。
镜像会增大约 400MB+，故默认做「轻量镜像」，按需开启。
