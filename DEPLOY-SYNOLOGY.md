# Polaris · 群晖 Synology 部署指南

> 配套文件：`docker-compose.synology.yml`（部署用）、桌面 PRD `Polaris-群晖Docker适配-PRD.html`（设计依据）。
> 本指南覆盖 R6（反向代理，DSM 侧配置）与整体上机步骤。GPU 为何不进容器见 PRD §04。

---

## 0. 前置

- 群晖 DSM 7.2，已安装 **Container Manager** 套件。
- 一个 Btrfs 存储空间（便于快照备份）。
- 若要公网访问：一个指向 NAS 的域名 + DSM 已能签 Let's Encrypt 证书。

---

## 1. 建数据目录（bind mount 落点）

File Station 里建共享文件夹 `docker`（Btrfs），并在其下建：

```
/volume1/docker/polaris/data      # KB / 对话 / 项目 / 产物 / 技能
/volume1/docker/polaris/claude    # claude 凭证 + .claude.json（登录态）
/volume1/docker/polaris/config    # XDG 配置
```

> 用 bind mount 而非命名卷：File Station 可见、可单独做 **Btrfs 快照复制** 备份。

---

## 2. 建受限运行用户（R2，禁止 root 跑容器）

控制面板 → 用户与群组 → 新增用户 `polaris`（不给管理员组，仅授予上面 `docker/polaris` 文件夹读写）。

取它的 UID/GID（SSH 登录后 `id polaris`，或经验值普通用户从 1026 起、群组 `users`=100）：

```
uid=1026(polaris) gid=100(users)
```

把这两个值填进 `docker-compose.synology.yml` 的 `PUID` / `PGID`。容器 entrypoint 会据此降权运行，
共享文件夹里产生的文件属主即 `polaris`，宿主侧可正常管理。

---

## 3. 导入项目并启动

Container Manager → 项目 → 新增：
- 项目名 `polaris`
- 路径选 `/volume1/docker/polaris`
- 来源选「上传 `docker-compose.yml`」→ 传 `docker-compose.synology.yml`
- 按需在「环境」里填 `ANTHROPIC_API_KEY`、`POLARIS_AUTH_TOKEN`（公网必填）、`PUID/PGID`
- 构建并启动

启动后容器内会以非 root 运行，`/api/health` 返回 `ok`，`/api/status` 可看内存/磁盘水位。

> 群晖内置 Compose 版本较旧：布尔值写 `1/0`；首次导入若报「不支持的选项」，对照 PRD §03.2
> 在该固件上做一次 smoke test（确认 healthcheck / mem_limit / build 被识别）。

---

## 4. 反向代理 + HTTPS（R6）

容器对外暴露 `8080`，**不要**直接抢 DSM 占用的 443/5001。统一用反代对外：

控制面板 → 登录门户 → 高级 → **反向代理** → 新增：

| 项 | 值 |
|---|---|
| 来源 协议 | HTTPS |
| 来源 主机名 | `polaris.你的域名` |
| 来源 端口 | `443` |
| 目标 协议 | HTTP |
| 目标 主机名 | `localhost` |
| 目标 端口 | `8080` |

**必做两项**（否则长任务会 504、流式会断）：
- 「自定义标头」→ 勾选 **Create WebSocket**（透传 `Upgrade`/`Connection`，Polaris 的 `/ws` 流式靠它）。
- 「自定义标头」或高级里把 **代理超时调到 600s**（批量 PPT / 视频等长任务单连接耗时长）。

证书：控制面板 → 安全性 → 证书，给这个反代域名签/绑 Let's Encrypt。

公网访问形如 `https://polaris.你的域名/?token=<POLARIS_AUTH_TOKEN>`。

---

## 5. 资源治理与稳定性（已在 compose 落地）

- `mem_limit: 6g` —— 防单容器泄漏拖垮整机；超限 OOM kill 本容器，DSM 与其它容器存活。
  （群晖内核缺 CFS 模块，CPU 硬限速 `cpus=` **不生效**，故不设，只能在 UI 设 CPU 优先级。）
- 日志 `json-file` `max-size 10m` / `max-file 5` —— 防 stdout 膨胀写满系统分区。
- 看门狗 `POLARIS_CHAT_TIMEOUT_SECS=180` —— 单轮对话**连续空闲**超时才杀进程组，
  活跃流式的长任务不会被误杀。
- 镜像自带 `tini` 作 PID1 —— 回收 claude 扇出的子进程僵尸。

水位监控：`GET http://<nas>:8080/api/status` 返回容器内存（贴近 mem_limit）、数据盘用量、
推理端点状态等，可接入告警。

---

## 5b. 出 PPT / 视频 —— 用 full 渲染镜像

默认镜像（slim）只跑聊天 / KB / 网站生成，**不含渲染栈**。要在容器里出 **PPT / 讲解视频**，构建 full：

```bash
# 命令行
docker build --build-arg POLARIS_RENDER=1 -t polaris-web:full .
# 或 compose（已接 build-arg + shm_size）
POLARIS_RENDER=1 docker compose -f docker-compose.synology.yml up -d --build
```

full 镜像额外打包 **chromium（截图）+ fonts-noto-cjk（防中文豆腐块）+ ffmpeg（出视频）**，约 +350MB。
其余约束已在 compose 接好：`shm_size: 512m`（chromium 截大页防崩）、`mem_limit ≥ 2g`（渲染峰值，群晖建议调高）。

**自检**：浏览器里调 `/api/status` 或命令 `forge_preflight`，看 `summary.can_render_ppt / can_render_video`：
- slim → `false` + blockers 列出「缺 chromium / 字体 / ffmpeg」（透明告知，不会跑到一半报错）。
- full → `true`，零 blocker。出视频还需配 MiniMax key（容器无系统语音兜底，preflight 会提示）。

> Forge 渲染引擎本体（capture/codec/tts/pptx/fx）仍在 P1–P5 路线上；当前 full 镜像已备好**渲染运行环境**，
> preflight 让产品能透明地告诉用户「这环境能出什么、缺什么」。

---

## 6. GPU 推理（A2）—— 独立节点，经网络对接（R3）

群晖 Container Manager **不开放容器内 GPU 透传**（官方仅 DVA 机型；社区魔改 CUDA 在容器内失败甚至
整机卡死，见 PRD §04）。因此 A2 推理不在本容器内：

- 在一台能正常 GPU 直通的 Linux 主机（裸机 / Proxmox / Unraid 直通）部署嵌入/重排/ASR 推理服务，
  按 `infer.rs` 顶部的端点契约暴露 `/embed` `/rerank` `/transcribe` `/health`。
- 在本 compose 的环境里设 `POLARIS_INFER_ENDPOINT=http://<gpu主机>:<端口>`。
- 不设则自动走群晖 CPU 兜底（Xeon D-1521 有 AVX2，可跑量化小模型，较慢）。
- `GET /api/status` 的 `infer` 字段会显示端点是否配置、是否连通。

---

## 7. 备份 / 恢复

- 用 **Snapshot Replication** 对 `/volume1/docker/polaris` 定时快照。
- DB 写入中的快照非崩溃一致：备份前停容器，或在低写入时段做。
- 恢复：还原快照目录 → 在 Container Manager 重新启动项目即可（数据/登录态随卷回来）。
