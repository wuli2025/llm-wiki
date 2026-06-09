#!/usr/bin/env node
// ─────────────────────────────────────────────────────────────────────
// compose.mjs — 故事视频合成器 (纯 ffmpeg，无需浏览器/录屏)
//
// 把 storyboard.json 里每个分镜的「高清图 + 旁白音频」合成一条 MP4：
//   · 每镜按音频时长做 Ken-Burns 运镜 (推近/拉远/横移/静止)
//   · 拼接所有镜头
//   · (可选) 按 subtitle 烧录字幕
//   · (可选) 循环铺底背景音乐并相对人声压低
// 输出 9:16 / 16:9 / 1:1 等画幅的成片。
//
// 用法:
//   node compose.mjs --storyboard=storyboard.json --output=~/Desktop/story.mp4
//
// 前置: 已跑过 minimax-image.mjs --batch (有分镜图) 与 minimax-tts.mjs --batch (有旁白)。
// 依赖: ffmpeg / ffprobe 在 PATH (或设 env FFMPEG / FFPROBE)。
// ─────────────────────────────────────────────────────────────────────
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { spawnSync } from "node:child_process";

const FFMPEG = process.env.FFMPEG || "ffmpeg";
const FFPROBE = process.env.FFPROBE || "ffprobe";
const FPS = 30;
const SUB_FONT = process.env.STORY_SUB_FONT || "Microsoft YaHei";

// 画幅 → 输出画布 [宽,高]
const CANVAS = {
  "9:16": [1080, 1920],
  "16:9": [1920, 1080],
  "1:1": [1080, 1080],
  "4:3": [1440, 1080],
  "3:4": [1080, 1440],
  "3:2": [1620, 1080],
  "2:3": [1080, 1620],
};

function resolveHome(p) {
  if (!p) return p;
  if (p.startsWith("~/")) return path.join(os.homedir(), p.slice(2));
  return p;
}

function run(bin, args, opts = {}) {
  const r = spawnSync(bin, args, { encoding: "utf8", ...opts });
  if (r.error) throw new Error(`${bin} 启动失败: ${r.error.message}`);
  if (r.status !== 0) {
    const tail = (r.stderr || "").split("\n").slice(-8).join("\n");
    throw new Error(`${bin} 退出码 ${r.status}\n${tail}`);
  }
  return r;
}

function probeDuration(file) {
  try {
    const r = spawnSync(
      FFPROBE,
      ["-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1", file],
      { encoding: "utf8" },
    );
    const d = parseFloat((r.stdout || "").trim());
    return Number.isFinite(d) && d > 0 ? d : 0;
  } catch {
    return 0;
  }
}

// 某镜的 Ken-Burns zoompan 表达式 (N = 该镜总帧数)
function kenburns(motion, N, W, H) {
  const cx = "x='iw/2-(iw/zoom/2)'";
  const cy = "y='ih/2-(ih/zoom/2)'";
  const zs = (0.4 / Math.max(N, 1)).toFixed(6);
  switch (motion) {
    case "zoom-out":
      return `zoompan=z='max(1.4-${zs}*on,1.0)':${cx}:${cy}:d=${N}:s=${W}x${H}:fps=${FPS}`;
    case "pan-left":
      return `zoompan=z='1.2':x='(iw-iw/zoom)*(1-on/${N})':y='ih/2-(ih/zoom/2)':d=${N}:s=${W}x${H}:fps=${FPS}`;
    case "pan-right":
      return `zoompan=z='1.2':x='(iw-iw/zoom)*(on/${N})':y='ih/2-(ih/zoom/2)':d=${N}:s=${W}x${H}:fps=${FPS}`;
    case "static":
      return `zoompan=z='1.0':${cx}:${cy}:d=${N}:s=${W}x${H}:fps=${FPS}`;
    case "zoom-in":
    default:
      return `zoompan=z='min(1.0+${zs}*on,1.4)':${cx}:${cy}:d=${N}:s=${W}x${H}:fps=${FPS}`;
  }
}

