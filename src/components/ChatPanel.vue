<script setup lang="ts">
import { ref, onMounted, onUnmounted, nextTick, watch } from "vue";
import {
  chat,
  convApi,
  listen,
  type PermissionMode,
  type ChatStreamEvent,
} from "../tauri";
import { useAppStore } from "../stores/app";

interface Bubble {
  role: "user" | "assistant" | "tool";
  text: string;
  tool?: string;
}

const app = useAppStore();

const input = ref("");
const bubbles = ref<Bubble[]>([]);
const sending = ref(false);
const currentReq = ref<string | null>(null);
const showPermDropdown = ref(false);
const permMode = ref<PermissionMode>("manual");
const useSandbox = ref(false); // 默认关:很多人本机有 claude 但没启 Docker
const scrollEl = ref<HTMLDivElement | null>(null);

let unlisten: (() => void) | null = null;

const permLabel: Record<PermissionMode, string> = {
  manual: "手动授权",
  auto_current: "仅当前会话",
  auto_all: "所有会话",
  deny: "拒绝授权",
};

async function loadHistory(convId: string | null) {
  if (!convId) {
    bubbles.value = [];
    return;
  }
  try {
    const msgs = await convApi.getMessages(convId);
    bubbles.value = msgs.map((m) => ({
      role: m.role,
      text: m.content,
    }));
    await nextTick();
    if (scrollEl.value) scrollEl.value.scrollTop = scrollEl.value.scrollHeight;
  } catch (e: any) {
    bubbles.value = [];
  }
}

watch(
  () => app.currentConvId,
  (cid) => {
    loadHistory(cid);
  }
);

onMounted(async () => {
  unlisten = await listen<ChatStreamEvent>("chat:stream", (ev) => {
    if (!currentReq.value || ev.reqId !== currentReq.value) return;
    const last = bubbles.value[bubbles.value.length - 1];
    if (ev.kind === "delta") {
      if (last && last.role === "assistant") {
        last.text += ev.text ?? "";
      } else {
        bubbles.value.push({ role: "assistant", text: ev.text ?? "" });
      }
    } else if (ev.kind === "tool") {
      bubbles.value.push({
        role: "tool",
        text: `调用工具:${ev.tool ?? "(unknown)"}`,
        tool: ev.tool,
      });
    } else if (ev.kind === "error") {
      bubbles.value.push({
        role: "assistant",
        text: `[错误] ${ev.text ?? ""}`,
      });
    } else if (ev.kind === "done") {
      sending.value = false;
      currentReq.value = null;
    }
    nextTick(() => {
      if (scrollEl.value)
        scrollEl.value.scrollTop = scrollEl.value.scrollHeight;
    });
  });
  await loadHistory(app.currentConvId);
});
onUnmounted(() => {
  if (unlisten) unlisten();
});

async function ensureConversation(): Promise<string | null> {
  if (app.currentConvId) return app.currentConvId;
  let pid = app.currentProjectId;
  if (!pid) {
    await app.refreshProjects();
    pid = app.currentProjectId;
  }
  if (!pid) {
    const p = await app.createProject("默认项目");
    pid = p.id;
  }
  const c = await app.createConversation(pid);
  return c.id;
}

async function send() {
  const text = input.value.trim();
  if (!text || sending.value) return;

  const convId = await ensureConversation();

  bubbles.value.push({ role: "user", text });
  input.value = "";
  sending.value = true;
  try {
    const reqId = await chat.send({
      prompt: text,
      permissionMode: permMode.value,
      useSandbox: useSandbox.value,
      conversationId: convId ?? undefined,
    });
    currentReq.value = reqId;
  } catch (e: any) {
    bubbles.value.push({
      role: "assistant",
      text: `[发送失败] ${e?.message ?? e}`,
    });
    sending.value = false;
  }
}

async function cancel() {
  if (currentReq.value) {
    try {
      await chat.cancel(currentReq.value);
    } catch (_) {}
  }
  sending.value = false;
}

function pickPerm(m: PermissionMode) {
  permMode.value = m;
  showPermDropdown.value = false;
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
    e.preventDefault();
    send();
  }
}

async function newChat() {
  let pid = app.currentProjectId;
  if (!pid) {
    await app.refreshProjects();
    pid = app.currentProjectId;
  }
  if (!pid) {
    const p = await app.createProject("默认项目");
    pid = p.id;
  }
  await app.createConversation(pid);
}
</script>

