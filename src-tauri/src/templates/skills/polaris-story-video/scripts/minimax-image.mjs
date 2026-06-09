#!/usr/bin/env node
// ─────────────────────────────────────────────────────────────────────
// minimax-image.mjs — Polaris MiniMax 文生图 (image-01)
//
// 给「故事视频」生成高清的人物 / 环境插画。和 minimax-tts.mjs 同一个 key、
// 同一个域名 (api.minimaxi.com)，所以 Polaris「粉丝福利」MiniMax key 直接可用，
// 无需 GroupId。支持：
//   · aspect_ratio / HD 宽高 (竖屏 9:16、横屏 16:9、方图 1:1…)
//   · subject_reference 人物参考图 —— 让同一角色跨分镜长相一致
//   · response_format=url，下载落盘成 png
//
// 用法:
//   node minimax-image.mjs --one "<画面描述>" <out.png> [aspect] [refImage.png]
//   node minimax-image.mjs --batch [--storyboard=storyboard.json] [--force]
//
// --batch 流程 (读 storyboard.json，路径相对 storyboard 所在目录):
//   1. 先按 characters[].prompt 生成每个角色的「设定图」→ characters[].ref
//   2. 再按 shots[].image_prompt(+ 全局 style 风格后缀)逐镜生图 → shots[].image
//      若该镜 characters[0] 有设定图，则作为 subject_reference 保证人物一致
//
// Key 发现顺序 (同 tts):
//   1. env MINIMAX_API_KEY
//   2. ~/Polaris/data/providers.json  ("minimax" 供应商的 token)
//
// 可调 env:
//   MINIMAX_IMAGE_URL   (默认 https://api.minimaxi.com/v1/image_generation)
//   MINIMAX_IMAGE_MODEL (默认 image-01)
// ─────────────────────────────────────────────────────────────────────
import fs from "node:fs";
import path from "node:path";
import os from "node:os";

const ENDPOINT = process.env.MINIMAX_IMAGE_URL || "https://api.minimaxi.com/v1/image_generation";
const MODEL = process.env.MINIMAX_IMAGE_MODEL || "image-01";

// 比例 → HD 宽高 (范围 [512,2048]，须被 8 整除)。生成比画布略大，给 ken-burns 运镜留余量。
const DIMS = {
  "9:16": [1152, 2048],
  "16:9": [2048, 1152],
  "1:1": [1536, 1536],
  "4:3": [2048, 1536],
  "3:4": [1536, 2048],
  "3:2": [2048, 1360],
  "2:3": [1360, 2048],
};

function resolveHome(p) {
  if (!p) return p;
  if (p.startsWith("~/")) return path.join(os.homedir(), p.slice(2));
  return p;
}

function discoverKey() {
  if (process.env.MINIMAX_API_KEY) return process.env.MINIMAX_API_KEY.trim();
  const pj = path.join(os.homedir(), "Polaris", "data", "providers.json");
  try {
    const store = JSON.parse(fs.readFileSync(pj, "utf8"));
    const mm = (store.items || []).find(
      (p) => p.id === "minimax" || /minimax/i.test(p.name || ""),
    );
    if (mm) {
      const env = (mm.settings_config && mm.settings_config.env) || {};
      const k = env.ANTHROPIC_AUTH_TOKEN || env.ANTHROPIC_API_KEY || env.MINIMAX_API_KEY;
      if (k) return k.trim();
    }
  } catch {}
  return null;
}

function dimsFor(aspect) {
  return DIMS[aspect] || DIMS["9:16"];
}

// 把本地图片读成 data URI（subject_reference 接受 base64 或 url，base64 最省事、不依赖外链托管）
function toDataUri(imgPath) {
  const abs = resolveHome(imgPath);
  const buf = fs.readFileSync(abs);
  const ext = path.extname(abs).toLowerCase();
  const mime = ext === ".png" ? "image/png" : ext === ".webp" ? "image/webp" : "image/jpeg";
  return `data:${mime};base64,${buf.toString("base64")}`;
}

