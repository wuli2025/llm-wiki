# Polaris Docker 长稳监控：默认 3h15m，每 10 分钟一轮。
# 每轮：健康检查 → 真实对话(验证 spawn/stream/kill 全链) → 记录内存/进程数/僵尸数。
# 输出累加到 .docker-monitor.log
param(
  [int]$Minutes = 195,
  [int]$IntervalSec = 600
)
$ErrorActionPreference = "Continue"
$base = "http://localhost:8080"
$log = "D:\polaris\polaris-app\.docker-monitor.log"
$deadline = (Get-Date).AddMinutes($Minutes)
$cycle = 0
$fails = 0

function Inv($cmd, $a) {
  $b = @{ cmd = $cmd; args = $a } | ConvertTo-Json -Depth 8 -Compress
  Invoke-RestMethod "$base/api/invoke" -Method Post -ContentType "application/json" -Body $b -TimeoutSec 30
}
function Log($m) { $line = "[{0}] {1}" -f (Get-Date -Format "HH:mm:ss"), $m; Write-Output $line; Add-Content $log $line }

Log "=== 监控开始 (时长 ${Minutes}m, 间隔 ${IntervalSec}s) ==="
# 建一个长期会话复用
$proj = Inv "conv_create_project" @{ name = "monitor" }
$conv = Inv "conv_create_conversation" @{ projectId = $proj.id }
$cid = $conv.id

while ((Get-Date) -lt $deadline) {
  $cycle++
  $ok = $true
  $detail = ""
  try {
    # 1) 健康
    $h = (Invoke-WebRequest "$base/api/health" -TimeoutSec 15).Content
    if ($h.Trim() -ne "ok") { $ok = $false; $detail += "health!=ok " }

    # 2) 真实对话（验证 spawn→stream→落库→进程回收）
    $before = (Inv "conv_get_messages" @{ conversationId = $cid } | Where-Object { $_.role -eq "assistant" }).Count
    $reqId = Inv "chat_send" @{ args = @{ prompt = "只回复两个字：在线"; permissionMode = "auto_all"; conversationId = $cid } }
    $replied = $false
    for ($i = 0; $i -lt 40; $i++) {
      Start-Sleep -Seconds 3
      $after = @(Inv "conv_get_messages" @{ conversationId = $cid } | Where-Object { $_.role -eq "assistant" })
      if ($after.Count -gt $before) { $replied = $true; break }
    }
    if (-not $replied) { $ok = $false; $detail += "chat-timeout " }

    # 3) 资源 + 进程 + 僵尸
    $mem = (docker stats polaris-web --no-stream --format "{{.MemUsage}}") -replace '\s',''
    $procs = (docker exec polaris-web sh -c "ls -d /proc/[0-9]* 2>/dev/null | wc -l").Trim()
    $zomb = (docker exec polaris-web sh -c "z=0; for p in /proc/[0-9]*/stat; do s=`$(awk '{print `$3}' `$p 2>/dev/null); [ `"`$s`" = Z ] && z=`$((z+1)); done; echo `$z").Trim()
    $detail += "mem=$mem procs=$procs zombies=$zomb"
    if ([int]$zomb -gt 0) { $ok = $false; $detail += " ZOMBIE!" }
  } catch {
    $ok = $false; $detail += "EXC:$($_.Exception.Message)"
  }
  if ($ok) { Log "cycle $cycle ✅ $detail" } else { $fails++; Log "cycle $cycle ❌ $detail" }
  Start-Sleep -Seconds $IntervalSec
}
Log "=== 监控结束: $cycle 轮, 失败 $fails 轮 ==="
if ($fails -gt 0) { exit 1 }
