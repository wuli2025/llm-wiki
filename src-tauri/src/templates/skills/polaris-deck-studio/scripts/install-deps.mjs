#!/usr/bin/env node
/* polaris-deck-studio :: install-deps.mjs
 *
 * Installs the (optional) PPTX-export toolchain into THIS skill folder:
 *   - playwright (chromium)  → headless screenshots of each slide
 *   - pptxgenjs              → assembles a .pptx with one full-bleed image per slide
 *
 * Idempotent + best-effort. If it fails (offline / no npm), the HTML deck still works
 * and you can fall back to "print → PDF" (Ctrl+P) or the python-pptx `pptx` skill.
 *
 * Usage:  node install-deps.mjs
 */
import { execSync } from "node:child_process";
import { existsSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));

function run(cmd, opts = {}) {
  console.log("→ " + cmd);
  execSync(cmd, { stdio: "inherit", cwd: here, ...opts });
}

try {
  // Local package.json so deps land inside the skill (not polluting the cwd project).
  const pkg = join(here, "package.json");
  if (!existsSync(pkg)) {
    mkdirSync(here, { recursive: true });
    writeFileSync(pkg, JSON.stringify({ name: "polaris-deck-export", private: true, type: "module" }, null, 2));
  }

  const haveNodeModules = existsSync(join(here, "node_modules", "pptxgenjs")) &&
    existsSync(join(here, "node_modules", "playwright"));
  if (!haveNodeModules) {
    run("npm install pptxgenjs playwright --no-audit --no-fund --loglevel=error");
  } else {
    console.log("✓ node deps already present");
  }

  // Chromium browser binary for playwright (skip if already installed).
  try {
    run("npx --yes playwright install chromium");
  } catch (e) {
    console.warn("⚠ playwright chromium 安装失败，可改用「打印为 PDF」兜底：", e.message);
  }

  console.log("✓ deck-studio 导出工具就绪");
} catch (e) {
  console.error("✗ 依赖安装失败（HTML 演示不受影响，可用打印 PDF 兜底）：", e.message);
  process.exit(1);
}
