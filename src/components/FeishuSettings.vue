<script setup lang="ts">
// 「聊天机器人」—— 飞书（暂时只保留飞书）。
// 配置 App ID/Secret → 启动长连接网关 → 飞书 @机器人/私聊 走 Polaris 真对话管线
// (Claude Code 带工具执行、消息落库 UI 可见) → 回复发回飞书。支持开机自动启动 + 防断守护。
import { onMounted, onUnmounted, ref } from "vue";
import { feishu, type FeishuConfig, type FeishuTestResult } from "../tauri";
import type { UnlistenFn } from "@tauri-apps/api/event";

const cfg = ref<FeishuConfig>({
  enabled: false,
  appId: "",
  appSecret: "",
  autoStart: false,
  domain: "feishu",
  dmPolicy: "open",
  groupRequireMention: true,
  allowFrom: [],
});
const allowText = ref("");
const saving = ref(false);
const testing = ref(false);
const testResult = ref<FeishuTestResult | null>(null);
const savedMsg = ref("");
const showSecret = ref(false);
const showAdvanced = ref(false);

// 扫码弹窗（去飞书开放平台建应用）
const qrOpen = ref(false);
const qrLoading = ref(false);
const qrSvg = ref("");
const qrError = ref("");

// 对话引擎（长连接网关）
const gwState = ref<string>("stopped"); // stopped|starting|installing|connected|reconnecting
const gwBusy = ref(false);
const gwLog = ref<string[]>([]);
let unlistenLog: UnlistenFn | null = null;
let unlistenStatus: UnlistenFn | null = null;

async function load() {
  cfg.value = await feishu.getConfig();
  allowText.value = (cfg.value.allowFrom || []).join("\n");
}
async function save() {
  saving.value = true;
  savedMsg.value = "";
  try {
    cfg.value.allowFrom = allowText.value
      .split(/\r?\n/)
      .map((s) => s.trim())
      .filter(Boolean);
    await feishu.setConfig(cfg.value);
    savedMsg.value = "已保存";
    await load();
  } catch (e: any) {
    savedMsg.value = `保存失败: ${e}`;
  } finally {
    saving.value = false;
  }
}
async function test() {
  testing.value = true;
  testResult.value = null;
  try {
    await save();
    testResult.value = await feishu.test();
  } catch (e: any) {
    testResult.value = { ok: false, botName: "", botOpenId: "", message: `${e}` };
  } finally {
    testing.value = false;
  }
}
function clearField(field: "appId" | "appSecret") {
  cfg.value[field] = "";
}
async function openScan() {
  qrOpen.value = true;
  qrError.value = "";
  qrSvg.value = "";
  qrLoading.value = true;
  try {
    const r = await feishu.createQr();
    qrSvg.value = r.svg;
  } catch (e: any) {
    qrError.value = `${e}`;
  } finally {
    qrLoading.value = false;
  }
}
function closeScan() {
  qrOpen.value = false;
}
async function openConsole() {
  try {
    await feishu.openConsole();
  } catch {
    /* 静默 */
  }
}

function pushLog(t: string) {
  const ts = new Date().toLocaleTimeString();
  gwLog.value.push(`[${ts}] ${t}`);
  if (gwLog.value.length > 200) gwLog.value.shift();
}
async function startGateway() {
  gwBusy.value = true;
  try {
    await save();
    await feishu.gatewayStart();
  } catch (e: any) {
    pushLog(`启动失败：${e?.message || e}`);
    gwBusy.value = false;
  }
}
async function stopGateway() {
  gwBusy.value = true;
  try {
    await feishu.gatewayStop();
  } catch (e: any) {
    pushLog(`停止失败：${e?.message || e}`);
  } finally {
    gwBusy.value = false;
  }
}
// 切「开机自动启动」即时存盘
async function toggleAutoStart() {
  await save();
}

const stateLabel = () =>
  gwState.value === "connected"
    ? "● 在线"
    : gwState.value === "starting"
      ? "○ 启动中…"
      : gwState.value === "installing"
        ? "○ 安装依赖中…"
        : gwState.value === "reconnecting"
          ? "○ 重连中…"
          : "○ 未启动";

onMounted(async () => {
  await load();
  try {
    const s = await feishu.gatewayStatus();
    gwState.value = s.running ? "connected" : "stopped";
  } catch {
    /* 浏览器模式 */
  }
  unlistenStatus = await feishu.onGatewayStatus((state) => {
    gwState.value = state;
    if (state === "connected" || state === "stopped") gwBusy.value = false;
  });
  unlistenLog = await feishu.onGatewayLog((text) => pushLog(text));
});
onUnmounted(() => {
  if (unlistenLog) unlistenLog();
  if (unlistenStatus) unlistenStatus();
});
</script>

