<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import {
  envDoctor,
  listen,
  isTauri,
  type ClaudeUpdateInfo,
  type EnvReport,
  type EnvStreamEvent,
  type ToolStatus,
} from "../tauri";
import McpConfigModal from "./McpConfigModal.vue";

/**
 * 环境医生 — 新用户开箱的「环境监测 + 配置安装」。
 * - gate=true: 作为启动流程的一道关 (全屏覆盖), 健康则自动放行;
 * - gate=false: 作为侧栏「环境」页随时复检 / 重装。
 */
const props = withDefaults(defineProps<{ gate?: boolean }>(), { gate: false });
const emit = defineEmits<{ (e: "done"): void }>();

const READY_FLAG = "polaris.env.ready.v1";

type Phase = "checking" | "ready-skip" | "panel";
const phase = ref<Phase>("checking");
const report = ref<EnvReport | null>(null);

// 安装 / 修复 / 更新 的进行态
const busyKind = ref<
  "" | "claude" | "claude-npm" | "claude-update" | "node" | "pwsh" | "path"
>("");
const installReqId = ref<string | null>(null);
const logs = ref<string[]>([]);
const banner = ref<{ kind: "ok" | "err" | "info"; text: string } | null>(null);
let unlisten: (() => void) | null = null;

// Claude Code 更新检测
const updateInfo = ref<ClaudeUpdateInfo | null>(null);
const checkingUpdate = ref(false);

const busy = computed(() => busyKind.value !== "");

// 本次会话只自动装一次 shell, 避免失败时每次复检都反复弹 UAC
let autoPwshTried = false;
// 想装 Claude 但缺 npm 时: 先自动装 Node.js, 装完链式继续装 Claude (一次点击装齐)
let chainClaudeAfterNode = false;

async function runCheck() {
  report.value = await envDoctor.check();
  return report.value;
}

/**
 * claude 已装但缺可用 shell (真身 PowerShell 7 / Git Bash) → 自动装 PowerShell 7。
 * 否则用户进去对话时 claude 会报「找不到 PowerShell / bash」。仅启动关 (gate) 自动触发,
 * 每次会话最多一次。返回是否已发起安装 (true ⇒ 调用方应让出, 进入流式日志)。
 */
function maybeAutoInstallShell(r: EnvReport): boolean {
  if (!props.gate || !isTauri) return false;
  if (!r.claude.found || r.shellReady || autoPwshTried || busy.value) return false;
  autoPwshTried = true;
  phase.value = "panel";
  installPwsh();
  banner.value = {
    kind: "info",
    text: "检测到缺少 Claude Code 可用的 Shell（PowerShell 7），正在自动为你安装——装好后即可正常对话，无需重启。",
  };
  return true;
}

onMounted(async () => {
  // 浏览器预览 / 非 Tauri: 启动关直接放行, 不打扰
  if (!isTauri) {
    if (props.gate) emit("done");
    else {
      report.value = await runCheck();
      phase.value = "panel";
    }
    return;
  }

  // 已通过一次环境检测 → 后续每次启动不再检测, 直接放行。
  // (用户要求: 只第一次打开时检测; 想复检走侧栏「环境」页 gate=false)
  if (props.gate && localStorage.getItem(READY_FLAG)) {
    emit("done");
    return;
  }

  unlisten = await listen<EnvStreamEvent>("env:stream", onStream);

  const r = await runCheck();
  if (r.ready) localStorage.setItem(READY_FLAG, "1");
  // claude 在但缺 shell → 自动补装 PowerShell 7 (进入流式日志), 不再放任用户进去后对话报错
  if (maybeAutoInstallShell(r)) return;
  phase.value = r.ready && props.gate ? "ready-skip" : "panel";
});

onBeforeUnmount(() => {
  if (unlisten) unlisten();
});

// 进入面板 / 跳过页且 Claude Code 已安装时, 自动静默检测一次更新
watch(phase, (p) => {
  if (
    (p === "panel" || p === "ready-skip") &&
    isTauri &&
    report.value?.claude.found &&
    !updateInfo.value &&
    !checkingUpdate.value
  ) {
    checkClaudeUpdate();
  }
});

