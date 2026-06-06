#!/usr/bin/env node
/**
 * Polaris 视频工坊 · Phase 4 录屏合成（自动出片）
 *
 * 用法:
 *   node 03-record.mjs --project=<presentation 目录> [--output=~/Desktop/x.mp4] [--port=5174]
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
  const out = { project: null, output: null, port: 5174 };
  for (const a of process.argv.slice(2)) {
    if (a.startsWith('--project=')) out.project = a.slice('--project='.length);
    else if (a.startsWith('--output=')) out.output = a.slice('--output='.length);
    else if (a.startsWith('--port=')) out.port = parseInt(a.slice('--port='.length), 10) || 5174;
  }
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
  // 每段: { chapter, step, audio: "<chapter>/<step>.mp3" }
  return list.map((s) => ({
    chapter: s.chapter,
    step: s.step,
    audio: path.join(PROJECT, 'public', 'audio', s.audio || `${s.chapter}/${s.step}.mp3`),
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
  let shotsDir, clipsDir, concatFile;
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
    await execAsync('ffmpeg', ['-y', '-f', 'concat', '-safe', '0', '-i', concatFile, '-c', 'copy', OUT]);
  } finally {
    await browser.close().catch(() => {});
    cleanupServer();
    // 清理临时文件
    if (shotsDir) await fs.rm(shotsDir, { recursive: true, force: true }).catch(() => {});
    if (clipsDir) await fs.rm(clipsDir, { recursive: true, force: true }).catch(() => {});
    if (concatFile) await fs.rm(concatFile, { force: true }).catch(() => {});
  }

  const stat = await fs.stat(OUT);
  console.log(`✓ 视频已生成: ${OUT}`);
  console.log(`  大小: ${(stat.size / 1024 / 1024).toFixed(2)} MB · ${segments.length} 步`);
}

main().catch((e) => {
  console.error('✗ 录屏合成失败:', e.message || e);
  process.exit(1);
});
