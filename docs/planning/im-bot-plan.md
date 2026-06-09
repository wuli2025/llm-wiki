# Polaris IM 机器人规划

> 基于 WeSight 源码分析，规划 Polaris 的社交平台机器人接入  
> 日期：2026-06-08  
> 参考：WeSight v0.x（Electron + TypeScript）

---

## 一、需求总览

根据用户提供的截图和需求，本次规划 **IM 机器人模块**：

### 1.1 社交平台机器人扫码接入
- **平台清单**（从 WeSight 截图识别）：
  - 微信（个人号）
  - 企业微信
  - 钉钉
  - 飞书
  - QQ
  - 云信（网易）
  - 小蜜蜂
  - POPO（网易）
- **核心体验**：扫码登录 → 机器人上线 → 消息转发到 Claude Code 对话

### 1.2 飞书模块处理
- **现状**：Polaris 已有飞书板块（板块⑬）
- **方案**：将飞书整合到新的「IM 机器人」板块中，作为 8 个平台之一

---

## 二、WeSight 架构分析

### 2.1 核心组件

#### （1）`imGatewayManager.ts` - IM 网关统一管理器
```typescript
// 路径：src/main/im/imGatewayManager.ts
// 职责：
// - 管理所有 IM 平台的生命周期（启动/停止/状态检查）
// - 统一消息分发（IMMessage → CoworkHandler）
// - 扫码登录流程（QR code 显示 → 轮询等待 → 连接建立）
// - 健康检查（connectivity check）

class IMGatewayManager extends EventEmitter {
  // 三大 Gateway：
  - nativeFeishuGateway: NativeFeishuGateway   // 飞书 SDK
  - nativeWeixinGateway: NativeWeixinGateway   // 微信 OpenClaw 插件
  - nimGateway: NimGateway                     // 云信 SDK
  
  // 其他平台（钉钉/QQ/小蜜蜂/POPO）通过 OpenClaw 的 MCP bridge
}
```

**关键流程**：
1. **扫码登录**：
   - 调用平台 SDK 的 `startLoginWithQr()` → 返回 QR code data URL
   - 前端显示二维码
   - 后端轮询 `waitForLogin()` → 扫码成功返回 token
2. **消息接收**：
   - WebSocket/长轮询监听消息
   - 解析为标准 `IMMessage` 格式
   - 传给 `IMCoworkHandler` → 转发到 Claude Code 对话
3. **消息发送**：
   - Claude 回复 → 通过 gateway 的 `sendMessage()` 发送到 IM 平台

#### （2）平台实现细节

| 平台 | 实现方式 | 依赖 | 扫码机制 |
|------|---------|------|---------|
| **飞书** | `NativeFeishuGateway.ts` | `@larksuiteoapi/node-sdk` | SDK 原生 QR 登录 |
| **微信** | `NativeWeixinGateway.ts` | OpenClaw 插件（wework-bot） | 插件提供 QR API |
| **企业微信** | 同微信 | 同上（复用 wework-bot） | 企业微信扫码接口 |
| **钉钉** | OpenClaw MCP | `dingtalk-stream` SDK | 通过 MCP bridge |
| **QQ** | OpenClaw MCP | QQ Bot SDK | 通过 MCP bridge |
| **云信** | `NimGateway.ts` | 网易云信 SDK | SDK QR 登录 |
| **小蜜蜂** | OpenClaw MCP | （未详细展开） | MCP bridge |
| **POPO** | OpenClaw MCP | （未详细展开） | MCP bridge |

#### （3）OpenClaw 的作用
WeSight 把 **OpenClaw** 作为"万能适配器"：
- OpenClaw 本身是一个类似 Claude Code 的 AI agent runtime
- WeSight 通过 `openclawRuntimeAdapter.ts` 与其通信
- OpenClaw 的 **MCP (Model Context Protocol)** bridge 可以扩展任意外部服务
- 钉钉/QQ/小蜜蜂/POPO 都是通过 OpenClaw 的 MCP 插件实现的

