#!/usr/bin/env node
// ─────────────────────────────────────────────────────────────────────
// minimax-tts.mjs — Polaris MiniMax T2A synthesizer (no bash / jq / mmx)
//
// Replaces ConardLi's mmx-cli based minimax provider. Auto-discovers the
// MiniMax key from the Polaris provider store (or env), calls the MiniMax
// T2A v2 HTTP API, decodes the hex audio payload, writes an mp3. Verified:
// the Polaris "粉丝福利" sk-cp- key authenticates T2A and needs NO GroupId.
//
// Usage:
//   node minimax-tts.mjs --one "<text>" <out.mp3> [voice]   single segment
//   node minimax-tts.mjs --batch [--force]                  read audio-segments.json
//
// Key discovery order:
//   1. env MINIMAX_API_KEY
//   2. ~/Polaris/data/providers.json  (the "minimax" provider's token)
//
// Tunables via env:
//   MINIMAX_TTS_MODEL  (default speech-02-turbo; speech-02-hd = higher quality)
//   MINIMAX_TTS_VOICE  (default male-qn-qingse)
//   MINIMAX_T2A_URL    (default https://api.minimaxi.com/v1/t2a_v2)
// ─────────────────────────────────────────────────────────────────────
import fs from "node:fs";
import path from "node:path";
import os from "node:os";

const ENDPOINT = process.env.MINIMAX_T2A_URL || "https://api.minimaxi.com/v1/t2a_v2";
const MODEL = process.env.MINIMAX_TTS_MODEL || "speech-02-turbo";
const DEFAULT_VOICE = process.env.MINIMAX_TTS_VOICE || "male-qn-qingse";
// 配音语言增强（粤语/英语/日语…）。对齐 MiniMax T2A v2 language_boost 取值，
// 如 "Chinese"、"Chinese,Yue"、"English"、"Japanese"、"auto"。
// 每段可在 audio-segments.json 用 language_boost 字段覆盖此默认值。
const DEFAULT_LANGUAGE_BOOST = (process.env.MINIMAX_LANGUAGE_BOOST || "").trim();

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

async function synth(text, outPath, voice, languageBoost) {
  const key = discoverKey();
  if (!key) {
    throw new Error(
      "找不到 MiniMax key：请在 Polaris 供应商坞启用「MiniMax」，或设置环境变量 MINIMAX_API_KEY",
    );
  }
  const body = {
    model: MODEL,
    text,
    stream: false,
    voice_setting: { voice_id: voice || DEFAULT_VOICE, speed: 1, vol: 1, pitch: 0 },
    audio_setting: { sample_rate: 32000, bitrate: 128000, format: "mp3" },
  };
  const boost = (languageBoost || DEFAULT_LANGUAGE_BOOST).trim();
  if (boost) body.language_boost = boost; // 提升目标语言（粤语/英语等）的发音准确度
  const res = await fetch(ENDPOINT, {
    method: "POST",
    headers: { Authorization: `Bearer ${key}`, "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const json = await res.json();
  const hex = json && json.data && json.data.audio;
  if (!hex) {
    const msg =
      (json && json.base_resp && json.base_resp.status_msg) ||
      JSON.stringify(json).slice(0, 300);
    throw new Error(`T2A 无音频返回: ${msg}`);
  }
  fs.mkdirSync(path.dirname(outPath), { recursive: true });
  fs.writeFileSync(outPath, Buffer.from(hex, "hex"));
  return outPath;
}

async function main() {
  const args = process.argv.slice(2);
  if (args[0] === "--one") {
    const [, text, out, voice, languageBoost] = args;
    if (!text || !out) {
      console.error('usage: node minimax-tts.mjs --one "<text>" <out.mp3> [voice] [language_boost]');
      process.exit(1);
    }
    const p = await synth(text, out, voice, languageBoost);
    console.log(`✓ ${p} (${fs.statSync(p).size} bytes)`);
    return;
  }
  if (args[0] === "--batch") {
    const root = process.cwd();
    const segFile = path.join(root, "audio-segments.json");
    const outDir = path.join(root, "public", "audio");
    const force = args.includes("--force");
    if (!fs.existsSync(segFile)) {
      console.error(`✗ ${segFile} 不存在。先跑：npm run extract-narrations`);
      process.exit(1);
    }
    const segs = JSON.parse(fs.readFileSync(segFile, "utf8"));
    let ok = 0,
      skip = 0,
      fail = 0;
    for (let i = 0; i < segs.length; i++) {
      const s = segs[i];
      const out = path.join(outDir, s.chapter, `${s.step}.mp3`);
      const tag = `${s.chapter}/${s.step}.mp3`;
      if (!s.text || !s.text.trim()) {
        skip++;
        continue;
      }
      if (fs.existsSync(out) && !force) {
        skip++;
        console.log(`[${i + 1}/${segs.length}] ${tag} skip (exists)`);
        continue;
      }
      try {
        await synth(s.text, out, s.voice, s.language_boost);
        ok++;
        console.log(`[${i + 1}/${segs.length}] ${tag} ✓`);
      } catch (e) {
        fail++;
        console.error(`[${i + 1}/${segs.length}] ${tag} ✗ ${e.message}`);
      }
    }
    console.log(`\n✓ done — synth ${ok}, skip ${skip}, fail ${fail}`);
    if (fail) process.exit(2);
    return;
  }
  console.error(
    "usage: node minimax-tts.mjs --one <text> <out.mp3> [voice] | --batch [--force]",
  );
  process.exit(1);
}

main().catch((e) => {
  console.error(`✗ ${e.message ?? e}`);
  process.exit(1);
});
