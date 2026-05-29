#!/bin/bash
# ─────────────────────────────────────────────────────────────
# Polaris · macOS 首次启动助手
#
# 用途：Polaris 是未经 Apple 付费签名的应用，macOS 会给「下载来的」文件
#       打上「隔离」标记并拦截。本脚本一键解除该标记并启动 Polaris，
#       解除后以后就能正常双击打开了。
#
# 用法：
#   1. 先把 Polaris 从 .dmg 拖进「应用程序」文件夹；
#   2. 双击本文件（首次会让你在「系统设置 → 隐私与安全性」点一次「仍要打开」）。
# ─────────────────────────────────────────────────────────────
set -e

CANDIDATES=(
  "/Applications/Polaris.app"
  "$HOME/Applications/Polaris.app"
  "$(cd "$(dirname "$0")" && pwd)/Polaris.app"
)

APP=""
for c in "${CANDIDATES[@]}"; do
  if [ -d "$c" ]; then APP="$c"; break; fi
done

echo "================ Polaris 首次启动助手 ================"
if [ -z "$APP" ]; then
  echo "✗ 没找到 Polaris.app。"
  echo "  请先打开 Polaris 的 .dmg，把 Polaris 拖进「应用程序」文件夹，再双击本文件。"
  echo
  read -n 1 -s -r -p "按任意键关闭…"
  exit 1
fi

echo "找到：$APP"
echo "→ 解除「隔离」标记…"
xattr -dr com.apple.quarantine "$APP" 2>/dev/null || true
echo "→ 启动 Polaris…"
open "$APP"
echo "✓ 完成。以后直接双击「应用程序」里的 Polaris 即可打开。"
sleep 1
