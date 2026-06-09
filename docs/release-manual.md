# 发版手册（手动建 Release + 打通自动更新）

本仓库的 Actions `GITHUB_TOKEN` **无法创建 Release**（即便 Workflow 权限为 write、
job 声明 `contents: write`，调 create-a-release 仍报 `Resource not accessible by
integration` —— 账号级限制）。所以 `release.yml` 只负责**构建 + 用更新私钥签名 + 上传
artifact**，Release 由仓库 owner 在本机用 `gh` 手动建。下面是完整流程（以 0.2.11 为例）。

## 1. 触发构建

```powershell
# 版本号需先在 src-tauri/tauri.conf.json 与 package.json 同步好
git tag -a v0.2.12 -m "..."
git push origin v0.2.12     # 触发 release.yml: Windows + macOS 并行构建并签名
```

构建从 **tag 指向的提交树**出包。注意本仓库 local `main`（含 macOS 支持）与
`origin/main` 是**无共同祖先的两条历史**，但内容上 local `main` = origin 内容 + mac 支持，
所以从 local `main` 打 tag 出的是内容超集，不会回退功能。

## 2. 等构建完，下载已签名产物

```powershell
$runId = (gh run list --workflow=release.yml --limit 1 --json databaseId --jq '.[0].databaseId')
$dir = "D:\polaris\_release"
gh run download $runId -D $dir          # 得到 bundles-windows-latest / bundles-macos-latest
```

关键产物：
- Windows: `...\nsis\Polaris_<v>_x64-setup.exe` 和 `.exe.sig`
- macOS:   `...\macos\Polaris.app.tar.gz` 和 `.sig`（更新用）、`...\dmg\Polaris_<v>_universal.dmg`（全新安装用）

> `release.yml` 不产出 `latest.json`（tauri-action 只在「发布」阶段才生成），所以要**手工拼**。

## 3. 手工拼 `latest.json`

`signature` 取对应 `.sig` 文件的**全文**（单行 base64）。macOS universal 包让
`darwin-x86_64` 与 `darwin-aarch64` 指向同一个 `Polaris.app.tar.gz`。

> ⚠️ **下载 url 必须套国内镜像**（`https://gh-proxy.com/` 前缀），否则国内用户**检查更新能弹窗、
> 但下载安装包必失败**（报 `error sending request for url`）。原因：`tauri.conf > endpoints` 只给
> 「检查 latest.json」那一跳配了镜像，Tauri updater **下载安装包时只认 latest.json 里写死的 url**，
> 不会再套 endpoints 镜像。签名是对**文件字节**签的，与 url 无关，套镜像不影响验签。
> （客户端侧 `updater.rs::mirror_candidates` 还会再补 `ghfast.top`/直连兜底，但 latest.json 这层
> 必须先把首选源设成镜像，老客户端才走得通。）

```json
{
  "version": "0.2.12",
  "notes": "……",
  "pub_date": "2026-..-..T..:..:..Z",
  "platforms": {
    "windows-x86_64": { "signature": "<setup.exe.sig 全文>", "url": "https://gh-proxy.com/https://github.com/wuli2025/polaris_coworker/releases/download/v0.2.12/Polaris_0.2.12_x64-setup.exe" },
    "darwin-x86_64":  { "signature": "<app.tar.gz.sig 全文>", "url": "https://gh-proxy.com/https://github.com/wuli2025/polaris_coworker/releases/download/v0.2.12/Polaris.app.tar.gz" },
    "darwin-aarch64": { "signature": "<app.tar.gz.sig 全文>", "url": "https://gh-proxy.com/https://github.com/wuli2025/polaris_coworker/releases/download/v0.2.12/Polaris.app.tar.gz" }
  }
}
```

> 已发布版本若忘了套镜像，可热修而无需重新发版：取线上 latest.json → 给三个 url 加镜像前缀 →
> `gh release upload vX.Y.Z latest.json --clobber`（签名不变，老客户端下次自检即生效）。

## 4. 建 Release（owner 身份，标记 latest）

```powershell
gh release create v0.2.12 `
  "$dir\...\Polaris_0.2.12_x64-setup.exe" `
  "$dir\...\Polaris_0.2.12_universal.dmg" `
  "$dir\...\Polaris.app.tar.gz" `
  "$dir\latest.json" `
  --repo wuli2025/polaris_coworker --title "Polaris v0.2.12" --notes "……" --latest
```

`--latest` 让 `releases/latest/download/latest.json` 解析到本版 —— 这正是
`tauri.conf.json > plugins.updater.endpoints` 轮询的地址，旧版本据此自动升级。

## 5. 同步 Cloudflare 自托管兜底（**必做**，否则客户端第三兜底源指向旧包）

客户端 `updater.rs::mirror_candidates` 的下载候选链 = `gh-proxy → ghfast.top → Cloudflare(站点 downloads/) → 直连 github`，
任一源「卡死/失败」自动切下一个（`update.timeout=120s` 触发）。**Cloudflare 那一跳要本版的安装包真在站点上**，
否则会 404 → 验签前就被跳过（安全但 CF 兜底失效）。装包很小（win ~6MB / mac ~14MB），Pages 直接托管、无需 R2。

```powershell
# 把 win 安装器 + mac 自动更新包（注意是 .app.tar.gz，不是 dmg）拷进站点 downloads/，文件名保持与 GitHub 资产一致
Copy-Item "$dir\...\Polaris_0.2.12_x64-setup.exe" "D:\polaris\polaris-site\downloads\"
Copy-Item "$dir\...\Polaris.app.tar.gz"          "D:\polaris\polaris-site\downloads\"
# 可选：dmg 供官网手动下载
Copy-Item "$dir\...\Polaris_0.2.12_universal.dmg" "D:\polaris\polaris-site\downloads\"
wrangler pages deploy "D:\polaris\polaris-site" --project-name polaris --commit-dirty=true
# 验证生产域名服务的是真二进制（exe 头=4d 5a / tar.gz 头=1f 8b），且字节数与本地一致
```

> ⚠️ `polaris-2us.pages.dev` 对未知路径会回退 `index.html`（返回 200 但其实是 HTML）。验证时**别只看状态码**，
> 要查首字节魔数 + 字节数与本地是否一致，确认是真包而非回退页。

## 6. 验证自动更新端点

```powershell
Invoke-RestMethod "https://github.com/wuli2025/polaris_coworker/releases/latest/download/latest.json"
# 应返回新版 version + windows-x86_64 / darwin-x86_64 / darwin-aarch64 三个平台条目
# 且每个 url 都带 https://gh-proxy.com/ 前缀（见第 3 步）
```

## 注意

- **macOS 未签名**：Tauri updater 的 minisign 签名校验与 Apple 公证是两回事。更新包能下载
  并校验通过，但未做 Apple 签名时自替换偶有不稳，且首启仍需 `xattr -dr com.apple.quarantine`。
  要彻底顺滑需 Apple Developer 证书（见 `docs/macos.md`）。
- **更新私钥**：CI 用仓库 secret `TAURI_SIGNING_PRIVATE_KEY` / `..._PASSWORD` 签名；本地构建
  才需显式传私钥文件（密码见项目记忆）。公钥已在 `tauri.conf.json > plugins.updater.pubkey`。
- `mac-build.yml`（`mac-v*` 标签）只出**未签名、无更新能力**的 dmg 供快速分发，**不能自动更新**；
  要自动更新一律走 `release.yml`（`v*` 标签）这条线。
