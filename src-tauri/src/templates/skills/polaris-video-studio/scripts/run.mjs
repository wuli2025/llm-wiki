#!/usr/bin/env node
/**
 * Polaris 视频工坊 · 一键执行脚本
 *
 * 用法:
 *   node run.mjs --input="文案.md" --theme=midnight-press --output="~/Desktop/output.mp4"
 *
 * 流程:
 *   1. 检查依赖
 *   2. 创建工作目录
 *   3. 复制/写入文案到 article.md
 *   4. 输出下一步指引（Phase 1-2 需 AI 执行，Phase 3-4 可脚本化）
 */
import fs from "fs";
import path from "path";
import os from "os";

function resolveHome(p) {
  if (p.startsWith("~/")) return path.join(os.homedir(), p.slice(2));
  return path.resolve(p);
}

function parseArgs() {
  const args = process.argv.slice(2);
  const out = { input: null, theme: "midnight-press", output: null, skipScaffold: false };
  for (const a of args) {
    if (a.startsWith("--input=")) out.input = a.slice(8);
    else if (a.startsWith("--theme=")) out.theme = a.slice(8);
    else if (a.startsWith("--output=")) out.output = a.slice(9);
    else if (a === "--skip-scaffold") out.skipScaffold = true;
  }
  return out;
}

function ensureDir(d) {
  if (!fs.existsSync(d)) fs.mkdirSync(d, { recursive: true });
}

function banner() {
  console.log(`
╔══════════════════════════════════════════╗
║     Polaris 视频工坊 · 一键出片          ║
╚══════════════════════════════════════════╝
`);
}

function main() {
  banner();
  const args = parseArgs();

  if (!args.input) {
    console.log("用法:");
    console.log("  node run.mjs --input=\"文案.md\" --theme=midnight-press --output=\"~/Desktop/output.mp4\"");
    console.log("");
    console.log("参数:");
    console.log("  --input        文案文件路径 (.md/.txt) 或直接文本");
    console.log("  --theme        视觉主题 (默认: midnight-press)");
    console.log("  --output       输出 MP4 路径 (默认: ~/Desktop/output.mp4)");
    console.log("  --skip-scaffold  如果项目已存在，跳过脚手架");
    process.exit(1);
  }

  const workDir = path.resolve("polaris-video-work");
  ensureDir(workDir);

  // 1. 写入 article.md
  const inputPath = resolveHome(args.input);
  let articleText;
  if (fs.existsSync(inputPath)) {
    articleText = fs.readFileSync(inputPath, "utf-8");
  } else {
    articleText = args.input; // 直接作为文本
  }
  fs.writeFileSync(path.join(workDir, "article.md"), articleText, "utf-8");
  console.log(`✓ article.md 已写入 (${articleText.length} 字符)`);

  // 2. 记录配置
  const config = {
    theme: args.theme,
    output: resolveHome(args.output || "~/Desktop/output.mp4"),
    workDir,
  };
  fs.writeFileSync(path.join(workDir, ".config.json"), JSON.stringify(config, null, 2));

  console.log(`✓ 工作目录: ${workDir}`);
  console.log(`✓ 主题: ${args.theme}`);
  console.log(`✓ 输出: ${config.output}`);
  console.log("");

  // 3. 输出 Phase 指引
  console.log("═══ 执行指引 ═══\n");
  console.log("Phase 1 · 内容（AI 执行）");
  console.log("  → 把 article.md 转成口播稿 script.md + 开发计划 outline.md");
  console.log("  → 调用: 在对话中让 AI 按 SKILL.md 的 Phase 1 执行\n");

  console.log("Phase 2 · 开发（AI 执行）");
  console.log("  → 脚手架 + 逐章开发网页演示");
  console.log("  → 调用: 在对话中让 AI 按 SKILL.md 的 Phase 2 执行\n");

  console.log("Phase 3 · 配音（脚本自动）");
  console.log(`  cd ${path.join(workDir, "presentation")}`);
  console.log("  npm run extract-narrations");
  console.log("  npm run synthesize-audio\n");

  console.log("Phase 4 · 出片（脚本自动）");
  console.log("  node scripts/pipeline/03-record.mjs");
  console.log(`     --project=${path.join(workDir, "presentation")}`);
  console.log(`     --output=${config.output}\n`);

  console.log("═══ 快捷方式 ═══\n");
  console.log("如果你想全自动（跳过 AI 对话），请直接在 Polaris 里使用");
  console.log("「生成课件类视频」功能，它会自动走完所有 Phase。\n");
}

main();
