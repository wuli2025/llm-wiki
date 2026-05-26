# Source Registry

[1] CubeSandbox (TencentCloud) GitHub | https://github.com/tencentcloud/CubeSandbox | OFFICIAL/PRIMARY | 2026 — RustVMM+KVM, <60ms, <5MB/inst CoW, 千级密度, E2B SDK 兼容, CubeVS eBPF, Apache2.0, Win=WSL2(Win11 22H2++nested), PVM
[2] CubeSandbox Docs quickstart | https://docs.cubesandbox.ai/guide/quickstart | OFFICIAL | 2026 — 系统要求 x86_64 Linux+KVM / WSL2
[3] E2B 官网 + memo.d.foundation/breakdown/e2b + Spheron blog | https://e2b.dev/ | OFFICIAL+SECONDARY | 2026 — Firecracker microVM, ~150ms 预热快照, 云服务, 24h session, 全功能VM(Chromium+terminal)
[4] microsandbox GitHub | https://github.com/microsandbox/microsandbox | OFFICIAL/PRIMARY | 2026 — libkrun+KVM, <100ms, 自托管本地, OCI镜像, SDK(Rust/Py/TS/Go), MCP(Claude Code), Apache2.0, 仅 Linux+KVM/macOS Apple Silicon(无 Windows)
[5] Daytona daytona.io + GitHub + Northflank对比 + Medium sub-90ms | https://www.daytona.io/ | OFFICIAL+SECONDARY | 2026 — 容器(Docker)默认, sub-90ms(27ms优化), 可选 Kata/Sysbox 强隔离, 自托管 docker-compose, 快照, API-key
[6] Northflank blog 排名/对比 | https://northflank.com/blog/best-code-execution-sandbox-for-ai-agents | SECONDARY/VENDOR | 2026 — Kata+gVisor microVM, 无限会话, 任意OCI, BYOC, 2M+/月
[7] Docker 官方博客 + Docker Sandboxes docs | https://www.docker.com/blog/comparing-sandboxing-approaches-ai-agents/ | OFFICIAL | 2026 — 容器共享内核易逃逸; Docker Sandboxes 用 microVM
[8] Windows Sandbox MS Learn + FAQ | https://learn.microsoft.com/en-us/windows/security/application-security/application-isolation/windows-sandbox/ | OFFICIAL | 2026-03 — 一次仅一个实例; 多实例建议改用 Hyper-V VM
[9] Firecracker on WSL2 | https://medium.com/@veltun/... + tutorialsdojo + gist startergo | SECONDARY | 2024-26 — WSL2 跑 Firecracker 需自定义内核(device-mapper/nested), 默认未开
[10] Hyper-V nested virtualization | https://learn.microsoft.com/en-us/windows-server/virtualization/hyper-v/enable-nested-virtualization | OFFICIAL | 2026 — Set-VMProcessor -ExposeVirtualizationExtensions, Linux VM 内可跑 KVM
[11] Addo Zhang Medium 容器→microVM | https://addozhang.medium.com/ai-agent-code-execution-sandboxes-isolation-from-containers-to-microvms-e80848effea5 | SECONDARY | 2026-03
[12] Veracode 报告(经 SoftwareSeni/Northflank 转引) | 2025 — 45% AI 生成代码未过安全测试
[13] betterstack 11 best sandbox runners / fast.io / rywalker | https://betterstack.com/community/comparisons/best-sandbox-runners/ | SECONDARY | 2026 — 品类横评

## Citation integrity
所有核心数字(冷启动/内存/隔离技术/Windows 要求/许可证)均来自一手仓库或官方文档([1][2][3][4][8][10])。性能横评辅以厂商博客([5][6][7]),标注为厂商来源。Windows 无原生 KVM 为操作系统事实。