<template>
  <div class="chat">
    <div class="chat-top">
      <div class="chat-title">
        <template v-if="app.currentConvId">
          <span class="t-glyph">●</span>
          <span class="t-text">{{
            (
              app.conversationsByProject[app.currentProjectId || ""] || []
            ).find((c) => c.id === app.currentConvId)?.title ||
            "(对话)"
          }}</span>
        </template>
        <template v-else>
          <span class="t-text muted">未选择对话</span>
        </template>
      </div>
      <button class="new-chat-btn" @click="newChat" title="新建对话">
        + 新建对话
      </button>
    </div>

    <div class="messages" ref="scrollEl">
      <div v-if="bubbles.length === 0" class="hero-wrap">
        <div class="hero">你说,北极星画</div>
        <div class="hero-sub">
          本地优先 · 调用 Claude Code · 维基知识库 KB-first 召回
        </div>
        <div class="hero-tips">
          <div>· <strong>对话历史会自动保存到当前项目</strong></div>
          <div>· 默认走宿主机 <code>claude</code>(已检测安装);切到沙箱前需先在「沙箱」页启动容器</div>
          <div>· 首次用 <code>claude</code> 请确认已 <code>claude login</code></div>
        </div>
      </div>

      <div v-for="(b, i) in bubbles" :key="i" class="bubble" :class="b.role">
        <div class="who">
          <template v-if="b.role === 'user'">你</template>
          <template v-else-if="b.role === 'tool'">⚙ 工具</template>
          <template v-else>北极星</template>
        </div>
        <div class="text">{{ b.text }}</div>
      </div>
    </div>

    <div class="input-wrap">
      <div class="input-card">
        <textarea
          v-model="input"
          placeholder="请输入消息(Ctrl + Enter 发送) …"
          rows="3"
          @keydown="onKeydown"
        ></textarea>
        <div class="bottom">
          <button
            class="btn"
            :class="{ active: useSandbox }"
            @click="useSandbox = !useSandbox"
            title="走 Docker 沙箱内的 claude;关闭则用宿主机 claude.exe"
          >
            ⛨ 沙箱执行
          </button>
          <div class="spacer"></div>
          <div class="perm-wrap">
            <button
              class="btn perm"
              :class="{ deny: permMode === 'deny' }"
              @click="showPermDropdown = !showPermDropdown"
            >
              <span class="perm-ic">{{
                permMode === "deny" ? "⊘" : "✋"
              }}</span>
              {{ permLabel[permMode] }} ▾
            </button>
            <div v-if="showPermDropdown" class="dropdown">
              <div
                v-for="m in [
                  { k: 'manual', l: '手动授权', d: '每次工具调用前确认' },
                  {
                    k: 'auto_current',
                    l: '自动 · 仅当前会话',
                    d: '本会话放行非高危操作',
                  },
                  {
                    k: 'auto_all',
                    l: '自动 · 所有会话',
                    d: '全局放行非高危(含定时任务)',
                  },
                  {
                    k: 'deny',
                    l: '拒绝授权(只读)',
                    d: '禁止写入/执行,只允许 Read/Grep/Glob',
                  },
                ]"
                :key="m.k"
                class="perm-row"
                :class="{ active: permMode === m.k, deny: m.k === 'deny' }"
                @click="pickPerm(m.k as PermissionMode)"
              >
                <div class="title">{{ m.l }}</div>
                <div class="desc">{{ m.d }}</div>
              </div>
            </div>
          </div>
          <button
            v-if="sending"
            class="send-btn stop"
            title="停止"
            @click="cancel"
          >
            ⏹
          </button>
          <button
            v-else
            class="send-btn"
            title="发送 (Ctrl+Enter)"
            :disabled="!input.trim()"
            @click="send"
          >
            ↑
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.chat {
  display: flex;
  flex-direction: column;
  height: 100vh;
}
.chat-top {
  padding: 12px 24px;
  display: flex;
  align-items: center;
  gap: 12px;
  border-bottom: 1px solid var(--hairline);
  background: var(--bg);
}
.chat-title {
  flex: 1;
  display: flex;
  align-items: center;
  gap: 8px;
  font-family: var(--serif);
}
.t-glyph {
  color: var(--primary);
  font-size: 9px;
}
.t-text {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
}
.t-text.muted {
  font-weight: 400;
  color: var(--muted);
}
.new-chat-btn {
  padding: 5px 12px;
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 4px;
  font-size: 12px;
  color: var(--text-2);
}
.new-chat-btn:hover {
  border-color: var(--primary);
  color: var(--primary);
}