function srtTime(sec) {
  const ms = Math.max(0, Math.round(sec * 1000));
  const h = String(Math.floor(ms / 3600000)).padStart(2, "0");
  const m = String(Math.floor((ms % 3600000) / 60000)).padStart(2, "0");
  const s = String(Math.floor((ms % 60000) / 1000)).padStart(2, "0");
  const mm = String(ms % 1000).padStart(3, "0");
  return `${h}:${m}:${s},${mm}`;
}

function main() {
  const args = process.argv.slice(2);
  const get = (k, d) => {
    const a = args.find((x) => x.startsWith(`--${k}=`));
    return a ? a.slice(k.length + 3) : d;
  };
  const sbPath = resolveHome(get("storyboard", "storyboard.json"));
  if (!fs.existsSync(sbPath)) {
    console.error(`✗ storyboard 不存在: ${sbPath}`);
    process.exit(1);
  }
  const root = path.dirname(path.resolve(sbPath));
  const sb = JSON.parse(fs.readFileSync(sbPath, "utf8"));
  const output = resolveHome(get("output", sb.output || "~/Desktop/story.mp4"));
  const aspect = sb.aspect || "9:16";
  const [W, H] = CANVAS[aspect] || CANVAS["9:16"];
  const abs = (rel) => (rel && path.isAbsolute(rel) ? rel : path.join(root, rel));
  const burn = sb.burnSubs !== false; // 默认烧录字幕
  const shots = Array.isArray(sb.shots) ? sb.shots : [];
  if (!shots.length) {
    console.error("✗ storyboard.shots 为空");
    process.exit(1);
  }

  // ffmpeg 可用性
  try {
    run(FFMPEG, ["-version"], { stdio: "ignore" });
  } catch {
    console.error("✗ 找不到 ffmpeg。请先跑 install-deps.mjs 安装，或设 env FFMPEG 指向可执行文件。");
    process.exit(1);
  }

  const work = path.join(root, ".compose");
  fs.mkdirSync(work, { recursive: true });
  console.log(`画幅 ${aspect} → ${W}x${H}，共 ${shots.length} 镜`);

  // 1) 逐镜出片段
  const clips = [];
  const durations = [];
  for (let i = 0; i < shots.length; i++) {
    const s = shots[i];
    const id = s.id ?? i + 1;
    const img = abs(s.image || `assets/shots/${id}.png`);
    if (!fs.existsSync(img)) {
      console.error(`✗ 镜 ${id} 缺图: ${img}（先跑 minimax-image.mjs --batch）`);
      process.exit(1);
    }
    const audio = abs(s.audio || `assets/audio/${id}.mp3`);
    const hasAudio = fs.existsSync(audio);
    let dur = hasAudio ? probeDuration(audio) : 0;
    if (!dur) dur = Number(s.duration) > 0 ? Number(s.duration) : 3.5; // 无旁白时给个默认停留
    dur = Math.max(1.2, dur + 0.25); // 句尾留 0.25s 呼吸
    const N = Math.max(2, Math.round(dur * FPS));
    durations.push(dur);

    const clip = path.join(work, `clip-${String(i).padStart(3, "0")}.mp4`);
    // 先把图放大 2x 给运镜留像素余量，再 zoompan 出画布尺寸
    const vf = `scale=${W * 2}:${H * 2}:force_original_aspect_ratio=increase,crop=${W * 2}:${H * 2},${kenburns(
      s.motion,
      N,
      W,
      H,
    )},setsar=1,format=yuv420p`;

    const ff = ["-y", "-loop", "1", "-framerate", String(FPS), "-i", img];
    if (hasAudio) ff.push("-i", audio);
    ff.push("-filter_complex", `[0:v]${vf}[v]`, "-map", "[v]");
    if (hasAudio) ff.push("-map", "1:a", "-c:a", "aac", "-b:a", "128k");
    else
      ff.push(
        // 无旁白：补一段静音轨，保证拼接后各片段都有音轨
        "-f",
        "lavfi",
        "-i",
        "anullsrc=channel_layout=stereo:sample_rate=44100",
        "-map",
        "2:a",
        "-c:a",
        "aac",
        "-b:a",
        "128k",
      );
    ff.push("-t", dur.toFixed(3), "-r", String(FPS), "-c:v", "libx264", "-pix_fmt", "yuv420p", clip);

    console.log(`[镜 ${id}] ${dur.toFixed(2)}s ${s.motion || "zoom-in"} → 片段`);
    run(FFMPEG, ff);
    clips.push(clip);
  }

  // 2) 拼接
  const listFile = path.join(work, "concat.txt");
  fs.writeFileSync(
    listFile,
    clips.map((c) => `file '${c.replace(/'/g, "'\\''")}'`).join("\n"),
    "utf8",
  );
  const joined = path.join(work, "joined.mp4");
  run(FFMPEG, [
    "-y",
    "-f",
    "concat",
    "-safe",
    "0",
    "-i",
    listFile,
    "-c:v",
    "libx264",
    "-pix_fmt",
    "yuv420p",
    "-c:a",
    "aac",
    "-b:a",
    "128k",
    joined,
  ]);

  // 3) 字幕 (cumulative 时间轴)
  let subsRel = null;
  if (burn) {
    let t = 0;
    const lines = [];
    for (let i = 0; i < shots.length; i++) {
      const text = (shots[i].subtitle || shots[i].narration || "").trim();
      const start = t;
      t += durations[i];
      if (!text) continue;
      lines.push(
        `${lines.length + 1}\n${srtTime(start)} --> ${srtTime(t)}\n${text}\n`,
      );
    }
    if (lines.length) {
      fs.writeFileSync(path.join(work, "subs.srt"), lines.join("\n"), "utf8");
      subsRel = "subs.srt"; // 相对名，ffmpeg 在 .compose 目录里跑，免去 Windows 路径转义
    }
  }

  // 4) 终合成：字幕 + BGM
  const bgm = sb.bgm ? resolveHome(sb.bgm) : "";
  const hasBgm = bgm && fs.existsSync(bgm);
  const vol = Number(sb.bgmVolume);
  const bgmVol = Number.isFinite(vol) && vol >= 0 ? vol : 0.18;
  const subStyle = `FontName=${SUB_FONT},Outline=2,Shadow=1,BorderStyle=1,Alignment=2,MarginV=${Math.round(
    H * 0.06,
  )}`;

  const outAbs = output;
  fs.mkdirSync(path.dirname(outAbs), { recursive: true });

  if (!subsRel && !hasBgm) {
    fs.copyFileSync(joined, outAbs);
    cleanup(work);
    console.log(`\n✅ 完成: ${outAbs}`);
    return;
  }

  const inputs = ["-y", "-i", "joined.mp4"];
  if (hasBgm) inputs.push("-stream_loop", "-1", "-i", bgm);
  const fc = [];
  let vlabel = "0:v";
  if (subsRel) {
    fc.push(`[0:v]subtitles=${subsRel}:force_style='${subStyle}'[v]`);
    vlabel = "[v]";
  }
  let alabel = "0:a";
  if (hasBgm) {
    fc.push(
      `[1:a]volume=${bgmVol}[bg];[0:a][bg]amix=inputs=2:duration=first:dropout_transition=3[a]`,
    );
    alabel = "[a]";
  }
  const ff = [...inputs, "-filter_complex", fc.join(";"), "-map", vlabel, "-map", alabel];
  ff.push("-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k", "-shortest", outAbs);
  // cwd = work，让 subtitles 滤镜用相对文件名
  run(FFMPEG, ff, { cwd: work });

  cleanup(work);
  console.log(`\n✅ 完成: ${outAbs}`);
}

function cleanup(work) {
  if (process.env.STORY_KEEP_TMP) return;
  try {
    fs.rmSync(work, { recursive: true, force: true });
  } catch {}
}

try {
  main();
} catch (e) {
  console.error(`✗ ${e.message ?? e}`);
  process.exit(1);
}
