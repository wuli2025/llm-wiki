# 板块⑭ 飞书网关（Feishu Gateway）PRD

> 状态：🔧 阶段 A 已落地（2026-06-02，5 单测绿）；阶段 B（WebSocket 长连接）待真机联调
> 思想来源：WeSight 用 `@larksuiteoapi/node-sdk` 的 **WebSocket 长连接 + 去重 + 权限 + ReplyGuard**。
> Polaris 是 Tauri/Rust，**不抄其 TS 代码**，用 Rust 自研同等链路。

## 0. 一句话

在「更多→飞书」里填飞书机器人的 App ID/Secret，**无需公网服务器**即可让用户在飞书里 @ 机器人对话，
消息走 Polaris 既有对话管线（人格+知识库），AI 回复发回飞书。目标：小白也能连上。

## 1. 为什么是长连接（而非 webhook）

webhook 要公网回调地址，小白配不了。飞书官方「长连接」由客户端**主动建立 WebSocket**，事件推过来，
无需公网。WeSight 正是走这条。Polaris 用 Rust 的 `tokio-tungstenite` 自研握手与帧解析。

## 2. 鉴权三件套

- App ID + App Secret（飞书开放平台建机器人即得）。
- domain：飞书国内版 / Lark 国际版。
- 用 App ID/Secret 换 `tenant_access_token`（REST `POST /open-apis/auth/v3/tenant_access_token/internal`），发消息用；定期刷新（≈2h）。
- 拉一次 `GET /open-apis/bot/v3/info` 拿机器人自己的 open_id → 过滤自发消息防自言自语。

## 3. 完整链路（自研 Rust 实现）

```
飞书用户 @机器人 → WS 长连接收事件 → 去重 → 权限检查 → 解析+去@ → Polaris 对话管线 → 分块回发
```

| 环节 | Rust 落点 | 要点 |
|---|---|---|
| 长连接 | `feishu/conn.rs` | REST 取 endpoint → `tokio-tungstenite` 连 → 心跳 pong → 自动重连（指数退避） |
| 去重 | `feishu/dedup.rs` | 最近 N(1000) 条 message_id 的环形集合（VecDeque+HashSet）。**纯函数，带单测** |
| 权限 | `feishu/policy.rs` | 私聊 dmPolicy(开放/白名单)、群聊需 @机器人。**纯函数，带单测** |
| 解析 | `feishu/parse.rs` | text 取 .text；post 取标题+正文；删 `<at>` 标记 |
| 跑 AI | 复用 `chat::send` 等 | 映射成内部「一条飞书会话→一个对话」 |
| ReplyGuard | `feishu/reply_guard.rs` | 核对「AI 自然语言承诺(定时/提醒)」vs「工具实际成功」，不符则替换为「未真正创建」提示。**纯函数，带单测** |
| 回发 | `feishu/send.rs` | 每 ~3500 字分块；首块 reply（引用），后续 create；REST `im/v1/message` |

## 4. 配置 UI（FeishuSettings.vue，在「更多」）

- 表单：App ID、App Secret（密码框）、domain（国内/国际）、私聊策略、群聊是否需 @、白名单。
- 「连接测试」按钮：取 token + bot info → 显示机器人名/状态。
- 连接状态指示（已连/重连中/断开 + 最近收发时间）。

## 5. 分阶段交付（务实，诚实标注风险）

- **阶段 A（先落地、可验证）**：配置存储 + `tenant_access_token` 获取 + `bot/v3/info` + 主动**发**消息 REST + 去重/权限/ReplyGuard 纯函数 + 单测。→ 这部分不依赖真实长连接即可测。
- **阶段 B（核心、风险点）**：WebSocket 长连接握手与事件帧解析、心跳、自动重连。**需真实飞书 app 凭证联调**，作为独立 PR。
- **阶段 C**：富媒体（图片/文件）、interactive card、配对码自助白名单。

## 6. 后端结构

`crates/polaris-feishu`（独立 crate，严守边界）或先 `src-tauri/src/feishu/` 子模块（体量定）。
依赖：`tokio-tungstenite`、`reqwest`、`serde`。对外只暴露：`start(config)`/`stop()`/`status()`/`send(conv,text)` + 一个 `on_message` 回调注入对话管线。

## 7. 验收

- 阶段 A：填凭证→连接测试通过（拿到 token/bot 名）；去重/权限/ReplyGuard 单测绿；`cargo test` 绿。
- 阶段 B：真实飞书 @机器人 → Polaris 回复发回；断网自动重连。
- 全程 `cargo check`/`build` 绿；凭证加密存储，不明文落盘日志。

## 8. 边界

独立 crate，跨板块只走公开 API + 事件；对话管线通过回调/命令对接，不在 feishu 里直接 import chat 内部 struct。