.messages {
  flex: 1;
  overflow-y: auto;
  padding: 40px 32px 16px;
}
.hero-wrap {
  margin: 60px auto 40px;
  text-align: center;
  max-width: 720px;
}
.hero {
  font-family: var(--serif);
  font-size: 36px;
  font-weight: 600;
  letter-spacing: 4px;
  color: var(--ink);
}
.hero-sub {
  margin-top: 16px;
  color: var(--muted);
  font-size: 13px;
  letter-spacing: 0.5px;
}
.hero-tips {
  margin-top: 28px;
  font-size: 12px;
  color: var(--muted);
  line-height: 2;
  text-align: left;
  display: inline-block;
}
.hero-tips code {
  background: var(--code-bg);
  padding: 1px 5px;
  border-radius: 2px;
  font-family: var(--mono);
  font-size: 11px;
}

.bubble {
  max-width: 820px;
  margin: 0 auto 14px;
  background: var(--panel);
  border: 1px solid var(--hairline);
  border-radius: 6px;
  padding: 14px 18px;
  box-shadow: var(--shadow-sm);
}
.bubble.user {
  background: var(--primary-soft);
  border-color: rgba(44, 70, 97, 0.12);
}
.bubble.tool {
  background: var(--bg-soft);
  border-color: var(--border-soft);
  font-family: var(--mono);
  font-size: 12px;
  color: var(--ink-2);
}
.who {
  font-family: var(--serif);
  font-size: 11px;
  letter-spacing: 1.5px;
  color: var(--muted);
  margin-bottom: 4px;
}
.bubble.user .who {
  color: var(--primary-deep);
}
.text {
  white-space: pre-wrap;
  word-break: break-word;
  font-size: 13.5px;
  color: var(--text);
}

.input-wrap {
  padding: 12px 32px 24px;
  display: flex;
  justify-content: center;
}
.input-card {
  width: 100%;
  max-width: 820px;
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 10px;
  box-shadow: var(--shadow);
  padding: 12px 14px;
}
textarea {
  width: 100%;
  border: none;
  outline: none;
  resize: none;
  font-size: 13.5px;
  background: transparent;
  color: var(--text);
  padding: 4px 0;
  line-height: 1.7;
}

.bottom {
  display: flex;
  align-items: center;
  gap: 4px;
  flex-wrap: wrap;
  margin-top: 6px;
}
.btn {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 6px 10px;
  border-radius: 4px;
  font-size: 12.5px;
  color: var(--text-2);
  border: none;
  background: transparent;
}
.btn:hover {
  background: var(--selection-bg);
}
.btn.active {
  background: var(--selection-bg);
  color: var(--text);
  font-weight: 500;
}
.btn.perm {
  background: var(--bg-soft);
  color: var(--text);
  border: 1px solid var(--border);
}
.btn.perm:hover {
  border-color: var(--primary);
}
.btn.perm.deny {
  color: var(--vermilion);
  border-color: rgba(192, 57, 43, 0.3);
}
.perm-ic {
  color: var(--primary);
}
.btn.perm.deny .perm-ic {
  color: var(--vermilion);
}
.spacer {
  flex: 1;
}
.perm-wrap {
  position: relative;
}
.dropdown {
  position: absolute;
  right: 0;
  bottom: 38px;
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 8px;
  box-shadow: var(--shadow-lg);
  width: 320px;
  padding: 6px;
  z-index: 10;
}
.perm-row {
  padding: 8px 10px;
  border-radius: 6px;
  cursor: pointer;
}
.perm-row:hover {
  background: var(--selection-bg);
}
.perm-row.active {
  background: var(--selection-bg);
}
.perm-row.deny .title {
  color: var(--vermilion);
}
.perm-row .title {
  font-size: 13px;
  color: var(--text);
  font-weight: 600;
}
.perm-row .desc {
  font-size: 11.5px;
  color: var(--muted);
  margin-top: 2px;
  line-height: 1.5;
}

.send-btn {
  width: 32px;
  height: 32px;
  background: var(--ink);
  color: #fafaf7;
  border: none;
  border-radius: 50%;
  font-size: 16px;
  margin-left: 4px;
}
.send-btn:disabled {
  background: var(--border-strong);
  cursor: not-allowed;
}
.send-btn.stop {
  background: var(--vermilion);
}
</style>
