#!/usr/bin/env node
/**
 * Polaris 视频工坊 · Phase 4 录屏合成（自动出片）
 *
 * 用法:
 *   node 03-record.mjs --project=<presentation 目录> [--output=~/Desktop/x.mp4] [--port=5174]
 *                      [--subtitles=zh-Hans,en] [--burn=zh-Hans,en | --no-burn]
 *
 * 字幕:
 *   --subtitles  逗号分隔的字幕语言（顺序敏感）。每段台词从 audio-segments.json
 *                的 subtitles[lang] 取，缺则回退到该段 text。每种语言另存一份 .srt，
 *                并作为可切换软字幕轨嵌入 MP4。
 *   --burn       其中要烧进画面的语言（最多 2 种叠成双语硬字幕）。默认烧前 2 种。
 *   --no-burn    一种都不烧，全部作软字幕。
 *
 * 与旧版的区别（旧版只能跑当年那个 demo）:
 *   - 不再写死 ROOT / OUT / CHAPTERS：
 *       · 工作目录来自 --project
 *       · 输出路径来自 --output
 *       · 章节/步骤结构来自配音阶段产出的 audio-segments.json（权威有序清单）
 *   - 端口可控且严格：--strictPort 锁端口，启动前先清掉占用该端口的残留进程
 *   - 进程杀树：Windows 用 taskkill /T，*nix 用进程组 kill —— 不再留孤儿 dev server
 *     占着端口导致"下次再跑就卡"
 */
import { chromium } from 'playwright';
import { spawn } from 'child_process';
import http from 'http';
import path from 'path';
import os from 'os';
import { promises as fs } from 'fs';
import { existsSync, readFileSync } from 'fs';