function onStream(ev: EnvStreamEvent) {
  if (installReqId.value && ev.reqId !== installReqId.value) return;
  if (ev.kind === "log" && ev.line) {
    logs.value.push(ev.line);
    if (logs.value.length > 400) logs.value.splice(0, logs.value.length - 400);
  } else if (ev.kind === "done") {
    finishInstall(ev.ok ?? false, ev.message ?? "");
  }
}

async function finishInstall(ok: boolean, message: string) {
  const justFinished = busyKind.value; // 重置前先记住刚装完的是什么
  installReqId.value = null;
  busyKind.value = "";
  banner.value = { kind: ok ? "ok" : "err", text: message || (ok ? "完成。" : "未成功。") };
  const r = await runCheck();
  if (r.ready) localStorage.setItem(READY_FLAG, "1");
  // 装好 / 更新完后重新检测版本, 让「更新」按钮翻成「已是最新」
  updateInfo.value = null;
  if (r.claude.found) checkClaudeUpdate();

  // 链式①: 刚装完 Node.js 且本意是装 Claude → npm 就绪后自动继续装 Claude (一次点击装齐)
  if (justFinished === "node" && chainClaudeAfterNode) {
    chainClaudeAfterNode = false;
    if (ok && r.npm.found && !r.claude.found) {
      banner.value = { kind: "info", text: "Node.js 就绪，正在自动继续安装 Claude Code…" };
      installClaude("npm");
      return;
    }
  }

  // 链式②: 刚装完 claude 但还缺 shell → 自动补上 PowerShell 7 (启动关, 仅 Windows)
  maybeAutoInstallShell(r);
}

async function installClaude(method: "native" | "npm") {
  // npm 方式但缺 Node/npm → 先自动装 Node, 装完由 finishInstall 链式继续装 Claude。
  // 这样用户一次点击即「Node.js + npm + Claude Code」一起装齐 (两端通用)。
  if (method === "npm" && !npmReady.value) {
    chainClaudeAfterNode = true;
    await installNode();
    return;
  }
  banner.value = null;
  logs.value = [];
  busyKind.value = method === "npm" ? "claude-npm" : "claude";
  try {
    installReqId.value = await envDoctor.installClaude(method);
    logs.value.push(
      method === "npm"
        ? "$ npm install -g @anthropic-ai/claude-code --registry=https://registry.npmmirror.com"
        : isWin.value
          ? "$ irm https://claude.ai/install.ps1 | iex"
          : "$ curl -fsSL https://claude.ai/install.sh | bash"
    );
  } catch (e) {
    busyKind.value = "";
    banner.value = { kind: "err", text: String(e) };
  }
}

async function installNode() {
  banner.value = null;
  logs.value = [];
  busyKind.value = "node";
  try {
    installReqId.value = await envDoctor.installNode();
    logs.value.push(
      isWin.value
        ? "$ winget install --id OpenJS.NodeJS.LTS -e  (或下载官方 MSI)"
        : "$ curl -fsSL https://cdn.npmmirror.com/binaries/node/... | tar xz  → ~/.local/polaris-node"
    );
  } catch (e) {
    busyKind.value = "";
    banner.value = { kind: "err", text: String(e) };
  }
}

async function installPwsh() {
  banner.value = null;
  logs.value = [];
  busyKind.value = "pwsh";
  try {
    installReqId.value = await envDoctor.installPwsh();
    logs.value.push("$ winget install --id Microsoft.PowerShell -e");
  } catch (e) {
    busyKind.value = "";
    banner.value = { kind: "err", text: String(e) };
  }
}

// 检测 Claude Code 是否有新版本 (静默, 不打扰; 仅在已安装时有意义)
async function checkClaudeUpdate() {
  if (checkingUpdate.value || busy.value) return;
  if (!report.value?.claude.found) return;
  checkingUpdate.value = true;
  try {
    updateInfo.value = await envDoctor.checkClaudeUpdate();
  } catch {
    // 检测失败不报错横幅, 静默留待用户手动点「检查更新」重试
  } finally {
    checkingUpdate.value = false;
  }
}