---

## 三、Polaris 实现方案

### 3.1 技术栈差异

| 维度 | WeSight | Polaris |
|------|---------|---------|
| **桌面框架** | Electron | Tauri |
| **后端语言** | TypeScript (Node.js) | Rust |
| **前端** | Vue 3 | Vue 3 |
| **IM SDK** | Node.js 生态（npm 包） | **需要桥接** |

**核心挑战**：Rust 无法直接调用 Node.js 的 IM SDK（如 `@larksuiteoapi/node-sdk`）

### 3.2 混合架构设计

#### 方案 A：Tauri 调用 Node.js 子进程（推荐）
```
[Tauri Rust 后端]
     ↓ spawn child_process
[Node.js Bridge 服务]  (独立进程)
     ↓ 调用 IM SDK
[@larksuiteoapi/node-sdk, dingtalk-stream, etc.]
     ↓ 接收消息
[通过 stdin/stdout 或 HTTP 回传给 Rust]
```

**优势**：
- 复用 Node.js 生态，所有 SDK 开箱即用
- Rust 负责存储、状态管理、与 Claude Code 的集成
- 类似 Polaris 现有的 `web-video-presentation` skill（已通过 Node.js 实现）

**实现步骤**：
1. 新建 `src-node/im-bridge/` 目录
2. 安装依赖：`@larksuiteoapi/node-sdk`、`dingtalk-stream` 等
3. 编写 `im-bridge.mjs`：
   ```javascript
   // 通过 stdin 接收 Rust 发来的 JSON 命令
   // { cmd: "feishu.login", params: {...} }
   // 调用对应 SDK，结果通过 stdout 返回
   ```
4. Rust 端新建 `src-tauri/src/modules/im_bridge.rs`：
   ```rust
   pub fn spawn_im_bridge() -> Child {
       Command::new("node")
           .arg("src-node/im-bridge/im-bridge.mjs")
           .stdin(Stdio::piped())
           .stdout(Stdio::piped())
           .spawn()?
   }
   ```

#### 方案 B：纯 Rust 实现（不推荐）
- 部分平台有 Rust SDK（如飞书有社区 crate）
- 但大部分 SDK 是 Node.js 独占（微信/钉钉/云信）
- **工作量巨大**，且生态不成熟

**结论**：采用 **方案 A**

---

### 3.3 板块设计

#### 「IM 机器人」板块

**位置**：左侧边栏「更多」→「IM 机器人」（新增板块⑯）

**UI 设计**（参考 WeSight 截图）：
```
┌─────────────────────────────────────┐
│  IM 机器人                           │
├─────────────────────────────────────┤
│  ● 微信              ○ 未连接  ▶   │  ← 点击展开扫码
│  ● 企业微信          ○ 未连接  ▶   │
│  ● 钉钉              ○ 未连接  ▶   │
│  ● 飞书              ● 已连接  ▶   │  ← 绿点表示在线
│    └─ Feishu Bot 1                  │
│  ● QQ                ○ 未连接  ▶   │
│  ● 云信              ○ 未连接  ▶   │
│  ● 小蜜蜂            ○ 未连接  ▶   │
│  ● POPO              ○ 未连接  ▶   │
└─────────────────────────────────────┘
```

**点击「微信」展开后**：
```
┌──────────────────────────────────────┐
│  微信设置                  [关闭]    │
├──────────────────────────────────────┤
│  扫码登录                            │
│  ┌────────────────┐                  │
│  │   [QR Code]    │  ← 二维码        │
│  │                │                  │
│  └────────────────┘                  │
│  请使用微信扫码登录                   │
│                                      │
│  Bot ID: _________   (可选)          │
│  Secret: _________   (可选)          │
│                        [保存]        │
└──────────────────────────────────────┘
```