<template>
  <div class="fs-root">
    <div class="head">
      <div class="title-row">
        <div class="title">聊天机器人 · 飞书</div>
        <span class="status" :class="gwState">{{ stateLabel() }}</span>
      </div>
      <div class="sub">
        飞书里 <b>@机器人</b>（或私聊）的指令，会经长连接进来交给 <b>Claude Code</b>（在「飞书机器人」项目里、带工具执行、可操作软件），
        过程在 Polaris 里实时可见，结果发回飞书。
      </div>
    </div>

    <!-- 扫码去建应用 -->
    <div class="scan-box">
      <button class="scan-btn" @click="openScan">扫码前往建应用</button>
      <p class="scan-hint">飞书无「扫码自动下发凭证」能力，扫码带你到开放平台创建机器人</p>
    </div>

    <div class="divider">
      <span class="line"></span>
      <span class="dtext">或 手动填写、修改已有机器人信息</span>
      <span class="line"></span>
    </div>

    <ol class="guide">
      <li>飞书开放平台创建「企业自建应用」（机器人），开启「事件订阅 · 长连接」并订阅 <code>im.message.receive_v1</code></li>
      <li>权限里开启「读取与发送单聊/群组消息」；「凭证与基础信息」页拿到 App ID 与 App Secret</li>
      <li>把 App ID / Secret 填入下方保存，再启动下方「对话引擎」</li>
    </ol>
    <a class="guide-link" @click="openConsole">配置手册 / 打开开放平台 ↗</a>

    <div class="form">
      <label class="fld">
        <span>App ID</span>
        <div class="ip">
          <input v-model="cfg.appId" placeholder="cli_xxxxxxxx" spellcheck="false" />
          <button v-if="cfg.appId" class="clr" @click="clearField('appId')">✕</button>
        </div>
      </label>
      <label class="fld">
        <span>App Secret</span>
        <div class="ip">
          <input
            v-model="cfg.appSecret"
            :type="showSecret ? 'text' : 'password'"
            placeholder="留 ******** 表示不修改"
            spellcheck="false"
          />
          <button v-if="cfg.appSecret" class="clr" @click="clearField('appSecret')">✕</button>
          <button class="eye" @click="showSecret = !showSecret">{{ showSecret ? "🙈" : "👁" }}</button>
        </div>
        <em class="hint">从飞书开放平台「凭证与基础信息」获取</em>
      </label>

      <button class="adv-toggle" @click="showAdvanced = !showAdvanced">
        {{ showAdvanced ? "▾" : "▸" }} 高级设置（版本 / 私聊策略 / 群聊）
      </button>
      <div v-if="showAdvanced" class="adv">
        <label class="fld">
          <span>版本</span>
          <select v-model="cfg.domain">
            <option value="feishu">飞书（国内）</option>
            <option value="lark">Lark（国际）</option>
          </select>
        </label>
        <label class="fld">
          <span>私聊策略</span>
          <select v-model="cfg.dmPolicy">
            <option value="open">开放（任何人可私聊）</option>
            <option value="allowlist">白名单（仅下方 open_id）</option>
            <option value="disabled">关闭私聊</option>
          </select>
        </label>
        <label class="fld check">
          <input type="checkbox" v-model="cfg.groupRequireMention" />
          <span>群聊需 @机器人才响应（推荐）</span>
        </label>
        <label class="fld" v-if="cfg.dmPolicy === 'allowlist'">
          <span>白名单 open_id（每行一个）</span>
          <textarea v-model="allowText" rows="3" placeholder="ou_xxx" spellcheck="false"></textarea>
        </label>
      </div>
    </div>

    <div class="actions">
      <button class="btn primary" :disabled="saving" @click="save">{{ saving ? "保存中…" : "保存" }}</button>
      <button class="btn" :disabled="testing" @click="test">{{ testing ? "测试中…" : "连接测试" }}</button>
      <span v-if="savedMsg" class="saved">{{ savedMsg }}</span>
    </div>
    <div v-if="testResult" class="result" :class="{ ok: testResult.ok, err: !testResult.ok }">
      {{ testResult.message }}
      <span v-if="testResult.ok && testResult.botOpenId" class="oid">bot open_id: {{ testResult.botOpenId }}</span>
    </div>

    <!-- 对话引擎 -->
    <div class="engine">
      <div class="eng-head">
        <span class="eng-title">对话引擎</span>
        <span class="eng-state" :class="gwState">{{ stateLabel() }}</span>
      </div>
      <p class="eng-desc">
        启动后挂飞书长连接收发消息。内置<b>防断守护</b>（断线/崩溃自动重连重起），来消息才跑 Claude，空闲几乎零开销。首启会自动 npm 装飞书 SDK（需 Node.js）。
      </p>
      <label class="fld check auto">
        <input type="checkbox" v-model="cfg.autoStart" @change="toggleAutoStart" />
        <span>App 启动时自动开启网关（开机自动上线）</span>
      </label>
      <div class="actions">
        <button
          v-if="gwState === 'stopped'"
          class="btn primary"
          :disabled="gwBusy"
          @click="startGateway"
        >
          {{ gwBusy ? "启动中…" : "启动网关" }}
        </button>
        <button v-else class="btn" :disabled="gwBusy" @click="stopGateway">停止网关</button>
      </div>
      <div v-if="gwLog.length" class="eng-log">
        <div v-for="(l, i) in gwLog" :key="i" class="log-line">{{ l }}</div>
      </div>
    </div>

    <!-- 扫码弹窗 -->
    <div v-if="qrOpen" class="qr-mask" @click.self="closeScan">
      <div class="qr-card">
        <div class="qr-title">前往飞书开放平台建应用</div>
        <div class="qr-frame">
          <div v-if="qrLoading" class="qr-ph">正在生成二维码…</div>
          <div v-else-if="qrError" class="qr-ph err">{{ qrError }}</div>
          <div v-else class="qr-img" v-html="qrSvg"></div>
        </div>
        <p class="qr-desc">用<b>飞书</b>扫一扫前往开放平台创建机器人；建好后回到这里填 App ID 和 Secret。</p>
        <div class="qr-actions">
          <button class="btn" @click="openConsole">在浏览器打开 ↗</button>
          <button class="btn primary" @click="closeScan">我已创建好，去填凭证</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.fs-root {
  flex: 1;
  overflow-y: auto;
  padding: 22px 28px;
  background: var(--bg);
}
.title-row {
  display: flex;
  align-items: center;
  gap: 12px;
}
.head .title {
  font-family: var(--serif);
  font-size: 18px;
  letter-spacing: 2px;
  color: var(--ink);
}
.status {
  font-size: 11px;
  padding: 2px 9px;
  border-radius: 999px;
  background: var(--selection-bg);
  color: var(--muted);
}
.status.connected {
  background: rgba(74, 222, 128, 0.16);
  color: #2f8f50;
}
.status.starting,
.status.installing,
.status.reconnecting {
  background: rgba(245, 166, 35, 0.16);
  color: #b06f00;
}
.head .sub {
  font-size: 12.5px;
  color: var(--muted);
  margin-top: 6px;
  max-width: 640px;
  line-height: 1.6;
}