// 一键更新 Claude Code (走国内 npmmirror), 复用安装的流式日志管线
async function updateClaude() {
  banner.value = null;
  logs.value = [];
  busyKind.value = "claude-update";
  try {
    installReqId.value = await envDoctor.updateClaude();
    logs.value.push(
      "$ npm install -g @anthropic-ai/claude-code@latest --registry=https://registry.npmmirror.com"
    );
  } catch (e) {
    busyKind.value = "";
    banner.value = { kind: "err", text: String(e) };
  }
}

async function fixPath() {
  banner.value = null;
  busyKind.value = "path";
  try {
    const res = await envDoctor.fixPath();
    banner.value = { kind: res.ok ? "ok" : "err", text: res.message };
    await runCheck();
  } catch (e) {
    banner.value = { kind: "err", text: String(e) };
  } finally {
    busyKind.value = "";
  }
}

async function cancelInstall() {
  if (installReqId.value) {
    await envDoctor.cancel(installReqId.value);
  }
}

async function recheck() {
  banner.value = null;
  phase.value = "checking";
  const r = await runCheck();
  if (r.ready) localStorage.setItem(READY_FLAG, "1");
  phase.value = "panel";
}

function enter() {
  emit("done");
}

// 工具状态 → 状态点级别
function level(t: ToolStatus): "ok" | "warn" | "bad" {
  if (t.found) return "ok";
  return t.required ? "bad" : "warn";
}
function statusText(t: ToolStatus): string {
  if (t.found) return t.onPath ? "已就绪" : "已安装 (不在 PATH)";
  return t.required ? "未安装 · 必需" : "未安装 · 建议";
}

// 当前系统 (后端 env_check 回传): "windows" | "macos" | "linux" | "browser"
const osName = computed(() => report.value?.os ?? "");
const isWin = computed(() => osName.value === "windows");

const tools = computed<ToolStatus[]>(() => {
  if (!report.value) return [];
  const all = [report.value.claude, report.value.pwsh, report.value.node, report.value.npm];
  // PowerShell 7 仅 Windows 上是 Claude 的可用 shell; mac/Linux 自带 sh/zsh, 不展示该行。
  return isWin.value ? all : all.filter((t) => t.key !== "pwsh");
});
const pathNeedsFix = computed(
  () =>
    !!report.value &&
    report.value.claude.found &&
    !report.value.claudeDirOnUserPath
);
// npm 安装方式需要 Node.js 带来的 npm; 没有则先引导装 Node
const npmReady = computed(() => !!report.value?.npm.found);
</script>

