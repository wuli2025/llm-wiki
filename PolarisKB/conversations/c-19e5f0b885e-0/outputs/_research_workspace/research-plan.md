# Research Plan — Windows-friendly agent sandboxes (Docker alternatives)

## Research question
为 Windows 系统寻找比 Docker 更适合 AI agent 的沙箱方案（类似 CubeSandbox / E2B），目标是实现 **agent 多开**（同机并发运行大量 agent 实例）。产出自包含 HTML 报告。

## Audience
Polaris 项目作者（已有 CubeSandbox(E2B) 后端 + Docker 可选后端），技术决策者。

## Freshness / geography
2025–2026 最新方案；全球生态（开源 + 商业云）。

## Output type
Full report (HTML, self-contained) + Decision Framework（选型）。

## Stakes
Medium — 影响 Polaris 沙箱后端架构选型。

## Threads
1. CubeSandbox / E2B 及"agent 沙箱"品类：是什么、原理（Firecracker microVM）、为何比 Docker 更适合 agent。
2. Windows 本地沙箱选项：Windows Sandbox、Hyper-V、WSL2、Docker Desktop(WSL2 后端)、Podman、Windows 容器。
3. 轻量隔离运行时：Firecracker / gVisor / Kata / Cloud Hypervisor / microsandbox —— 能否在 Windows 跑。
4. Agent 多开的密度/性能：冷启动、内存开销、快照、并发，落到 Windows 的现实建议。

## What would change the conclusion
- 若存在 Windows 原生、低开销、可多实例的 microVM 方案，则本地优先。
- 若本地隔离在 Windows 代价过高，则云 sandbox（E2B/Daytona/Modal）+ 本地 WSL2/容器 混合为最优。
