#!/usr/bin/env node
// ─────────────────────────────────────────────────────────────────────
// run.mjs — 故事视频「一键出片」编排
//
// 给定一份 storyboard.json，依次跑完：
//   1. 生图   minimax-image.mjs --batch   (角色设定图 → 逐镜高清图)
//   2. 配音   minimax-tts.mjs   --batch   (逐镜旁白 mp3)
//   3. 合成   compose.mjs                 (运镜 + 字幕 + BGM → MP4)
//
// 用法:
//   node run.mjs --storyboard=storyboard.json --output=~/Desktop/story.mp4 [--force]
//
// storyboard.json 由 AI 在「规划」阶段产出 (schema 见 references/STORYBOARD.md)。
// ─────────────────────────────────────────────────────────────────────
import path from "node:path";
import os from "node:os";
import fs from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

function resolveHome(p) {
  if (!p) return p;
  if (p.startsWith("~/")) return path.join(os.homedir(), p.slice(2));
  return p;
}

function step(name, bin, args) {
  console.log(`\n━━━ ${name} ━━━`);
  const r = spawnSync(bin, args, { stdio: "inherit" });
  if (r.status !== 0) {
    console.error(`✗ ${name} 失败 (退出码 ${r.status ?? "?"})`);
    process.exit(r.status || 1);
  }
}

function main() {
  const args = process.argv.slice(2);
  const get = (k, d) => {
    const a = args.find((x) => x.startsWith(`--${k}=`));
    return a ? a.slice(k.length + 3) : d;
  };
  const sb = resolveHome(get("storyboard", "storyboard.json"));
  if (!fs.existsSync(sb)) {
    console.error(`✗ storyboard 不存在: ${sb}`);
    console.error("请先在「规划」阶段产出 storyboard.json（schema 见 references/STORYBOARD.md）。");
    process.exit(1);
  }
  const output = resolveHome(get("output", "~/Desktop/story.mp4"));
  const force = args.includes("--force") ? ["--force"] : [];

  console.log(`
╔══════════════════════════════════════════╗
║   Polaris 故事视频 · 一键出片            ║
╚══════════════════════════════════════════╝
storyboard: ${sb}
output:     ${output}`);

  const img = path.join(__dirname, "minimax-image.mjs");
  const tts = path.join(__dirname, "minimax-tts.mjs");
  const compose = path.join(__dirname, "compose.mjs");

  step("1/3 生图（人物 + 环境，高清）", process.execPath, [img, "--batch", `--storyboard=${sb}`, ...force]);
  step("2/3 配音（逐镜旁白）", process.execPath, [tts, "--batch", `--storyboard=${sb}`, ...force]);
  step("3/3 合成（运镜 + 字幕 + BGM）", process.execPath, [
    compose,
    `--storyboard=${sb}`,
    `--output=${output}`,
  ]);

  console.log(`\n✅ 全部完成 → ${output}`);
}

main();