<template>
  <div :class="props.gate ? 'gate' : 'page'">
    <div class="card">
      <!-- 头 -->
      <div class="badge"><span class="star"></span></div>
      <h1 class="title">环境检测与配置</h1>
      <p class="lead">
        北极星依托 <strong>Claude Code</strong> 在你本机干活。先帮你把运行环境安顿好——
        缺什么一键补上，<strong>环境变量</strong>也会一并配好。
      </p>

      <!-- 检测中 -->
      <div v-if="phase === 'checking'" class="checking">
        <span class="spinner"></span> 正在检测本机环境…
      </div>

      <template v-else>
        <!-- 工具清单 -->
        <ul class="tools">
          <li v-for="t in tools" :key="t.key" class="tool">
            <span class="dot" :class="level(t)"></span>
            <div class="t-main">
              <div class="t-row">
                <span class="t-name">{{ t.name }}</span>
                <span class="t-status" :class="level(t)">{{ statusText(t) }}</span>
              </div>
              <div class="t-sub">
                <span v-if="t.version" class="t-ver">{{ t.version }}</span>
                <span v-else class="t-hint">{{ t.hint }}</span>
                <span v-if="t.path" class="t-path" :title="t.path">{{ t.path }}</span>
              </div>
            </div>
            <!-- 行内动作 -->
            <div class="t-act">
              <template v-if="t.key === 'claude' && !t.found">
                <!-- 有 npm: 默认 npm(国内镜像)装 —— 两端一致, 国内可达, 不碰 claude.ai -->
                <button
                  v-if="npmReady"
                  class="btn primary"
                  :disabled="busy"
                  @click="installClaude('npm')"
                >
                  {{ busyKind === "claude-npm" ? "安装中…" : "一键安装" }}
                </button>
                <!-- 无 npm: 一次点击「Node.js + npm + Claude Code」一起装齐 —— 先自动装
                     Node (Windows winget/MSI, macOS 走 npmmirror tar.gz), 装完链式继续装 Claude -->
                <button
                  v-else
                  class="btn primary"
                  :disabled="busy"
                  title="缺 Node.js：自动先装 Node/npm，再继续用 npm(国内镜像)装 Claude Code"
                  @click="installClaude('npm')"
                >
                  {{
                    busyKind === "node"
                      ? "装 Node.js 中…"
                      : busyKind === "claude-npm"
                        ? "装 Claude 中…"
                        : "一键安装（含 Node.js）"
                  }}
                </button>
              </template>
              <!-- 已装 Claude Code: 检查 / 一键更新 (走国内镜像) -->
              <template v-else-if="t.key === 'claude' && t.found">
                <button
                  v-if="updateInfo?.updateAvailable"
                  class="btn primary"
                  :disabled="busy"
                  :title="`更新到 ${updateInfo.latest}（当前 ${updateInfo.current}）`"
                  @click="updateClaude"
                >
                  {{ busyKind === "claude-update" ? "更新中…" : `更新到 ${updateInfo.latest}` }}
                </button>
                <button
                  v-else
                  class="btn"
                  :disabled="busy || checkingUpdate"
                  :title="updateInfo?.checked ? updateInfo.message : '检查 Claude Code 是否有新版本'"
                  @click="checkClaudeUpdate"
                >
                  {{
                    checkingUpdate
                      ? "检查中…"
                      : updateInfo?.checked
                        ? "已是最新"
                        : "检查更新"
                  }}
                </button>
              </template>
              <template v-else-if="t.key === 'node' && !t.found">
                <button class="btn" :disabled="busy" @click="installNode">
                  {{ busyKind === "node" ? "安装中…" : "安装" }}
                </button>
              </template>
              <template v-else-if="t.key === 'pwsh' && !t.found">
                <button class="btn" :disabled="busy" @click="installPwsh">
                  {{ busyKind === "pwsh" ? "安装中…" : "安装" }}
                </button>
              </template>
            </div>
          </li>
        </ul>

        <!-- 安装 Claude 的方式说明 + 兜底 (两端默认 npm 国内镜像; 官方脚本仅境外网络兜底) -->
        <p v-if="report && !report.claude.found" class="alt">
          默认经<strong>国内镜像</strong>用 npm 安装
          <code>npm i -g @anthropic-ai/claude-code --registry=npmmirror.com</code>（含平台原生二进制，国内可装、不碰 claude.ai）。
          <span v-if="!npmReady">需先安装 <strong>Node.js</strong>（{{ isWin ? "随 npm 一起来" : "免 sudo 装到 ~/.local" }}）。</span>
          <button class="link" :disabled="busy" @click="installClaude('native')">
            或改用官方脚本（境外网络{{ isWin ? "" : " · install.sh" }}）
          </button>
        </p>

        <!-- 环境变量 (PATH) 体检 -->
        <div v-if="pathNeedsFix" class="path-warn">
          <div class="pw-text">
            检测到 <strong>Claude Code 已安装但其目录不在 PATH</strong> 里——
            终端 / 重启后可能找不到 <code>claude</code>。
            <span v-if="report?.claudeDir" class="pw-dir">{{ report.claudeDir }}</span>
          </div>
          <button class="btn primary" :disabled="busy" @click="fixPath">
            {{ busyKind === "path" ? "修复中…" : "修复 PATH" }}
          </button>
        </div>

        <!-- 流式安装日志 -->
        <div v-if="busy && logs.length" class="logwrap">
          <div class="log-head">
            <span>安装日志</span>
            <button v-if="installReqId" class="link" @click="cancelInstall">取消</button>
          </div>
          <pre class="log"><code v-for="(l, i) in logs" :key="i">{{ l }}