.scan-box {
  margin-top: 18px;
  max-width: 620px;
  border: 1px dashed var(--border);
  border-radius: 12px;
  padding: 22px 16px;
  text-align: center;
  background: var(--panel);
}
.scan-btn {
  padding: 10px 22px;
  border-radius: 10px;
  border: none;
  background: var(--primary);
  color: #fff;
  font-size: 14px;
  font-weight: 600;
  cursor: pointer;
}
.scan-btn:hover {
  opacity: 0.9;
}
.scan-hint {
  margin: 10px 0 0;
  font-size: 12px;
  color: var(--muted);
}

.divider {
  display: flex;
  align-items: center;
  gap: 12px;
  margin: 18px 0 14px;
  max-width: 620px;
}
.divider .line {
  flex: 1;
  height: 1px;
  background: var(--border-soft);
}
.divider .dtext {
  font-size: 12px;
  color: var(--muted);
  white-space: nowrap;
}
.guide {
  margin: 0;
  padding-left: 20px;
  max-width: 620px;
  color: var(--muted);
  font-size: 12.5px;
  line-height: 1.9;
}
.guide code {
  font-family: ui-monospace, Consolas, monospace;
  font-size: 11.5px;
}
.guide-link {
  display: inline-block;
  margin-top: 6px;
  font-size: 12.5px;
  color: var(--primary);
  cursor: pointer;
}
.guide-link:hover {
  text-decoration: underline;
}

