# Polaris 规划 PRD 索引

本目录存放 **MVP v0.1 未实现** 的板块规划文档。改一处需同步主 PRD
(`c:\Users\mi\Desktop\新建文件夹\PRD-v6.html`)。

## 实现状态

| 板块 | 状态 | MVP 范围 |
|------|------|---------|
| ① 对话核心 | ✅ v0.1 已实现 | 直接调 claude CLI(沙箱内/宿主),stream-json 渲染气泡 |
| ② 维基知识库 | ✅ v0.1 已实现 | 扫描 / 关键词加权评分搜索 / 双链图谱 / 简易 ingest |
| ③ Skill 技能库 | ⏳ 规划中 | 见 [03-skill-PRD.md](./03-skill-PRD.md) |
| ④ 统一调度中心 | ⏳ 规划中 | 见 [04-scheduling-PRD.md](./04-scheduling-PRD.md) |
| ⑤ 安全沙箱层 | ✅ v0.1 已实现 → **Phase 1 板块化提取完成** | 轻量 alpine 镜像 + docker CLI 包装,build/start/stop/exec。已抽离为独立 `polaris-sandbox` crate |
| ⑥ 多模态输入 | ⏳ 规划中 | 见 [06-multimodal-PRD.md](./06-multimodal-PRD.md) |
| ⑦ 设置中心 | ⏳ 规划中 | 见 [07-settings-PRD.md](./07-settings-PRD.md) |

## 演进路径

```
v0.1 (现在)  →  ① + ② + ⑤  跑通核心闭环
v0.2         →  ④ 调度中心 (权限策略 + 进程池) 替换 v0.1 直传参数
v0.3         →  ③ Skill 库 + ⑥ 多模态 (文件转 MD)
v0.4         →  ⑥ 语音输入 (豆包模式) + ⑦ 设置中心整合
```

## 板块边界铁律 (沿用 PRD §16)

1. **跨板块只能调公开 API**,不能 import 对方内部 struct/fn
2. **事件优先于直接调用**(`tauri::Emitter::emit` + `listen`)
3. **每个板块独立测试**(`cargo test -p polaris-mod-xxx`)

## 想加新板块?

先在主 PRD 加章节,再在本目录建 `NN-xxx-PRD.md`,最后才动代码。