</code></pre>
        </div>

        <!-- 结果横幅 -->
        <div v-if="banner" class="banner" :class="banner.kind">{{ banner.text }}</div>

        <!-- 底部动作 -->
        <div class="actions">
          <button class="btn ghost" :disabled="busy" @click="recheck">重新检测</button>
          <div class="spacer"></div>
          <template v-if="props.gate">
            <button v-if="!report?.ready" class="btn text" :disabled="busy" @click="enter">
              稍后再说，先进入
            </button>
            <button class="btn primary" :disabled="busy && !report?.ready" @click="enter">
              {{ report?.ready ? "环境就绪 · 进入北极星" : "仍要进入" }}
            </button>
          </template>
        </div>

        <!-- MCP 服务配置 -->
        <div v-if="!props.gate" class="mcp-section"
        >
          <McpConfigModal inline @close="() => {}" />
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.gate {
  position: fixed;
  inset: 0;
  z-index: 9997;
  display: flex;
  align-items: center;
  justify-content: center;
  background: radial-gradient(120% 80% at 50% -10%, #eef2f7 0%, var(--bg) 55%);
  padding: 40px;
  overflow-y: auto;
}
.page {
  flex: 1;
  overflow-y: auto;
  padding: 40px 56px 80px;
  width: 100%;
}
.card {
  width: 100%;
  max-width: 600px;
  margin: 0 auto;
  background: var(--panel);
  border: 1px solid var(--hairline);
  border-radius: 6px;
  box-shadow: var(--shadow-lg);
  padding: 36px 40px 30px;
  animation: cardIn 0.45s cubic-bezier(0.2, 0.7, 0.2, 1);
}
.page .card {
  box-shadow: var(--shadow-sm);
}
@keyframes cardIn {
  from { opacity: 0; transform: translateY(12px); }
  to { opacity: 1; transform: translateY(0); }
}

.badge {
  display: flex;
  justify-content: center;
  margin-bottom: 18px;
}
.star {
  position: relative;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--primary);
  box-shadow: 0 0 0 4px var(--primary-soft), 0 0 18px 4px rgba(44, 70, 97, 0.25);
}
.star::before,
.star::after {
  content: "";
  position: absolute;
  left: 50%;
  top: 50%;
  background: linear-gradient(var(--g, to right), transparent, var(--primary), transparent);
}
.star::before { width: 40px; height: 1.5px; transform: translate(-50%, -50%); }
.star::after { width: 1.5px; height: 40px; transform: translate(-50%, -50%); }

.title {
  font-family: var(--serif);
  font-size: 21px;
  font-weight: 600;
  letter-spacing: 2px;
  color: var(--ink);
  text-align: center;
  margin: 0 0 14px;
}
.lead {
  font-size: 13px;
  line-height: 1.95;
  color: var(--text-2);
  margin: 0 0 22px;
  letter-spacing: 0.3px;
}
.lead strong { color: var(--ink); font-weight: 600; }

.checking {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 10px;
  padding: 30px 0;
  color: var(--muted);
  font-size: 13px;
}
.spinner {
  width: 14px;
  height: 14px;
  border: 2px solid var(--border);
  border-top-color: var(--primary);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}
@keyframes spin { to { transform: rotate(360deg); } }

