# 北极星 · Polaris (MVP v0.1)

本地优先 AI 工作台,墨蓝水墨风。Tauri 2 + Vue 3 + Rust。

## v0.1 实现范围

MVP 只实现 PRD v6 七板块中的 3 个核心板块,其余写规划 PRD 待后续:

| 板块 | 实现 |
|------|------|
| ① 对话核心 | spawn `claude` CLI(沙箱或宿主),stream-json 流式渲染 |
| ② 维基知识库 | 文件扫描 / 关键词加权评分搜索 / 双链图谱 / 简易 ingest |
| ⑤ 安全沙箱层 | 基于 `alpine:3.20` 的轻量镜像(<200MB),docker CLI 包装 |

UI:
- 左栏可收缩,极简(对话 / 知识库 / 图谱 / 沙箱 / 设置)
- 主区:hero「你说,北极星画」+ 输入框 + 四档权限按钮
- 右抽屉可收缩(输出产物 / 参考资料 / 沙箱日志)

未实现板块的规划 PRD: [`docs/planning/`](./docs/planning/)

## 前置依赖

| 工具 | 用途 |
|------|------|
| Node 20+ | 前端构建 (`npm`) |
| Rust 1.80+ | Tauri 后端 |
| Docker Desktop | 沙箱镜像构建 / 运行 |
| `claude` CLI | 对话核心调用(沙箱内自动装,宿主测试需 `npm i -g @anthropic-ai/claude-code`) |

## 启动开发模式

```powershell
# 必须把 cargo 加进 PATH (memory 已记录)
$env:PATH = "C:\Users\mi\.cargo\bin;$env:PATH"

cd D:\polaris\polaris-app
npm install   # 首次
npm run tauri:dev
```

Vite 端口固定 1420。若失败先清端口:

```powershell
Get-NetTCPConnection -LocalPort 1420 -ErrorAction SilentlyContinue |
  Select-Object -ExpandProperty OwningProcess | ForEach-Object {
    Stop-Process -Id $_ -Force
  }
```

## 三大模块手动测试路径

### 1. 维基知识库
- 左栏点「知识库」
- 「管理」tab → 填本机 `.md` 文件绝对路径 → Ingest
- 「浏览」tab → 搜索 / 点开预览
- 「图谱」(左栏第三项)→ 看 `[[wiki-link]]` 构成的图

KB 根目录:`~/Polaris/PolarisKB/`(初次启动自动生成 `raw/`, `output/`, `wiki/` 三层骨架 + `CLAUDE.md` + `wiki/index.md`)

### 2. 安全沙箱
- 左栏点「沙箱」
- 「构建镜像」(首次 2-5 分钟,拉 `alpine:3.20` + `npm install -g @anthropic-ai/claude-code`)
- 「启动容器」(`docker run -d --memory=4g --cpus=2 ... polaris-sandbox:alpine sleep infinity`)
- 「执行」框输入 `claude --version` 验证 CLI 可用

Dockerfile 在 `~/Polaris/sandbox/Dockerfile`(由 app 启动时拷贝出来,方便用户审计/魔改)。

### 3. 对话核心
- 左栏点「对话」
- 默认「沙箱执行」+「注入 KB」+「手动授权」开启
- 输入「你好,介绍一下 Karpathy 维基方法论」(Ctrl+Enter 发送)
- 看气泡流式输出

如果沙箱未启动,关掉「沙箱执行」会回退到宿主机 `claude.exe`(需已 `claude login`)。

## 文件结构

```
polaris-app/
├── src/                   # Vue 3 前端
│   ├── App.vue            # 三栏 grid 布局
│   ├── tauri.ts           # 后端 API 包装 (浏览器降级 stub)
│   ├── stores/app.ts      # Pinia: 侧栏/抽屉/视图
│   └── components/        # Sidebar / RightDrawer / ChatPanel / WikiBrowse / KnowledgeGraph / SandboxStatus
├── src-tauri/             # Rust 后端
│   ├── src/lib.rs         # 入口 + 命令注册
│   ├── src/kb.rs          # 模块② 维基
│   ├── src/sandbox.rs     # 模块⑤ 沙箱
│   ├── src/chat.rs        # 模块① 对话
│   └── src/templates/     # Dockerfile + KB 骨架模板
├── docs/planning/         # 未实现板块的规划 PRD
└── README.md              # 本文
```

## 已知限制 (留给 v0.2)

- 索引只在内存,进程重启重扫(待接 SQLite via `kb::storage::*`)
- 沙箱 audit 流未接入(右抽屉「沙箱日志」tab 是 placeholder)
- 进程池 / 排队 / 优先级未实现(对话发出即调起一个 claude 进程)
- 权限策略仅传 `--permission-mode` 参数,未实现 `permission_check` 拦截
- 浏览器模式(`npm run dev`)只能预览 UI,所有后端调用走 stub
