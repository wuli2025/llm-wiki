# 板块⑯ IM 机器人 · 照搬 WeSight 落地规划

> 目标：扫码即配好 → 配好就能在机器人里对话 → 权限全配好。
> 方法：模仿 WeSight（wesight-main）的 IM 机器人模块，抄其可用方法。
> 日期：2026-06-08

---

## 0. WeSight 底层怎么连 Claude Code（已查证，带 file:line）

**不是单一网关，是「引擎路由器 + 统一 Runtime 接口」（策略模式）。**

链路：`IM 网关 → IMCoworkHandler → CoworkEngineRouter(按引擎选) → RuntimeAdapter → 流式事件 → 回发`

| 引擎 | 连法 | 关键文件 |
|---|---|---|
| Cowork(默认 yd_cowork) | 进程内 `@anthropic-ai/claude-agent-sdk` 的 `query()` | `coworkRunner.ts:2385`、`claudeSdk.ts:45` |
| **Claude Code** | **spawn `claude` CLI 子进程**，读 stdout 事件流 | `externalCliRuntimeAdapter.ts:271` |
| Codex | spawn codex CLI + 本地 HTTP/JSON-RPC | `codexAppRuntimeAdapter.ts:91` |
| OpenClaw | 独立 CLI 进程当**网关**，WebSocket 通信，给微信/企微/钉钉/QQ 用插件 | `openclawRuntimeAdapter.ts:1342` |

- 统一接口 `CoworkRuntime`（`agentEngine/types.ts:36`）：`startSession/continueSession` + EventEmitter（message/messageUpdate/complete/error）。
- 路由器 `CoworkEngineRouter`（`coworkEngineRouter.ts:34`）按 session/全局配置查表分发。
- 「跟随全局」= `resolveCoworkAgentEngine()`（`main.ts:1425`）。

**结论**：UI 里「Claude Code」卡 = spawn `claude` CLI（子进程，非网关）。OpenClaw 才是网关那条。

## 1. Rust 能不能做？——能，且已起步

- spawn `claude` = WeSight `ExternalCliRuntimeAdapter` 同款，Rust 直接 `std::process::Command` 即可（feishu.rs 已做）。
- 飞书收发：用 `@larksuiteoapi/node-sdk` 的 `WSClient` + `im.message.receive_v1`，**与 WeSight `nativeFeishuGateway.ts:277` 完全一致**（Polaris 已用 Node 桥实现）。
- Polaris 现状：`feishu.rs` 网关 = Node 桥(WSClient) → Rust 路由 → spawn `claude` → 回发。**架构已对齐 WeSight 的 native 飞书 + Claude Code 引擎**。

## 2. 扫码全自动的真实边界（连 WeSight 也只有两家）

| 平台 | 扫码全自动 | 机制（WeSight） |
|---|---|---|
| 微信 | ✅ | `nativeWeixinGateway.qrLoginStart/Wait` → `@tencent-weixin/openclaw-weixin` 的 `startWeixinLoginWithQr/waitForWeixinLogin` → 存 token → 自动启用 |
| 企业微信 | ✅ | `@wecom/wecom-aibot-sdk` `openBotInfoAuthWindow` → botId/secret → 走 `wecom-openclaw-plugin` 收发 |
| 飞书/钉钉/QQ/云信 | ❌ 手动 | 后台建应用 → 复制 appId/secret；钉钉/QQ/云信走 OpenClaw 插件 |

平台侧权限（飞书事件订阅、消息 scope）只能各自开发者后台开 —— 代码设不了，WeSight 也靠 `docs/im-bot-config` 文档引导。

## 3. 分阶段落地

### 阶段 1 · 飞书引擎打磨（核心，进行中）
- [x] Node 桥 WSClient 长连接收发（= nativeFeishuGateway）
- [x] Rust 网关路由 + spawn claude + 回发
- [x] **per-chat 会话连续性**：chat_id → claude session_id，`--resume` 续接（对标 cowork session）
- [ ] 接 Polaris 人格 + 知识库（在绑定项目里跑，复用 `claude_md::render_for_project`）
- [ ] 权限策略 UI：dmPolicy/groupPolicy/allowFrom/群级（对标 `types.ts:79` FeishuOpenClawConfig）
- [ ] 飞书后台权限引导文档（事件订阅 im.message.receive_v1 + 消息 scope）

### 阶段 2 · 引擎路由抽象（对标 CoworkEngineRouter）
- [ ] Rust `trait BotRuntime { fn reply(&self, chat, text, resume) -> (text, session) }`
- [ ] 适配器：`ClaudeCodeRuntime`(spawn claude，已有)、`CodexRuntime`(spawn codex)、可选 OpenClaw
- [ ] 引擎选择：全局 + 每机器人覆盖（对标「跟随全局」）

### 阶段 3 · 扫码自动配置
- [x] 企业微信：OAuth 回环（系统浏览器 + 内嵌 SDK，绕开 Tauri 弹窗墙）—— 待 source 准入
- [ ] 微信：移植 `@tencent-weixin/openclaw-weixin` 的 QR 登录（qrLoginStart/Wait）→ 存 token → 自动启用
- [ ] 企微/微信消息收发：走 OpenClaw 插件 or 自研桥

### 阶段 4 · 其余平台（钉钉/QQ/云信）
- WeSight 全走 OpenClaw 插件。两条路：(A) 引入 OpenClaw 进程当网关，复用其插件；(B) 逐平台原生 Node 桥（同飞书思路）。
- 决策点：是否引入 OpenClaw（重，但一次性拿到 4 平台）。

## 4. 关键差异 / 待决
- WeSight 用 Agent SDK 可拿**流式 token + 工具权限回调(canUseTool)**；Polaris 现 spawn `claude --print` 是整段返回。要不要上 stream-json 流式？（IM 场景整段回复其实够用）
- 是否引入 OpenClaw（拿微信/企微/钉钉/QQ 一锅端）vs 逐平台自研桥（可控但慢）。
- 平台后台权限无法代设 —— 只能文档 + 启动前检查清单。
