# Polaris Docker 冒烟测试：健康/静态/REST/真实对话全链路验证。
# 用法：pwsh scripts/docker-smoke.ps1
$ErrorActionPreference = "Stop"
$base = "http://localhost:8080"

function Invoke-Cmd($cmd, $argsObj) {
  $body = @{ cmd = $cmd; args = $argsObj } | ConvertTo-Json -Depth 10 -Compress
  return Invoke-RestMethod -Uri "$base/api/invoke" -Method Post -ContentType "application/json" -Body $body
}

Write-Host "1) /api/health" -ForegroundColor Cyan
(Invoke-WebRequest "$base/api/health").Content

Write-Host "`n2) 前端静态 /" -ForegroundColor Cyan
$idx = (Invoke-WebRequest "$base/").Content
if ($idx -match "<div id=`"app`"|<title") { "index.html OK ($($idx.Length) bytes)" } else { "WARN: 首页异常" }

Write-Host "`n3) kb_root" -ForegroundColor Cyan
Invoke-Cmd "kb_root" @{}

Write-Host "`n4) provider_list currentId" -ForegroundColor Cyan
(Invoke-Cmd "provider_list" @{}).currentId

Write-Host "`n5) env_check (claude 是否就绪)" -ForegroundColor Cyan
$env_ = Invoke-Cmd "env_check" @{}
"claude.found=$($env_.claude.found)  version=$($env_.claude.version)  ready=$($env_.ready)"

Write-Host "`n6) 建项目 + 会话" -ForegroundColor Cyan
$proj = Invoke-Cmd "conv_create_project" @{ name = "docker-smoke" }
$projId = $proj.id
$conv = Invoke-Cmd "conv_create_conversation" @{ projectId = $projId }
$cid = $conv.id
"projectId=$projId  conversationId=$cid"

Write-Host "`n7) chat_send（真实跑一轮对话）" -ForegroundColor Cyan
$reqId = Invoke-Cmd "chat_send" @{ args = @{ prompt = "用一句话自我介绍，并说出 1+1 等于几。"; permissionMode = "auto_all"; conversationId = $cid } }
"reqId=$reqId  等待回复..."

Write-Host "`n8) 轮询会话消息（最多 180s）" -ForegroundColor Cyan
$got = $false
for ($i = 0; $i -lt 60; $i++) {
  Start-Sleep -Seconds 3
  $msgs = Invoke-Cmd "conv_get_messages" @{ conversationId = $cid }
  $asst = @($msgs | Where-Object { $_.role -eq "assistant" })
  if ($asst.Count -gt 0 -and $asst[-1].content.Trim().Length -gt 0) {
    Write-Host "✅ 助手已回复：" -ForegroundColor Green
    $asst[-1].content.Substring(0, [Math]::Min(400, $asst[-1].content.Length))
    $got = $true
    break
  }
  Write-Host "  ...等待中 ($(($i+1)*3)s)"
}
if (-not $got) { Write-Host "❌ 超时未见助手回复" -ForegroundColor Red; exit 1 }
Write-Host "`n冒烟测试通过 ✅" -ForegroundColor Green