.tools {
  list-style: none;
  margin: 0 0 4px;
  padding: 0;
  border: 1px solid var(--border-soft);
  border-radius: 4px;
  overflow: hidden;
}
.tool {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 14px;
  border-bottom: 1px solid var(--border-soft);
}
.tool:last-child { border-bottom: none; }
.dot {
  width: 9px;
  height: 9px;
  border-radius: 50%;
  flex-shrink: 0;
}
.dot.ok { background: #4a8f6d; box-shadow: 0 0 0 3px rgba(74, 143, 109, 0.15); }
.dot.warn { background: #c08a3e; box-shadow: 0 0 0 3px rgba(192, 138, 62, 0.15); }
.dot.bad { background: var(--vermilion); box-shadow: 0 0 0 3px var(--vermilion-soft); }

.t-main { flex: 1; min-width: 0; }
.t-row { display: flex; align-items: baseline; gap: 10px; }
.t-name { font-size: 13.5px; color: var(--ink); font-weight: 500; }
.t-status { font-size: 11px; letter-spacing: 0.5px; }
.t-status.ok { color: #4a8f6d; }
.t-status.warn { color: #c08a3e; }
.t-status.bad { color: var(--vermilion); }
.t-sub {
  display: flex;
  gap: 10px;
  margin-top: 2px;
  font-size: 11px;
  color: var(--muted);
  overflow: hidden;
}
.t-ver { color: var(--text-2); }
.t-path {
  font-family: var(--mono);
  color: var(--dim);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.t-act { flex-shrink: 0; }

.alt {
  font-size: 11.5px;
  color: var(--muted);
  margin: 12px 2px 0;
  line-height: 1.8;
}
.alt code {
  background: var(--code-bg);
  color: var(--code-text);
  padding: 1px 5px;
  border-radius: 3px;
  font-family: var(--mono);
  font-size: 10.5px;
}

.path-warn {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-top: 16px;
  padding: 12px 14px;
  border-radius: 4px;
  background: rgba(192, 138, 62, 0.08);
  border-left: 2px solid #c08a3e;
}
.pw-text { flex: 1; font-size: 12px; line-height: 1.7; color: var(--text-2); }
.pw-text strong { color: var(--ink); }
.pw-dir {
  display: block;
  margin-top: 3px;
  font-family: var(--mono);
  font-size: 10.5px;
  color: var(--dim);
}
.path-warn code {
  background: var(--code-bg);
  color: var(--code-text);
  padding: 0 4px;
  border-radius: 2px;
  font-family: var(--mono);
}

.logwrap {
  margin-top: 16px;
  border: 1px solid var(--border-soft);
  border-radius: 4px;
  overflow: hidden;
}
.log-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 6px 12px;
  background: var(--bg-soft);
  font-size: 11px;
  letter-spacing: 1px;
  color: var(--dim);
  font-family: var(--serif);
}
.log {
  margin: 0;
  padding: 10px 12px;
  max-height: 220px;
  overflow-y: auto;
  background: #0c1320;
  color: #c8d4e6;
  font-family: var(--mono);
  font-size: 11px;
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-all;
}
.log code { background: transparent; color: inherit; }

.banner {
  margin-top: 16px;
  padding: 9px 13px;
  border-radius: 3px;
  font-size: 12.5px;
  line-height: 1.7;
  white-space: pre-wrap;
}
.banner.ok {
  background: var(--primary-soft);
  color: var(--primary-deep);
  border-left: 2px solid var(--primary);
}
.banner.err {
  background: var(--vermilion-soft);
  color: var(--vermilion);
  border-left: 2px solid var(--vermilion);
}
.banner.info {
  background: var(--selection-bg);
  color: var(--text-2);
  border-left: 2px solid var(--border);
}

.actions {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 26px;
}
.spacer { flex: 1; }

.btn {
  padding: 8px 16px;
  border-radius: 3px;
  font-size: 12.5px;
  letter-spacing: 0.5px;
  border: 1px solid var(--border);
  background: transparent;
  color: var(--text-2);
  cursor: pointer;
}
.btn:hover:not(:disabled) { border-color: var(--ink); color: var(--ink); }
.btn.primary { background: var(--ink); color: #fff; border-color: var(--ink); }
.btn.primary:hover:not(:disabled) { background: var(--primary); border-color: var(--primary); }
.btn.ghost { color: var(--text-2); }
.btn.text { border-color: transparent; color: var(--muted); }
.btn.text:hover:not(:disabled) { color: var(--ink); }
.btn:disabled { opacity: 0.5; cursor: not-allowed; }

.link {
  background: transparent;
  border: none;
  color: var(--primary);
  font-size: 11.5px;
  cursor: pointer;
  padding: 0;
}
.link:hover:not(:disabled) { text-decoration: underline; }

.mcp-section {
  margin-top: 28px;
  padding-top: 20px;
  border-top: 1px solid var(--border-soft);
}
.link:disabled { opacity: 0.4; cursor: not-allowed; }
</style>
