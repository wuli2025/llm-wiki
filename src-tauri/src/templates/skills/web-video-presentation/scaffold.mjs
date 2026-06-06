#!/usr/bin/env node
// ─────────────────────────────────────────────────────────────────────
// scaffold.mjs — cross-platform (Windows-friendly) Node port of the
// ConardLi web-video-presentation scaffold.sh. No bash / WSL needed.
//
// Creates a Vite + React + TS presentation project, copies the skill's
// stage primitives + the chosen theme, and wires the audio pipeline to
// the Polaris MiniMax synthesizer (node scripts/minimax-tts.mjs).
//
// Usage:
//   node <pkg>/polaris/scaffold.mjs <target-dir> [--theme=<id>]
//   node <pkg>/polaris/scaffold.mjs --list-themes
// ─────────────────────────────────────────────────────────────────────
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { execSync } from "node:child_process";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const PKG = path.resolve(__dirname, ".."); // skill package root
const TEMPLATES = path.join(PKG, "templates");
const THEMES = path.join(PKG, "themes");
const DEFAULT_THEME = "midnight-press";

function readField(json, key) {
  const m = json.match(new RegExp(`"${key}"\\s*:\\s*"([^"]+)"`));
  return m ? m[1] : "";
}

function listThemes() {
  console.log(`可用主题（来自 ${THEMES}）:\n`);
  for (const id of fs.readdirSync(THEMES)) {
    const meta = path.join(THEMES, id, "theme.json");
    if (!fs.existsSync(meta)) continue;
    const j = fs.readFileSync(meta, "utf8");
    console.log(`  • ${id.padEnd(18)} ${readField(j, "nameZh")}`);
    console.log(`      ${readField(j, "descriptionZh")}\n`);
  }
  console.log(`用 --theme=<id> 选定一个。默认：${DEFAULT_THEME}。`);
}

function run(cmd, cwd) {
  // NB: do NOT use the spawn `cwd` option — on Windows, execSync({shell:true,
  // cwd}) intermittently fails with "spawnSync cmd.exe ENOENT". Embedding a
  // `cd` in the command (same shell invocation that works without cwd) is
  // the portable workaround.
  if (cwd) {
    const prefix = process.platform === "win32" ? `cd /d "${cwd}" && ` : `cd "${cwd}" && `;
    cmd = prefix + cmd;
  }
  execSync(cmd, { stdio: "inherit", shell: true });
}

