#!/usr/bin/env node
// ─────────────────────────────────────────────────────────────────────
// bootstrap.mjs — one-shot dependency check for the web-video-presentation
// skill. Run automatically right after the skill is installed so the user
// learns up-front what (if anything) is missing. Node + npm are required;
// ffmpeg is optional (only for duration validation). Per-project npm deps
// are installed later by scaffold.mjs (`npm install`).
// ─────────────────────────────────────────────────────────────────────
import { execSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";

function has(cmd) {
  try {
    execSync(process.platform === "win32" ? `where ${cmd}` : `command -v ${cmd}`, {
      stdio: "ignore",
      shell: true,
    });
    return true;
  } catch {
    return false;
  }
}

function keyOk() {
  if (process.env.MINIMAX_API_KEY) return true;
  try {
    const store = JSON.parse(
      fs.readFileSync(path.join(os.homedir(), "Polaris", "data", "providers.json"), "utf8"),
    );
    return (store.items || []).some((p) => {
      const env = (p.settings_config && p.settings_config.env) || {};
      return (
        (p.id === "minimax" || /minimax/i.test(p.name || "")) &&
        (env.ANTHROPIC_AUTH_TOKEN || env.ANTHROPIC_API_KEY)
      );
    });
  } catch {
    return false;
  }
}

const node = has("node");
const npm = has("npm");
const ffmpeg = has("ffmpeg") && has("ffprobe");
const mmKey = keyOk();

console.log("网页演示视频 · 依赖自检\n");
console.log(`  node            ${node ? "✓" : "✗ 必需，请安装 Node.js 18+"}`);
console.log(`  npm             ${npm ? "✓" : "✗ 必需，随 Node 一起装"}`);
console.log(`  ffmpeg/ffprobe  ${ffmpeg ? "✓" : "○ 可选（只用于音频时长校验）"}`);
console.log(`  MiniMax key     ${mmKey ? "✓ 已就绪（配音将自动调用 MiniMax）" : "○ 未配置（在供应商坞启用 MiniMax 即可配音）"}`);

if (!ffmpeg) {
  console.log(
    "\n装 ffmpeg（可选）：\n  Windows: winget install Gyan.FFmpeg\n  macOS:   brew install ffmpeg\n  Linux:   apt-get install ffmpeg",
  );
}
console.log("\n项目级 npm 依赖会在脚手架阶段自动安装（scaffold 内含 npm install）。");

if (!node || !npm) process.exit(1);
console.log("\n✓ 核心依赖就绪，可以开始：让 Polaris 帮你跑脚手架并逐章开发。");