.form {
  margin-top: 16px;
  display: flex;
  flex-direction: column;
  gap: 14px;
  max-width: 500px;
}
.fld {
  display: flex;
  flex-direction: column;
  gap: 5px;
}
.fld > span {
  font-size: 12px;
  color: var(--text-2);
}
.ip {
  position: relative;
  display: flex;
  align-items: center;
}
.ip input {
  flex: 1;
  padding: 8px 64px 8px 10px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--panel);
  color: var(--text);
  font-size: 13px;
}
.ip input:focus {
  outline: none;
  border-color: var(--primary);
}
.ip .clr,
.ip .eye {
  position: absolute;
  border: none;
  background: transparent;
  color: var(--muted);
  cursor: pointer;
  font-size: 13px;
  padding: 2px 4px;
}
.ip .clr {
  right: 34px;
}
.ip .eye {
  right: 6px;
}
.ip .clr:hover,
.ip .eye:hover {
  color: var(--primary);
}
.hint {
  font-size: 11.5px;
  color: var(--muted);
  font-style: normal;
}
.fld select,
.fld textarea {
  padding: 7px 10px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--panel);
  color: var(--text);
  font-size: 13px;
}
.fld select:focus,
.fld textarea:focus {
  outline: none;
  border-color: var(--primary);
}
.fld.check {
  flex-direction: row;
  align-items: center;
  gap: 8px;
}
.fld.check span {
  font-size: 13px;
  color: var(--text-2);
}
.adv-toggle {
  align-self: flex-start;
  border: none;
  background: transparent;
  color: var(--muted);
  font-size: 12.5px;
  cursor: pointer;
  padding: 2px 0;
}
.adv-toggle:hover {
  color: var(--text);
}
.adv {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 14px;
  border: 1px solid var(--border-soft);
  border-radius: 10px;
  background: var(--panel);
}
.actions {
  margin-top: 18px;
  display: flex;
  align-items: center;
  gap: 10px;
}
.btn {
  padding: 7px 16px;
  border-radius: 8px;
  border: 1px solid var(--border);
  background: var(--panel);
  color: var(--text);
  font-size: 13px;
  cursor: pointer;
}
.btn:hover {
  background: var(--selection-bg);
}
.btn.primary {
  background: var(--primary);
  border-color: var(--primary);
  color: #fff;
}
.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.saved {
  font-size: 12.5px;
  color: var(--primary);
}
.result {
  margin-top: 14px;
  padding: 10px 14px;
  border-radius: 8px;
  font-size: 13px;
  max-width: 620px;
}
.result.ok {
  background: rgba(74, 222, 128, 0.12);
  color: #2f8f50;
}
.result.err {
  background: rgba(248, 113, 113, 0.12);
  color: var(--vermilion);
}
.result .oid {
  display: block;
  margin-top: 4px;
  font-family: ui-monospace, Consolas, monospace;
  font-size: 11.5px;
  opacity: 0.8;
}

.engine {
  margin-top: 24px;
  padding-top: 18px;
  border-top: 1px solid var(--border-soft);
  max-width: 620px;
}
.eng-head {
  display: flex;
  align-items: center;
  gap: 10px;
}
.eng-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
}
.eng-state {
  font-size: 12px;
  color: var(--muted);
}
.eng-state.connected {
  color: #2f8f50;
}
.eng-state.starting,
.eng-state.installing,
.eng-state.reconnecting {
  color: #b06f00;
}
.eng-desc {
  margin: 8px 0 0;
  font-size: 12px;
  color: var(--muted);
  line-height: 1.7;
}
.fld.check.auto {
  margin-top: 12px;
}
.eng-log {
  margin-top: 12px;
  max-height: 180px;
  overflow-y: auto;
  background: #0e0f13;
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  padding: 10px 12px;
}
.log-line {
  font-family: ui-monospace, Consolas, monospace;
  font-size: 11.5px;
  line-height: 1.6;
  color: #c7ccd4;
  white-space: pre-wrap;
  word-break: break-all;
}

.qr-mask {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.55);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
}
.qr-card {
  width: 340px;
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 16px;
  padding: 24px;
  text-align: center;
  box-shadow: 0 20px 60px rgba(0, 0, 0, 0.4);
}
.qr-title {
  font-size: 15px;
  font-weight: 600;
  color: var(--text);
  margin-bottom: 16px;
}
.qr-frame {
  width: 240px;
  height: 240px;
  margin: 0 auto;
  border-radius: 12px;
  background: #fff;
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
}
.qr-img :deep(svg) {
  width: 220px;
  height: 220px;
  display: block;
}
.qr-ph {
  font-size: 12.5px;
  color: #999;
}
.qr-ph.err {
  color: var(--vermilion);
  padding: 0 16px;
}
.qr-desc {
  margin: 14px 0 16px;
  font-size: 12.5px;
  color: var(--muted);
  line-height: 1.6;
}
.qr-actions {
  display: flex;
  gap: 10px;
  justify-content: center;
}
</style>
