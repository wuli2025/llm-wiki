#!/usr/bin/env node
// ─────────────────────────────────────────────────────────────────────
// install-deps.mjs — 故事视频依赖自检
//
// 故事视频「不需要浏览器/Playwright」—— 只用 Node + ffmpeg。
//   · Node ≥ 18   (Polaris 内置)
//   · ffmpeg / ffprobe  (运镜 + 字幕 + 混音 + 时长探测)
//   · MiniMax key (生图 image-01 + 配音 T2A，Polaris 供应商坞自动提供)
// ─────────────────────────────────────────────────────────────────────
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";

function has(bin, arg = "-version") {
  try {
    const r = spawnSync(bin, [arg], { encoding: "utf8" });
    return r.status === 0 || /version/i.test((r.stdout || "") + (r.stderr || ""));
  } catch {
    return false;
  }
}

function hasMiniMaxKey() {
  if (process.env.MINIMAX_API_KEY) return true;
  const pj = path.join(os.homedir(), "Polaris", "data", "providers.json");
  try {
    const store = JSON.parse(fs.readFileSync(pj, "utf8"));
    const mm = (store.items || []).find((p) => p.id === "minimax" || /minimax/i.test(p.name || ""));
    if (mm) {
      const env = (mm.settings_config && mm.settings_config.env) || {};
      return !!(env.ANTHROPIC_AUTH_TOKEN || env.ANTHROPIC_API_KEY || env.MINIMAX_API_KEY);
    }
  } catch {}
  return false;
}

console.log("故事视频 · 依赖自检\n");

const nodeOk = (() => {
  const major = parseInt(process.versions.node.split(".")[0], 10);
  return major >= 18;
})();
console.log(`${nodeOk ? "✓" : "✗"} Node.js ${process.versions.node}${nodeOk ? "" : "  (需 ≥ 18)"}`);

const ffmpegOk = has("ffmpeg");
const ffprobeOk = has("ffprobe");
console.log(`${ffmpegOk ? "✓" : "✗"} ffmpeg`);
console.log(`${ffprobeOk ? "✓" : "✗"} ffprobe`);

const keyOk = hasMiniMaxKey();
console.log(`${keyOk ? "✓" : "✗"} MiniMax key (生图 + 配音)`);

if (!ffmpegOk || !ffprobeOk) {
  console.log(`
缺 ffmpeg/ffprobe。安装方式：
  · Windows : winget install Gyan.FFmpeg   (或 choco install ffmpeg / scoop install ffmpeg)
  · macOS   : brew install ffmpeg
  · Linux   : sudo apt install ffmpeg  /  sudo dnf install ffmpeg
装好后重开终端，或设环境变量 FFMPEG / FFPROBE 指向可执行文件。`);
}
if (!keyOk) {
  console.log(`
没探到 MiniMax key。请到 Polaris「API 供应商」启用「MiniMax」，
或设环境变量 MINIMAX_API_KEY。生图(image-01)与配音(T2A)共用这一个 key。`);
}

const allOk = nodeOk && ffmpegOk && ffprobeOk && keyOk;
console.log(`\n${allOk ? "✅ 依赖齐全，可以出片。" : "⚠️ 按上面补齐后再出片。"}`);
process.exit(allOk ? 0 : 1);