function main() {
  const args = process.argv.slice(2);
  if (args.includes("--list-themes")) {
    listThemes();
    return;
  }
  let target = "";
  let theme = DEFAULT_THEME;
  for (const a of args) {
    if (a.startsWith("--theme=")) theme = a.slice("--theme=".length);
    else if (a.startsWith("--")) {
      console.error(`✗ 未知参数: ${a}`);
      process.exit(1);
    } else if (!target) target = a;
  }
  target = target || "presentation";
  // Resolve to absolute for our own fs ops, but feed create-vite a BARE
  // basename run from the parent dir — passing an absolute/relative path with
  // a drive colon ("D:/...") makes Windows cmd drop the colon and create a
  // bogus nested tree. basename has no colon/slash, so it's safe everywhere.
  const targetAbs = path.resolve(target);
  const parentDir = path.dirname(targetAbs);
  const baseName = path.basename(targetAbs);

  const themeTokens = path.join(THEMES, theme, "tokens.css");
  if (!fs.existsSync(themeTokens)) {
    console.error(`✗ 找不到主题 '${theme}'。用 --list-themes 看可用主题。`);
    process.exit(1);
  }
  if (fs.existsSync(targetAbs) && fs.readdirSync(targetAbs).length) {
    console.error(`✗ 目标目录 '${targetAbs}' 已存在且非空，已中止。`);
    process.exit(1);
  }
  fs.mkdirSync(parentDir, { recursive: true });

  console.log(`▸ 在 ${targetAbs} 创建 Vite + React + TS 项目（主题：${theme}）`);
  run(`npm create vite@latest "${baseName}" -- --template react-ts`, parentDir);

  console.log("▸ 安装依赖（可能要等一会）...");
  run("npm install", targetAbs);
  console.log("▸ 安装 tsx（extract-narrations 用）...");
  run("npm install --save-dev tsx", targetAbs);

  console.log("▸ 用演示骨架替换默认 boilerplate");
  const rm = (p) => fs.rmSync(path.join(targetAbs, p), { recursive: true, force: true });
  ["src/App.tsx", "src/App.css", "src/main.tsx", "src/index.css",
   "src/assets/react.svg", "public/vite.svg", "README.md", "src/assets"].forEach(rm);

  const mk = (p) => fs.mkdirSync(path.join(targetAbs, p), { recursive: true });
  ["src/styles", "src/hooks", "src/components", "src/registry",
   "src/chapters/01-example", "public", "scripts/tts-providers"].forEach(mk);

  const cp = (from, to) =>
    fs.cpSync(path.join(TEMPLATES, from), path.join(targetAbs, to), { recursive: true });

  cp("vite.config.ts", "vite.config.ts");
  cp("index.html", "index.html");
  cp("src/main.tsx", "src/main.tsx");
  cp("src/App.tsx", "src/App.tsx");
  fs.cpSync(themeTokens, path.join(target, "src/styles/tokens.css"));
  cp("src/styles/base.css", "src/styles/base.css");
  cp("src/styles/animations.css", "src/styles/animations.css");
  cp("src/styles/fonts.css", "src/styles/fonts.css");
  ["useStageScale.ts", "useStepper.ts", "useAudioPlayer.ts", "useAutoMode.ts"]
    .forEach((f) => cp(`src/hooks/${f}`, `src/hooks/${f}`));
  ["Stage.tsx", "MaskReveal.tsx", "ProgressBar.tsx", "ProgressBar.css",
   "AutoStartGate.tsx", "AutoStartGate.css", "AutoToggle.tsx", "AutoToggle.css"]
    .forEach((f) => cp(`src/components/${f}`, `src/components/${f}`));
  cp("src/registry/types.ts", "src/registry/types.ts");
  cp("src/registry/chapters.ts", "src/registry/chapters.ts");
  cp("src/chapters/01-example/Example.tsx", "src/chapters/01-example/Example.tsx");
  cp("src/chapters/01-example/Example.css", "src/chapters/01-example/Example.css");
  cp("src/chapters/01-example/narrations.ts", "src/chapters/01-example/narrations.ts");

  // Audio pipeline: extract via tsx, synthesize via Polaris MiniMax (node).
  cp("scripts/extract-narrations.ts", "scripts/extract-narrations.ts");
  fs.cpSync(path.join(__dirname, "minimax-tts.mjs"),
            path.join(targetAbs, "scripts/minimax-tts.mjs"));
  // Keep ConardLi's bash runner + providers for mac/linux users who prefer it.
  try {
    cp("scripts/synthesize-audio.sh", "scripts/synthesize-audio.sh");
    cp("scripts/tts-providers/README.md", "scripts/tts-providers/README.md");
    cp("scripts/tts-providers/minimax.sh", "scripts/tts-providers/minimax.sh");
    cp("scripts/tts-providers/openai.sh", "scripts/tts-providers/openai.sh");
  } catch {}

  // Wire npm scripts.
  const pkgPath = path.join(targetAbs, "package.json");
  const pkg = JSON.parse(fs.readFileSync(pkgPath, "utf8"));
  pkg.scripts = Object.assign({}, pkg.scripts, {
    "extract-narrations": "tsx scripts/extract-narrations.ts",
    "synthesize-audio": "node scripts/minimax-tts.mjs --batch",
  });
  fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");
  fs.writeFileSync(path.join(targetAbs, ".theme"), theme + "\n");

  console.log("▸ 跑 typecheck ...");
  run("npx tsc --noEmit", targetAbs);

  console.log(`
✓ 完成。下一步：

  1. cd ${targetAbs}
  2. npm run dev            # 打开 http://localhost:5173/5174

音频（用 Polaris 内置 MiniMax，自动取 key，无需 mmx / 登录）：
  npm run extract-narrations   # 扫 narrations.ts → audio-segments.json
  npm run synthesize-audio     # 调 MiniMax T2A → public/audio/<id>/<step>.mp3

录屏：打开 http://localhost:5173/?auto=1 → 按 SPACE → 整片自动播完 → 停录。
当前主题：${theme}（见 .theme）
`);
}

main();
