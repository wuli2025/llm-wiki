#!/usr/bin/env node
/**
 * Polaris 视频工坊 · 依赖安装脚本
 * 检测 Node/npm/ffmpeg/Playwright/MiniMax key，缺什么引导装什么。
 * Windows/macOS/Linux 全平台。
 */
import { spawn } from "child_process";
import fs from "fs";
import os from "os";
import path from "path";

const MIN_NODE = 18;
const MIN_NPM = 9;

function run(cmd, args = []) {
  return new Promise((resolve) => {
    const p = spawn(cmd, args, { shell: true, stdio: "pipe" });
    let out = "";
    p.stdout.on("data", (d) => (out += d));
    p.stderr.on("data", (d) => (out += d));
    p.on("close", (code) => resolve(code === 0 ? out.trim() : null));
  });
}

async function checkNode() {
  const v = await run("node", ["--version"]);
  if (!v) return { ok: false, msg: "Node.js 未安装" };
  const major = parseInt(v.replace("v", "").split(".")[0]);
  if (major < MIN_NODE) return { ok: false, msg: `Node.js ${v} < ${MIN_NODE}` };
  return { ok: true, msg: `Node.js ${v}` };
}

async function checkNpm() {
  const v = await run("npm", ["--version"]);
  if (!v) return { ok: false, msg: "npm 未安装" };
  const major = parseInt(v.split(".")[0]);
  if (major < MIN_NPM) return { ok: false, msg: `npm ${v} < ${MIN_NPM}` };
  return { ok: true, msg: `npm ${v}` };
}

async function checkFfmpeg() {
  const v = await run("ffmpeg", ["-version"]);
  if (!v) return { ok: false, msg: "ffmpeg 未安装" };
  const line = v.split("\n")[0];
  return { ok: true, msg: line.slice(0, 60) };
}

async function checkPlaywright() {
  const v = await run("npx", ["playwright", "--version"]);
  if (!v) {
    // 检查全局安装
    const g = await run("playwright", ["--version"]);
    if (!g) return { ok: false, msg: "Playwright 未安装" };
    return { ok: true, msg: `Playwright ${g}` };
  }
  return { ok: true, msg: `Playwright ${v}` };
}

async function checkMiniMaxKey() {
  const key = process.env.MINIMAX_API_KEY || process.env.ANTHROPIC_AUTH_TOKEN;
  if (!key) return { ok: false, msg: "MINIMAX_API_KEY 未设置（Polaris 供应商坞可自动提供）" };
  return { ok: true, msg: `MiniMax key 已设置 (${key.slice(0, 8)}...)` };
}

async function main() {
  console.log("═══ Polaris 视频工坊 · 依赖检查 ═══\n");

  const checks = [
    { name: "Node.js", check: checkNode },
    { name: "npm", check: checkNpm },
    { name: "ffmpeg", check: checkFfmpeg },
    { name: "Playwright", check: checkPlaywright },
    { name: "MiniMax Key", check: checkMiniMaxKey },
  ];

  let allOk = true;
  for (const c of checks) {
    const r = await c.check();
    const icon = r.ok ? "✓" : "✗";
    console.log(`${icon} ${c.name.padEnd(12)} ${r.msg}`);
    if (!r.ok) allOk = false;
  }

  console.log("");
  if (allOk) {
    console.log("✓ 所有依赖已就绪，可以直接使用！");
    return;
  }

  console.log("═══ 安装指引 ═══\n");

  const platform = os.platform();

  // ffmpeg 安装指引
  const ffmpegOk = (await checkFfmpeg()).ok;
  if (!ffmpegOk) {
    console.log("【ffmpeg】");
    if (platform === "win32") {
      console.log("  1. 下载: https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip");
      console.log("  2. 解压到 C:\\ffmpeg，把 C:\\ffmpeg\\bin 加到 PATH");
      console.log("  3. 或用 winget: winget install Gyan.FFmpeg");
    } else if (platform === "darwin") {
      console.log("  brew install ffmpeg");
    } else {
      console.log("  sudo apt install ffmpeg    # Debian/Ubuntu");
      console.log("  sudo pacman -S ffmpeg      # Arch");
    }
    console.log("");
  }

  // Playwright 安装指引
  const pwOk = (await checkPlaywright()).ok;
  if (!pwOk) {
    console.log("【Playwright】");
    console.log("  npm install -g playwright");
    console.log("  npx playwright install chromium");
    console.log("");
  }

  // MiniMax key 指引
  const keyOk = (await checkMiniMaxKey()).ok;
  if (!keyOk) {
    console.log("【MiniMax API Key】");
    console.log("  方式 A: 在 Polaris 左下角「供应商坞」启用 MiniMax");
    console.log("  方式 B: 手动设置环境变量: $env:MINIMAX_API_KEY='your-key'");
    console.log("");
  }

  console.log("装完后再运行: node scripts/install-deps.mjs");
}

main();
