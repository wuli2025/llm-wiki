# Polaris 规划 PRD 索引

本目录存放 **MVP v0.1 未实现** 的板块规划文档。改一处需同步主 PRD
(`c:\Users\mi\Desktop\新建文件夹\PRD-v6.html`)。

## 实现状态

| 板块 | 状态 | 范围 |
|------|------|---------|
| ① 对话核心 | ✅ 已实现 | 直接调 claude CLI(沙箱内/宿主),stream-json 渲染气泡 + 成品预览 |
| ② 维基知识库 | ✅ 已实现 → **借鉴 llm_wiki 增强落地** | 扫描 / 关键词加权评分搜索 / 双链星河图谱 / 拖拽入库 / 构建知识网(摄入即编译)。**新增**:上下文预算(治 32k)+ 安全路径护栏 + 增量入库缓存 + `kb_lint` 质量检查 + `kb_enrich_links` 自动补双链 + `kb_dedup` 智能去重(均「AI 出决策、代码执行」)。见 [02-kb-enhancement-PRD.md](./02-kb-enhancement-PRD.md) |
| ③ Skill 技能库 | ✅ 已实现 | 技能中心：catalog 预置 + 用户自建 + 外部导入(git/url/zip)。见 [03-skill-PRD.md](./03-skill-PRD.md) |
| ④ 统一调度中心 | ⏳ 规划中 | 见 [04-scheduling-PRD.md](./04-scheduling-PRD.md) |
| ⑤ 安全沙箱层 | ✅ 已实现 → **Phase 1 板块化提取完成** | 轻量 alpine 镜像 + docker CLI 包装,build/start/stop/exec。已抽离为独立 `polaris-sandbox` crate |
| ⑥ 多模态输入 | 🔧 部分实现 | 任意格式拖拽 → 转 Markdown(`convert.rs`) 已落地；语音输入待做。见 [06-multimodal-PRD.md](./06-multimodal-PRD.md) |
| ⑦ 设置中心 | 🔧 部分实现 | 工作文件夹配置 + 首次启动引导已落地；其余见 [07-settings-PRD.md](./07-settings-PRD.md) |

### 框架外新增（不在原七板块）

| 模块 | 状态 | 说明 |
|------|------|------|
| API 供应商坞 + 用量看板 | ✅ 已实现 | 多供应商一键切换(写 `~/.claude/settings.json`) + 读 `~/.claude/projects` 统计用量 |
| 启动体验 | ✅ 已实现 | 北极星启动页(SplashScreen) + 首次工作文件夹引导(Onboarding) |
| 内置浏览器 | ⏳ 规划中 | CloakBrowser 隐身浏览器集成。见 [08-browser-PRD.md](./08-browser-PRD.md) |
| **Coworker 升级** | 🔧 进行中 | PPT 能力 + 产物归属 + 参考资料文件夹 + CubeSandbox(E2B) + 多开 + 工位数字人 + 完成提醒。见 [09-coworker-upgrade-PRD.md](./09-coworker-upgrade-PRD.md) |
| **毛主席资料库与人格** | ✅ 已实现 | 默认资料库随包打进安装包 + 首启播种 + 内置「毛主席」人格项目 + 请教毛主席(毛选式客观分析,标来源 HTML) + 资料库删除/清空。见 [10-mao-persona-PRD.md](./10-mao-persona-PRD.md) |
| **动态编排(多智能体)** | ✅ 已实现 | 对话框「动态编排」模式：编排器拆 N 个独立子任务 → `Task` 子代理并行扇出 → 每条 实现/对抗式校验/修复 → 汇总；headless 下放行 `Task`。见 [11-dynamic-workflows-PRD.md](./11-dynamic-workflows-PRD.md) |
| **⑫ 人格模块** | ✅ 已实现 | 「目录说明」升格为「人格」(顶层导航)：7 预设人格(6+毛主席) + 画廊一键套用 + 「每个项目=一个人格」+ 人格↔专属知识库 scope 绑定 + L1 身份/L5 时间分层注入。导航改版:人格升顶层、更新降「更多」。见 [12-persona-PRD.md](./12-persona-PRD.md) |
| **⑬ 桌面宠物** | ✅ 已实现 | 透明置顶不抢焦点的 Tauri 小窗(`?win=pet`) + SVG/CSS 五情绪动画,镜像对话运行态(thinking/coding/done/error,前端 `pet://status` 广播);「更多→桌面宠物」开关+情绪预览。见 [13-desktop-pet-PRD.md](./13-desktop-pet-PRD.md) |
| **⑭ 飞书网关** | 🔧 阶段 A 落地 | Rust 自研:配置存储 + tenant_access_token + 机器人信息 + 连接测试 + 去重/权限/ReplyGuard 纯函数(5 单测绿);「更多→飞书」配置 UI。阶段 B(WebSocket 长连接实时收发)需真实凭证联调,待续。见 [14-feishu-gateway-PRD.md](./14-feishu-gateway-PRD.md) |

## 演进路径

```
v0.1         →  ① + ② + ⑤  跑通核心闭环
v0.x (现在)  →  ③ 技能中心 + ④ 供应商坞 + ⑥ 文件转换 + 启动引导 已落地
下一步       →  ④ 调度中心 (权限策略 + 进程池) 替换直传参数
             →  ⑥ 语音输入 (豆包模式) + ⑦ 设置中心整合
             →  ⑧ 内置浏览器 (CloakBrowser)
```

## 板块边界铁律 (沿用 PRD §16)

1. **跨板块只能调公开 API**,不能 import 对方内部 struct/fn
2. **事件优先于直接调用**(`tauri::Emitter::emit` + `listen`)
3. **每个板块独立测试**(`cargo test -p polaris-mod-xxx`)

## 想加新板块?

先在主 PRD 加章节,再在本目录建 `NN-xxx-PRD.md`,最后才动代码。