async function genImage(prompt, outPath, opts = {}) {
  const key = discoverKey();
  if (!key) {
    throw new Error(
      "找不到 MiniMax key：请在 Polaris 供应商坞启用「MiniMax」，或设置环境变量 MINIMAX_API_KEY",
    );
  }
  const [w, h] = dimsFor(opts.aspect || "9:16");
  const body = {
    model: MODEL,
    prompt: String(prompt || "").slice(0, 1500),
    width: w,
    height: h,
    response_format: "url",
    n: 1,
    prompt_optimizer: true,
  };
  // 人物一致性：把角色设定图作为参考主体。失败(读不到/不支持)就静默退化为纯文本生图。
  if (opts.refImage) {
    try {
      body.subject_reference = [{ type: "character", image_file: toDataUri(opts.refImage) }];
    } catch (e) {
      console.warn(`  ! 角色参考图读取失败，本镜改纯文本生图: ${e.message}`);
    }
  }

  const res = await fetch(ENDPOINT, {
    method: "POST",
    headers: { Authorization: `Bearer ${key}`, "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const json = await res.json();
  const data = json && json.data;
  const url = data && ((data.image_urls && data.image_urls[0]) || (data.image_url));
  const b64 = data && data.image_base64 && data.image_base64[0];
  if (!url && !b64) {
    const msg =
      (json && json.base_resp && json.base_resp.status_msg) ||
      JSON.stringify(json).slice(0, 300);
    throw new Error(`生图无结果: ${msg}`);
  }

  fs.mkdirSync(path.dirname(outPath), { recursive: true });
  if (b64) {
    fs.writeFileSync(outPath, Buffer.from(b64, "base64"));
  } else {
    const img = await fetch(url);
    if (!img.ok) throw new Error(`下载图片失败 HTTP ${img.status}`);
    const arr = Buffer.from(await img.arrayBuffer());
    fs.writeFileSync(outPath, arr);
  }
  return outPath;
}

async function main() {
  const args = process.argv.slice(2);

  if (args[0] === "--one") {
    const [, prompt, out, aspect, refImage] = args;
    if (!prompt || !out) {
      console.error('usage: node minimax-image.mjs --one "<prompt>" <out.png> [aspect] [refImage]');
      process.exit(1);
    }
    const p = await genImage(prompt, resolveHome(out), { aspect: aspect || "9:16", refImage });
    console.log(`✓ ${p} (${fs.statSync(p).size} bytes)`);
    return;
  }

  if (args[0] === "--batch") {
    const force = args.includes("--force");
    const sbArg = args.find((a) => a.startsWith("--storyboard="));
    const sbPath = resolveHome(sbArg ? sbArg.slice(13) : "storyboard.json");
    if (!fs.existsSync(sbPath)) {
      console.error(`✗ ${sbPath} 不存在。`);
      process.exit(1);
    }
    const root = path.dirname(path.resolve(sbPath));
    const sb = JSON.parse(fs.readFileSync(sbPath, "utf8"));
    const aspect = sb.aspect || "9:16";
    const style = (sb.style || "").trim();
    const abs = (rel) => (path.isAbsolute(rel) ? rel : path.join(root, rel));

    let ok = 0,
      skip = 0,
      fail = 0;

    // 1) 角色设定图 —— 后续分镜靠它保证人物一致
    const chars = Array.isArray(sb.characters) ? sb.characters : [];
    const charRef = {}; // id → 绝对路径
    for (const c of chars) {
      if (!c || !c.id) continue;
      const rel = c.ref || `assets/characters/${c.id}.png`;
      const out = abs(rel);
      charRef[c.id] = out;
      if (fs.existsSync(out) && !force) {
        skip++;
        console.log(`[角色 ${c.id}] skip (exists)`);
        continue;
      }
      const prompt = [
        c.prompt || c.name || c.id,
        "角色设定图：单人，全身或半身居中，干净纯色背景，正面或四分之三视角，光照均匀，五官清晰，便于作为后续分镜的人物参考。",
        style,
      ]
        .filter(Boolean)
        .join("。");
      try {
        await genImage(prompt, out, { aspect: "2:3" }); // 角色卡用竖构图，主体更突出
        ok++;
        console.log(`[角色 ${c.id}] ✓ ${out}`);
      } catch (e) {
        fail++;
        console.error(`[角色 ${c.id}] ✗ ${e.message}`);
      }
    }

    // 2) 逐镜生图
    const shots = Array.isArray(sb.shots) ? sb.shots : [];
    for (let i = 0; i < shots.length; i++) {
      const s = shots[i];
      const rel = s.image || `assets/shots/${s.id ?? i + 1}.png`;
      const out = abs(rel);
      const tag = `镜 ${s.id ?? i + 1}/${shots.length}`;
      if (fs.existsSync(out) && !force) {
        skip++;
        console.log(`[${tag}] skip (exists)`);
        continue;
      }
      const prompt = [s.image_prompt || s.subtitle || s.narration || "", style]
        .filter(Boolean)
        .join("。");
      // 该镜的主角(若有设定图)作为参考主体
      const primary = Array.isArray(s.characters) && s.characters.length ? s.characters[0] : null;
      const refImage = primary && charRef[primary] && fs.existsSync(charRef[primary]) ? charRef[primary] : null;
      try {
        await genImage(prompt, out, { aspect, refImage });
        ok++;
        console.log(`[${tag}] ✓ ${out}${refImage ? ` (参考角色 ${primary})` : ""}`);
      } catch (e) {
        fail++;
        console.error(`[${tag}] ✗ ${e.message}`);
      }
    }

    console.log(`\n✓ 生图完成 — 成功 ${ok}，跳过 ${skip}，失败 ${fail}`);
    if (fail) process.exit(2);
    return;
  }

  console.error(
    'usage: node minimax-image.mjs --one "<prompt>" <out.png> [aspect] [refImage] | --batch [--storyboard=path] [--force]',
  );
  process.exit(1);
}

main().catch((e) => {
  console.error(`✗ ${e.message ?? e}`);
  process.exit(1);
});