// ───────── 参数 ─────────
function resolveHome(p) {
  if (!p) return p;
  if (p.startsWith('~/') || p === '~') return path.join(os.homedir(), p.slice(1).replace(/^\//, ''));
  return path.resolve(p);
}

function parseArgs() {
  const out = { project: null, output: null, port: 5174, subtitles: [], burn: [], noBurn: false };
  const list = (v) => v.split(',').map((s) => s.trim()).filter(Boolean);
  for (const a of process.argv.slice(2)) {
    if (a.startsWith('--project=')) out.project = a.slice('--project='.length);
    else if (a.startsWith('--output=')) out.output = a.slice('--output='.length);
    else if (a.startsWith('--port=')) out.port = parseInt(a.slice('--port='.length), 10) || 5174;
    else if (a.startsWith('--subtitles=')) out.subtitles = list(a.slice('--subtitles='.length));
    else if (a.startsWith('--burn=')) out.burn = list(a.slice('--burn='.length));
    else if (a === '--no-burn') out.noBurn = true;
  }
  // 未显式给 --burn 时，默认烧录前 2 种字幕（除非 --no-burn）
  if (!out.noBurn && out.subtitles.length && !out.burn.length) out.burn = out.subtitles.slice(0, 2);
  if (out.noBurn) out.burn = [];
  // burn 必须是 subtitles 的子集
  out.burn = out.burn.filter((c) => out.subtitles.includes(c));
  return out;
}

const args = parseArgs();
if (!args.project) {
  console.error('✗ 必须指定 --project=<presentation 目录>');
  console.error('  例: node 03-record.mjs --project=./polaris-video-work/presentation --output=~/Desktop/out.mp4');
  process.exit(1);
}

const PROJECT = resolveHome(args.project);
const OUT = resolveHome(args.output || '~/Desktop/polaris-video.mp4');
const PORT = args.port;
const SUB_LANGS = args.subtitles; // 要生成的字幕语言（顺序敏感）
const BURN_LANGS = args.burn;     // 其中要烧进画面的语言（最多 2 种叠成双语）

// 字幕语言 → 显示名 / ISO 639-2 轨道标签
const SUB_META = {
  'zh-Hans': { name: '简体中文', iso: 'chi' },
  'zh-Hant': { name: '繁體中文', iso: 'chi' },
  yue: { name: '粤语', iso: 'yue' },
  en: { name: 'English', iso: 'eng' },
  ja: { name: '日本語', iso: 'jpn' },
  ko: { name: '한국어', iso: 'kor' },
  es: { name: 'Español', iso: 'spa' },
  fr: { name: 'Français', iso: 'fra' },
  de: { name: 'Deutsch', iso: 'deu' },
  ru: { name: 'Русский', iso: 'rus' },
  pt: { name: 'Português', iso: 'por' },
  it: { name: 'Italiano', iso: 'ita' },
  ar: { name: 'العربية', iso: 'ara' },
  hi: { name: 'हिन्दी', iso: 'hin' },
  th: { name: 'ไทย', iso: 'tha' },
  vi: { name: 'Tiếng Việt', iso: 'vie' },
  id: { name: 'Indonesia', iso: 'ind' },
};
const subName = (c) => (SUB_META[c]?.name) || c;
const subIso = (c) => (SUB_META[c]?.iso) || 'und';

if (!existsSync(path.join(PROJECT, 'package.json'))) {
  console.error(`✗ ${PROJECT} 下没有 package.json，不是一个有效的 presentation 项目`);
  process.exit(1);
}

// ───────── 读 audio-segments.json（结构来源）─────────
function loadSegments() {
  const segPath = path.join(PROJECT, 'audio-segments.json');
  if (!existsSync(segPath)) {
    console.error(`✗ 找不到 ${segPath}`);
    console.error('  请先在 presentation 目录跑: npm run extract-narrations && npm run synthesize-audio');
    process.exit(1);
  }
  let list;
  try {
    list = JSON.parse(readFileSync(segPath, 'utf-8'));
  } catch (e) {
    console.error(`✗ audio-segments.json 解析失败: ${e.message}`);
    process.exit(1);
  }
  if (!Array.isArray(list) || list.length === 0) {
    console.error('✗ audio-segments.json 为空，没有可录制的步骤');
    process.exit(1);
  }
  // 每段: { chapter, step, audio, text, subtitles? }
  return list.map((s) => ({
    chapter: s.chapter,
    step: s.step,
    audio: path.join(PROJECT, 'public', 'audio', s.audio || `${s.chapter}/${s.step}.mp3`),
    text: typeof s.text === 'string' ? s.text : '',
    subs: s.subtitles && typeof s.subtitles === 'object' ? s.subtitles : {},
  }));
}

// ───────── 子进程工具 ─────────
function execAsync(cmd, cmdArgs, opts = {}) {
  return new Promise((resolve, reject) => {
    const p = spawn(cmd, cmdArgs, { ...opts, stdio: 'pipe' });
    let out = '', err = '';
    p.stdout?.on('data', (d) => (out += d));
    p.stderr?.on('data', (d) => (err += d));
    p.on('error', reject);
    p.on('close', (code) => {
      if (code === 0) resolve(out);
      else reject(new Error(`${cmd} ${cmdArgs.join(' ')} exit ${code}: ${err.slice(-800)}`));
    });
  });
}

/** 杀进程树：win 用 taskkill /T /F，*nix 杀进程组 */
function killTree(proc) {
  if (!proc || proc.killed || proc.exitCode != null) return;
  try {
    if (process.platform === 'win32') {
      spawn('taskkill', ['/PID', String(proc.pid), '/T', '/F'], { stdio: 'ignore' });
    } else {
      try { process.kill(-proc.pid, 'SIGTERM'); } catch { proc.kill('SIGTERM'); }
    }
  } catch { /* ignore */ }
}

/** 启动前清掉占用目标端口的残留进程（关键：避免上一次的孤儿 dev server 让本次卡死）*/
async function freePort(port) {
  try {
    if (process.platform === 'win32') {
      const out = await execAsync('cmd', ['/c', `netstat -ano | findstr :${port}`]).catch(() => '');
      const pids = new Set();
      for (const line of out.split(/\r?\n/)) {
        const m = line.trim().match(/LISTENING\s+(\d+)\s*$/);
        if (m && m[1] !== '0') pids.add(m[1]);
      }
      for (const pid of pids) {
        await execAsync('taskkill', ['/PID', pid, '/F', '/T']).catch(() => {});
        console.log(`  · 已清理占用 :${port} 的残留进程 PID ${pid}`);
      }
    } else {
      await execAsync('bash', ['-c', `lsof -ti:${port} | xargs -r kill -9`]).catch(() => {});
    }
  } catch { /* ignore */ }
}

function waitForPort(port, timeout = 60_000) {
  return new Promise((resolve) => {
    const start = Date.now();
    const check = () => {
      const req = http.get(`http://localhost:${port}/`, { timeout: 1500 }, (res) => {
        if (res.statusCode && res.statusCode < 500) return resolve(true);
        retry();
      });
      req.on('error', retry);
      req.on('timeout', () => { req.destroy(); retry(); });
    };
    const retry = () => {
      if (Date.now() - start > timeout) return resolve(false);
      setTimeout(check, 600);
    };
    check();
  });
}

async function getAudioDuration(mp3Path) {
  const out = await execAsync('ffprobe', [
    '-v', 'error', '-show_entries', 'format=duration',
    '-of', 'default=noprint_wrappers=1:nokey=1', mp3Path,
  ]);
  return parseFloat(out.trim());
}

// ───────── 字幕生成（按每段音频精确时长排时间轴）─────────
function srtTime(sec) {
  if (!isFinite(sec) || sec < 0) sec = 0;
  const ms = Math.round(sec * 1000);
  const h = Math.floor(ms / 3600000);
  const m = Math.floor((ms % 3600000) / 60000);
  const s = Math.floor((ms % 60000) / 1000);
  const r = ms % 1000;
  const p = (n, w = 2) => String(n).padStart(w, '0');
  return `${p(h)}:${p(m)}:${p(s)},${p(r, 3)}`;
}

/** 取某段在某语言下的字幕文本：优先 subtitles[lang]，否则回退到该段 text。 */
function cueText(seg, langs) {
  return langs
    .map((l) => (seg.subs && seg.subs[l]) || seg.text || '')
    .map((t) => t.trim())
    .filter(Boolean)
    .join('\n');
}

/** 用 shots（含 duration）+ segments 生成一份 SRT 文本。langs 多于 1 个时叠成多行（双语）。 */
function buildSrt(shots, segments, langs) {
  let t = 0;
  let idx = 0;
  const blocks = [];
  for (let i = 0; i < shots.length; i++) {
    const dur = shots[i].duration > 0 ? shots[i].duration : 2.0;
    const start = t;
    const end = t + dur;
    t = end;
    const text = cueText(segments[i], langs);
    if (text) {
      idx++;
      blocks.push(`${idx}\n${srtTime(start)} --> ${srtTime(end)}\n${text}\n`);
    }
  }
  return blocks.join('\n');
}

/** ffmpeg subtitles 滤镜的路径转义：反斜杠→正斜杠，冒号需转义。 */
function ffSubPath(p) {
  return p.replace(/\\/g, '/').replace(/:/g, '\\:');
}

// MP4 旁同名字幕文件路径：<out 去扩展名>.<lang>.srt
function sidecarSrt(lang) {
  const ext = path.extname(OUT);
  const base = OUT.slice(0, OUT.length - ext.length);
  return `${base}.${lang}.srt`;
}

const SUB_STYLE =
  process.platform === 'win32'
    ? "FontName=Microsoft YaHei,FontSize=22,Outline=2,Shadow=0,MarginV=42,Alignment=2"
    : process.platform === 'darwin'
      ? "FontName=PingFang SC,FontSize=22,Outline=2,Shadow=0,MarginV=42,Alignment=2"
      : "FontName=Noto Sans CJK SC,FontSize=22,Outline=2,Shadow=0,MarginV=42,Alignment=2";

// ───────── 主流程 ─────────
async function main() {
  const segments = loadSegments();
  console.log(`▶ 项目: ${PROJECT}`);
  console.log(`▶ 步骤: ${segments.length} 段（来自 audio-segments.json）`);
  console.log(`▶ 输出: ${OUT}`);

  // 0. 预清端口
  await freePort(PORT);

  // 1. 启动 dev server（锁端口，严格失败而非自增）
  console.log(`▶ 启动 dev server (锁定 :${PORT})...`);
  const server = spawn('npm', ['run', 'dev', '--', '--port', String(PORT), '--strictPort'], {
    cwd: PROJECT,
    shell: true,
    stdio: 'pipe',
    detached: process.platform !== 'win32', // *nix 下建独立进程组以便杀树
    env: { ...process.env, BROWSER: 'none' },
  });
  let serverLog = '';
  server.stdout?.on('data', (d) => (serverLog += d));
  server.stderr?.on('data', (d) => (serverLog += d));

  // 任何阶段失败都确保 server 被杀
  const cleanupServer = () => killTree(server);
  process.on('exit', cleanupServer);

  console.log('⏳ 等待服务就绪...');
  if (!(await waitForPort(PORT))) {
    console.error(`✗ 服务启动超时（:${PORT}）。dev server 日志:`);
    console.error(serverLog.slice(-1200) || '(无输出)');
    cleanupServer();
    process.exit(1);
  }

  // 2. 逐步截图（每段对应一次 ArrowRight 推进）
  console.log('▶ 截图每一步...');
  const browser = await chromium.launch({ headless: true });
  let shotsDir, clipsDir, concatFile, concatTmp, burnSrtFile;
  try {
    const context = await browser.newContext({ viewport: { width: 1920, height: 1080 } });
    const page = await context.newPage();
    await page.goto(`http://localhost:${PORT}/`, { waitUntil: 'networkidle' }).catch(() => {});
    await page.waitForTimeout(1500);

    shotsDir = path.join(PROJECT, '.polaris-shots');
    await fs.mkdir(shotsDir, { recursive: true });

    const shots = [];
    for (let i = 0; i < segments.length; i++) {
      const seg = segments[i];
      const png = path.join(shotsDir, `step_${String(i).padStart(3, '0')}.png`);
      await page.screenshot({ path: png, fullPage: false });

      let duration = 2.0;
      try {
        duration = await getAudioDuration(seg.audio);
        console.log(`  [${i}] ${seg.chapter}/${seg.step}.mp3 = ${duration.toFixed(2)}s`);
      } catch {
        console.log(`  [${i}] ${seg.chapter}/${seg.step} (无音频, 默认 2.0s)`);
      }
      shots.push({ png, audio: seg.audio, duration, hasAudio: existsSync(seg.audio) });

      if (i < segments.length - 1) {
        await page.keyboard.press('ArrowRight');
        await page.waitForTimeout(350);
      }
    }
    await context.close();

    // 3. 每步合成带音频的片段
    console.log('▶ 生成每步视频片段...');
    clipsDir = path.join(PROJECT, '.polaris-clips');
    await fs.mkdir(clipsDir, { recursive: true });
    const vf = 'scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:(ow-iw)/2:(oh-ih)/2';
    const concatList = [];
    for (let i = 0; i < shots.length; i++) {
      const s = shots[i];
      const clip = path.join(clipsDir, `clip_${String(i).padStart(3, '0')}.mp4`);
      if (s.hasAudio) {
        await execAsync('ffmpeg', [
          '-y', '-loop', '1', '-i', s.png, '-i', s.audio,
          '-c:v', 'libx264', '-t', String(s.duration), '-pix_fmt', 'yuv420p', '-vf', vf,
          '-c:a', 'aac', '-b:a', '128k', '-shortest', clip,
        ]);
      } else {
        await execAsync('ffmpeg', [
          '-y', '-loop', '1', '-i', s.png,
          '-c:v', 'libx264', '-t', String(s.duration), '-pix_fmt', 'yuv420p', '-vf', vf, '-an', clip,
        ]);
      }
      concatList.push(`file '${clip.replace(/'/g, "'\\''")}'`);
    }

    // 4. 拼接
    console.log('▶ 拼接最终视频...');
    concatFile = path.join(PROJECT, '.polaris-concat.txt');
    await fs.writeFile(concatFile, concatList.join('\n'));
    await fs.mkdir(path.dirname(OUT), { recursive: true });

    if (!SUB_LANGS.length) {
      // 无字幕：直接拼到成品
      await execAsync('ffmpeg', ['-y', '-f', 'concat', '-safe', '0', '-i', concatFile, '-c', 'copy', OUT]);
    } else {
      // 有字幕：先拼到临时文件，再做一次字幕合成
      concatTmp = path.join(PROJECT, '.polaris-concat-out.mp4');
      await execAsync('ffmpeg', ['-y', '-f', 'concat', '-safe', '0', '-i', concatFile, '-c', 'copy', concatTmp]);

      // 4a. 每种语言生成一份 .srt 边车文件
      console.log(`▶ 生成字幕（${SUB_LANGS.map(subName).join('、')}）...`);
      const srtPaths = {};
      for (const lang of SUB_LANGS) {
        const srt = buildSrt(shots, segments, [lang]);
        const p = sidecarSrt(lang);
        await fs.writeFile(p, srt, 'utf-8');
        srtPaths[lang] = p;
        console.log(`  · ${subName(lang)} → ${p}`);
      }

      // 4b. 组 ffmpeg：软字幕轨 + 可选硬烧录
      const ff = ['-y', '-i', concatTmp];
      for (const lang of SUB_LANGS) ff.push('-i', srtPaths[lang]);
      ff.push('-map', '0:v', '-map', '0:a');
      for (let k = 0; k < SUB_LANGS.length; k++) ff.push('-map', String(k + 1));

      if (BURN_LANGS.length) {
        // 前 1–2 种叠成（双语）硬字幕烧进画面
        const burnSrt = buildSrt(shots, segments, BURN_LANGS);
        burnSrtFile = path.join(PROJECT, '.polaris-burn.srt');
        await fs.writeFile(burnSrtFile, burnSrt, 'utf-8');
        ff.push('-vf', `subtitles='${ffSubPath(burnSrtFile)}':force_style='${SUB_STYLE}'`);
        ff.push('-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-crf', '20', '-preset', 'medium');
        console.log(`▶ 烧录硬字幕：${BURN_LANGS.map(subName).join(' + ')}`);
      } else {
        ff.push('-c:v', 'copy');
      }
      ff.push('-c:a', 'copy', '-c:s', 'mov_text');
      // 软字幕轨语言/标题标签
      for (let k = 0; k < SUB_LANGS.length; k++) {
        ff.push(`-metadata:s:s:${k}`, `language=${subIso(SUB_LANGS[k])}`);
        ff.push(`-metadata:s:s:${k}`, `title=${subName(SUB_LANGS[k])}`);
      }
      ff.push(OUT);
      await execAsync('ffmpeg', ff);
    }
  } finally {
    await browser.close().catch(() => {});
    cleanupServer();
    // 清理临时文件
    if (shotsDir) await fs.rm(shotsDir, { recursive: true, force: true }).catch(() => {});
    if (clipsDir) await fs.rm(clipsDir, { recursive: true, force: true }).catch(() => {});
    if (concatFile) await fs.rm(concatFile, { force: true }).catch(() => {});
    if (concatTmp) await fs.rm(concatTmp, { force: true }).catch(() => {});
    if (burnSrtFile) await fs.rm(burnSrtFile, { force: true }).catch(() => {});
  }

  const stat = await fs.stat(OUT);
  console.log(`✓ 视频已生成: ${OUT}`);
  console.log(`  大小: ${(stat.size / 1024 / 1024).toFixed(2)} MB · ${segments.length} 步`);
  if (SUB_LANGS.length) {
    console.log(
      `  字幕: ${SUB_LANGS.map(subName).join('、')}` +
        (BURN_LANGS.length ? `（烧录 ${BURN_LANGS.map(subName).join(' + ')}）` : '（全软字幕轨）') +
        ` · 另存 ${SUB_LANGS.length} 份 .srt`,
    );
  }
}

main().catch((e) => {
  console.error('✗ 录屏合成失败:', e.message || e);
  process.exit(1);
});