**扫码成功后**：
```
┌──────────────────────────────────────┐
│  微信设置                  [关闭]    │
├──────────────────────────────────────┤
│  ✅ 已连接                           │
│  账号：张三 (wxid_abc123)            │
│  在线时长：2小时35分                 │
│                                      │
│  [断开连接]  [查看消息记录]          │
└──────────────────────────────────────┘
```

---

### 3.4 数据流设计

#### （1）扫码登录流程
```
[前端 Vue]
  ↓ 点击「微信」→「扫码登录」
[Rust Command: start_weixin_login]
  ↓ spawn Node.js bridge
  ↓ 调用 wework-bot SDK: startWeixinLoginWithQr()
  ↓ 返回 QR data URL
[前端显示二维码]
  ↓ 用户扫码
[Node.js 轮询 waitForWeixinLogin()]
  ↓ 扫码成功 → 返回 { accountId, token }
[Rust 保存到数据库]
  ↓ 发送事件给前端
[前端更新状态：已连接]
```

#### （2）消息接收流程
```
[IM 平台] 用户发消息："帮我写个 Rust 排序函数"
  ↓ WebSocket 推送
[Node.js Bridge] 接收消息
  ↓ 通过 stdout 发给 Rust
[Rust: im_handler.rs]
  ↓ 解析为标准 IMMessage
  ↓ 调用 chat.rs: send_message_for_im()
[Claude Code 处理]
  ↓ 返回回复
[Rust 发给 Node.js Bridge]
  ↓ Node.js 调用 SDK 发送消息
[IM 平台] 用户收到："以下是 Rust 排序..."
```

---

### 3.5 模块文件清单

#### Rust 后端
```
src-tauri/src/modules/
├── im_bridge.rs            // Node.js bridge 生命周期管理
├── im_handler.rs           // IM 消息处理（转发到 chat.rs）
└── im_store.rs             // IM 配置存储（账号/token/状态）

src-tauri/src/commands/
└── im_commands.rs          // Tauri 命令：登录/发送/接收
```

#### Node.js Bridge
```
src-node/im-bridge/
├── im-bridge.mjs           // 主入口（stdin/stdout 通信）
├── platforms/
│   ├── feishu.mjs          // 飞书 SDK 封装
│   ├── weixin.mjs          // 微信 SDK 封装
│   ├── dingtalk.mjs        // 钉钉 SDK 封装
│   ├── qq.mjs              // QQ SDK 封装
│   ├── yunxin.mjs          // 云信 SDK 封装
│   └── ...
├── package.json
└── package-lock.json
```

#### 前端 Vue
```
src/views/sidebar/
└── IMBot.vue               // IM 机器人板块（替换原 Feishu.vue）

src/components/im/
├── PlatformCard.vue        // 单个平台卡片（微信/钉钉/...）
├── QRCodeModal.vue         // 扫码弹窗
└── BotStatusBadge.vue      // 在线/离线状态指示器
```

---

## 四、分阶段实现计划

### Phase 0：基础架构（1-2 天）
- [ ] 创建 `src-node/im-bridge/` 目录
- [ ] 实现 Node.js bridge 基础框架（stdin/stdout 通信）
- [ ] Rust 端实现 `im_bridge.rs`（spawn/管理子进程）
- [ ] 单元测试：Rust ↔ Node.js 双向通信

### Phase 1：飞书接入（2-3 天）
**为什么先做飞书**：
- Polaris 已有飞书模块代码基础（`src-tauri/src/modules/feishu.rs`）
- 飞书 SDK 文档完善（`@larksuiteoapi/node-sdk`）
- 可复用现有 UI 组件

**任务**：
1. Node.js 侧：
   - 安装 `@larksuiteoapi/node-sdk`
   - 实现 `platforms/feishu.mjs`：
     - `feishu.login()` → 返回 QR code
     - `feishu.waitLogin()` → 轮询扫码
     - `feishu.sendMessage()` → 发送消息
     - `feishu.onMessage()` → 监听消息
