#!/usr/bin/env node
// ─────────────────────────────────────────────────────────────────────
// minimax-tts.mjs — Polaris MiniMax 配音 (T2A v2)，故事视频版
//
// 和「视频工坊」那份同源，但 --batch 改读 storyboard.json 的 shots[]，
// 逐镜把 narration 合成 mp3 → shots[].audio，并支持每镜 voice / speed /
// language_boost。Polaris「粉丝福利」sk-cp- key 直接认 T2A，无需 GroupId。
//
// 用法:
//   node minimax-tts.mjs --one "<text>" <out.mp3> [voice] [speed] [language_boost]
//   node minimax-tts.mjs --batch [--storyboard=storyboard.json] [--force]
//
// Key 发现顺序: env MINIMAX_API_KEY → ~/Polaris/data/providers.json
// 可调 env: MINIMAX_TTS_MODEL(默认 speech-02-hd) / MINIMAX_T2A_URL / MINIMAX_LANGUAGE_BOOST
// ─────────────────────────────────────────────────────────────────────
import fs from "node:fs";
import path from "node:path";
import os from "node:os";

const ENDPOINT = process.env.MINIMAX_T2A_URL || "https://api.minimaxi.com/v1/t2a_v2";
const MODEL = process.env.MINIMAX_TTS_MODEL || "speech-02-hd";
const DEFAULT_VOICE = process.env.MINIMAX_TTS_VOICE || "audiobook_male_1";
const DEFAULT_LANGUAGE_BOOST = (process.env.MINIMAX_LANGUAGE_BOOST || "").trim();

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

async function synth(text, outPath, voice, speed, languageBoost) {
  const key = discoverKey();
  if (!key) {
    throw new Error(
      "找不到 MiniMax key：请在 Polaris 供应商坞启用「MiniMax」，或设置环境变量 MINIMAX_API_KEY",
    );
  }
  const sp = Number(speed);
  const body = {
    model: MODEL,
    text,
    stream: false,
    voice_setting: {
      voice_id: voice || DEFAULT_VOICE,
      speed: Number.isFinite(sp) && sp > 0 ? sp : 1,
      vol: 1,
      pitch: 0,
    },
    audio_setting: { sample_rate: 32000, bitrate: 128000, format: "mp3" },
  };
  const boost = (languageBoost || DEFAULT_LANGUAGE_BOOST).trim();
  if (boost) body.language_boost = boost;
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
    const [, text, out, voice, speed, languageBoost] = args;
    if (!text || !out) {
      console.error('usage: node minimax-tts.mjs --one "<text>" <out.mp3> [voice] [speed] [language_boost]');
      process.exit(1);
    }
    const p = await synth(text, resolveHome(out), voice, speed, languageBoost);
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
    const abs = (rel) => (path.isAbsolute(rel) ? rel : path.join(root, rel));
    const gVoice = sb.voice || DEFAULT_VOICE;
    const gSpeed = sb.speed || 1;
    const gBoost = sb.language_boost || "";
    const shots = Array.isArray(sb.shots) ? sb.shots : [];

    let ok = 0,
      skip = 0,
      fail = 0;
    for (let i = 0; i < shots.length; i++) {
      const s = shots[i];
      const rel = s.audio || `assets/audio/${s.id ?? i + 1}.mp3`;
      const out = abs(rel);
      const tag = `镜 ${s.id ?? i + 1}/${shots.length}`;
      const text = (s.narration || s.subtitle || "").trim();
      if (!text) {
        skip++;
        console.log(`[${tag}] skip (无旁白)`);
        continue;
      }
      if (fs.existsSync(out) && !force) {
        skip++;
        console.log(`[${tag}] skip (exists)`);
        continue;
      }
      try {
        await synth(text, out, s.voice || gVoice, s.speed || gSpeed, s.language_boost || gBoost);
        ok++;
        console.log(`[${tag}] ✓`);
      } catch (e) {
        fail++;
        console.error(`[${tag}] ✗ ${e.message}`);
      }
    }
    console.log(`\n✓ 配音完成 — 成功 ${ok}，跳过 ${skip}，失败 ${fail}`);
    if (fail) process.exit(2);
    return;
  }

  console.error(
    'usage: node minimax-tts.mjs --one "<text>" <out.mp3> [voice] [speed] [boost] | --batch [--storyboard=path] [--force]',
  );
  process.exit(1);
}

main().catch((e) => {
  console.error(`✗ ${e.message ?? e}`);
  process.exit(1);
});
