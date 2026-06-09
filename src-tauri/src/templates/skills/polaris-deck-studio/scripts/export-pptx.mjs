#!/usr/bin/env node
/* polaris-deck-studio :: export-pptx.mjs
 *
 * Render a deck.html to a .pptx where each slide is a full-bleed, pixel-perfect image —
 * so the PPTX looks EXACTLY like the themed HTML deck (text is not editable; that's the
 * trade-off for visual fidelity). Uses playwright (chromium) + pptxgenjs (pure Node).
 *
 * Usage:
 *   node export-pptx.mjs --deck="C:/path/deck.html" --out="C:/path/演示.pptx" [--width=1920] [--height=1080]
 *
 * Notes:
 *   - Loads the deck once, then drives window.__deck.go(i) for each slide (deterministic).
 *   - Adds ?export=1 so runtime.js disables entrance animations → clean stills.
 *   - 16:9 by default (1920x1080 → LAYOUT_WIDE 13.333"x7.5").
 *   - Run scripts from THIS skill folder so it resolves the deps install-deps.mjs put here.
 */
import { pathToFileURL } from "node:url";
import { dirname, resolve, isAbsolute } from "node:path";
import { existsSync } from "node:fs";

function arg(name, def) {
  const hit = process.argv.find((a) => a.startsWith("--" + name + "="));
  return hit ? hit.slice(name.length + 3) : def;
}

const deckPath = arg("deck");
const outPath = arg("out");
const W = parseInt(arg("width", "1920"), 10);
const H = parseInt(arg("height", "1080"), 10);

if (!deckPath || !outPath) {
  console.error('用法: node export-pptx.mjs --deck="deck.html" --out="out.pptx" [--width=1920] [--height=1080]');
  process.exit(2);
}
if (!existsSync(deckPath)) {
  console.error("✗ 找不到 deck 文件: " + deckPath);
  process.exit(2);
}

let chromium, PptxGenJS;
try {
  ({ chromium } = await import("playwright"));
  PptxGenJS = (await import("pptxgenjs")).default;
} catch (e) {
  console.error("✗ 缺少导出依赖（playwright / pptxgenjs）。先运行: node install-deps.mjs");
  console.error("  或改用兜底：在浏览器打开 deck.html → Ctrl+P → 另存为 PDF。");
  process.exit(3);
}

const fileUrl = pathToFileURL(isAbsolute(deckPath) ? deckPath : resolve(process.cwd(), deckPath)).href;
const outAbs = isAbsolute(outPath) ? outPath : resolve(process.cwd(), outPath);

console.log("→ 启动 chromium…");
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: W, height: H }, deviceScaleFactor: 1 });

console.log("→ 载入 deck: " + fileUrl);
await page.goto(fileUrl + "?export=1", { waitUntil: "networkidle" });

// resolve slide count from the runtime
const total = await page.evaluate(() => (window.__deck && window.__deck.total) ||
  document.querySelectorAll(".slide").length);
if (!total) { console.error("✗ deck 里没有 .slide"); await browser.close(); process.exit(4); }
console.log("→ 共 " + total + " 页，开始逐页截图…");

const pptx = new PptxGenJS();
pptx.defineLayout({ name: "DECK", width: 13.333, height: 7.5 });
pptx.layout = "DECK";

for (let i = 0; i < total; i++) {
  await page.evaluate((n) => window.__deck.go(n), i);
  await page.waitForTimeout(260); // let layout settle (animations are off in export mode)
  const buf = await page.screenshot({ type: "png" });
  const dataUrl = "data:image/png;base64," + buf.toString("base64");
  const slide = pptx.addSlide();
  slide.addImage({ data: dataUrl, x: 0, y: 0, w: 13.333, h: 7.5 });
  process.stdout.write("  · " + (i + 1) + "/" + total + "\r");
}

await browser.close();
await pptx.writeFile({ fileName: outAbs });
console.log("\n✓ 已生成 PPTX: " + outAbs);