2. Rust 侧：
   - `im_commands.rs`: `start_feishu_login()`、`send_feishu_message()`
   - `im_handler.rs`: 接收 Node.js 推送的消息 → 转发到 `chat.rs`
3. 前端：
   - 新建 `IMBot.vue`（仅飞书部分）
   - 扫码流程 + 消息测试

### Phase 2：微信/企业微信接入（3-4 天）
**难点**：微信没有官方 SDK，需要用第三方方案

**方案**：
1. **调研微信接入方式**：
   - 开源方案：[wechaty](https://github.com/wechaty/wechaty)（支持个人微信/企业微信）
   - 或使用企业微信官方 API（需企业认证）
2. **集成 Wechaty**：
   ```javascript
   // src-node/im-bridge/platforms/weixin.mjs
   import { WechatyBuilder } from 'wechaty'
   
   export async function weixinLogin() {
       const bot = WechatyBuilder.build()
       bot.on('scan', (qrcode) => {
           // 发送 QR code 给 Rust
       })
       bot.on('login', (user) => {
           // 登录成功
       })
       await bot.start()
   }
   ```
3. **前端**：复用飞书的 UI，切换平台参数

### Phase 3：钉钉/QQ/云信/小蜜蜂/POPO（4-5 天）
- 钉钉：`dingtalk-stream` SDK
- QQ：`oicq` 或 QQ 官方 Bot SDK
- 云信：`@yxim/nim-web-sdk`
- 小蜜蜂/POPO：调研对应 SDK

每个平台按 Phase 1 的流程实现。

### Phase 4：集成测试 + 优化（2-3 天）
- 端到端测试：扫码 → 发消息 → Claude 回复 → 收到回复
- 错误处理：网络断开、token 过期、SDK 崩溃
- 性能优化：并发消息处理、消息队列

---

## 五、关键技术细节

### 5.1 Rust ↔ Node.js 通信协议

#### （1）Rust → Node.js（通过 stdin）
```json
{
  "id": "req-001",
  "platform": "feishu",
  "action": "login",
  "params": {}
}
```

#### （2）Node.js → Rust（通过 stdout）
```json
{
  "id": "req-001",
  "status": "ok",
  "data": {
    "qrcode": "data:image/png;base64,iVBORw0KGgoAAAANS..."
  }
}
```

或：
```json
{
  "event": "message",
  "platform": "feishu",
  "data": {
    "from": "user123",
    "text": "帮我写个函数",
    "timestamp": 1717833600
  }
}
```

### 5.2 存储结构

#### SQLite 表结构（新增）
```sql
-- IM 账号表
CREATE TABLE im_accounts (
    id TEXT PRIMARY KEY,
    platform TEXT NOT NULL,  -- 'feishu' | 'weixin' | 'dingtalk' | 'qq' | 'yunxin' | 'xiaomifeng' | 'popo'
    account_id TEXT,
    account_name TEXT,       -- 显示名称
    token TEXT,
    bot_config TEXT,         -- JSON 格式的额外配置（如 bot_id, secret）
    status TEXT,             -- 'online' | 'offline'
    created_at INTEGER,
    updated_at INTEGER
);

-- IM 消息记录（可选，用于历史查询）
CREATE TABLE im_messages (
    id TEXT PRIMARY KEY,
    account_id TEXT,
    platform TEXT,
    from_user TEXT,
    to_user TEXT,
    content TEXT,
    timestamp INTEGER,
    FOREIGN KEY(account_id) REFERENCES im_accounts(id)
);
```

### 5.3 错误处理

| 错误场景 | 处理方案 |
|---------|---------|
| Node.js 进程崩溃 | Rust 监听 `child.wait()`，自动重启 |
| SDK token 过期 | 前端提示重新扫码 |
| 网络断开 | 心跳检测 + 自动重连（每 30s ping） |
| 消息发送失败 | 重试队列（最多 3 次） |

---

## 六、UI/UX 细节

### 6.1 扫码体验优化
- **加载状态**：显示"正在生成二维码..."（避免白屏）
- **倒计时**：二维码有效期（如 2 分钟），到期自动刷新
- **扫码成功动画**：✅ + 绿色波纹扩散

### 6.2 消息通知
- **桌面通知**：收到 IM 消息时弹窗（可配置）
- **未读角标**：左侧边栏「IM 机器人」图标显示未读数

### 6.3 多账号支持
- 允许同一平台登录多个账号（如微信小号）
- 每个账号独立开关

---

## 七、风险与挑战

### 7.1 技术风险
| 风险 | 影响 | 缓解措施 |
|------|------|---------|
| Node.js SDK 版本不兼容 | 部分平台无法接入 | 提前测试，准备降级方案 |
| Rust 子进程管理复杂 | 内存泄漏/僵尸进程 | 使用 `tokio::process` + 定期健康检查 |
| 微信反爬虫检测 | 账号被封 | 使用官方协议（如 Wechaty Puppet Official） |

### 7.2 合规风险
- **微信/QQ**：个人号机器人违反 ToS，建议仅支持**企业微信/QQ 开放平台**
- **免责声明**：UI 中提示"仅用于个人学习，商业使用请遵守平台规则"

### 7.3 维护成本
- IM SDK 更新频繁（如飞书 API v2 → v3）
- 需定期跟进各平台 SDK 升级

---

## 八、开发排期（总计 12-15 天）

```
Week 1:
  Day 1-2:  Phase 0 基础架构
  Day 3-5:  Phase 1 飞书接入
  
Week 2:
  Day 6-9:  Phase 2 微信/企业微信
  Day 10:   Phase 3 钉钉

Week 3:
  Day 11-12: Phase 3 QQ/云信/小蜜蜂/POPO
  Day 13-15: Phase 4 测试 + Bug 修复 + 文档
```

---

## 九、后续扩展

### 9.1 高级功能
- **消息模板**：预设回复（如"正在处理，请稍候"）
- **关键词触发**：监听特定关键词自动执行 workflow
- **群聊支持**：@ 机器人才响应
- **语音消息**：自动转文字（调用语音识别 API）

### 9.2 与现有模块集成
- **自媒体运营板块**：IM 收到选题 → 自动跑撰稿 workflow
- **知识库**：IM 问答自动注入 KB 上下文
- **自动化流程**：定时推送运营报告到飞书群
- **人格模块**：IM 可选择不同人格（如毛主席）回复

---

## 十、总结

### 核心设计思路
1. **复用 WeSight 的 IM 架构**，用 Rust + Node.js 混合实现
2. **新增 IM 机器人板块**，统一管理 8 个社交平台（微信/企业微信/钉钉/飞书/QQ/云信/小蜜蜂/POPO）
3. **扫码体验对标微信登录**，2 分钟内完成接入
4. **原飞书板块整合进 IM 板块**，作为 8 个平台之一

### 技术亮点
- **Tauri + Node.js 混合架构**：发挥两者生态优势
- **标准化消息协议**：`IMMessage` 统一格式，易扩展
- **可插拔平台设计**：新增平台只需写一个 `.mjs` 文件
- **跨平台消息历史**：统一存储，方便查询和分析

### 预期效果
- 用户可通过微信/钉钉/飞书等 8 个平台与 Polaris 的 Claude Code 对话
- 一个 Polaris 实例可同时管理多个平台的多个机器人账号
- 对标 WeSight 的 IM Hub 功能，成为企业级 AI Agent 协作中心

---

**下一步行动**：
1. 用户确认规划方案
2. 开始 Phase 0 基础架构开发
3. 搭建 CI 自动测试各平台 SDK
