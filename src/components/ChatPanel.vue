<script setup lang="ts">
import { ref, onMounted, onUnmounted, nextTick, watch } from "vue";
import {
  Puzzle,
  Search,
  ChevronDown,
  X,
  ArrowRight,
  Square,
  Sparkles,
  Globe,
  Wrench,
  FileText,
  Table,
  AudioLines,
  Clapperboard,
  Image as ImageIcon,
  Ghost,
  FileCode,
  File as FileIcon,
  ExternalLink,
  Paperclip,
  LoaderCircle,
} from "@lucide/vue";
import {
  chat,
  convApi,
  listen,
  skills as skillsApi,
  type PermissionMode,
  type ChatStreamEvent,
  type Skill,
  type AttachedFile,
} from "../tauri";
import { useAppStore } from "../stores/app";
import { useSkillsStore } from "../stores/skills";
import { useArtifactsStore } from "../stores/artifacts";
import { useFileDrop } from "../composables/useFileDrop";

interface Bubble {
  role: "user" | "assistant" | "tool";
  text: string;
  tool?: string;
  /** 本条 assistant 消息生成的成品文件（绝对路径，正斜杠） */
  artifacts?: string[];
  /** 本条 user 消息携带的上传附件 */
  files?: AttachedFile[];
}

/** 解析正文里夹带的产物清单 marker，返回剥离 marker 后的纯文本 + 路径数组 */
function parseArtifacts(content: string): { text: string; artifacts: string[] } {
  const m = content.match(/<!--POLARIS_ARTIFACTS:(\[[\s\S]*?\])-->/);
  if (!m) return { text: content, artifacts: [] };
  let arr: string[] = [];
  try {
    arr = JSON.parse(m[1]);
  } catch {
    arr = [];
  }
  const text = content.replace(m[0], "").trimEnd();
  return { text, artifacts: arr };
}

function fileName(path: string): string {
  return path.split("/").pop() || path;
}

function fileExt(path: string): string {
  const n = fileName(path);
  const i = n.lastIndexOf(".");
  return i >= 0 ? n.slice(i + 1).toLowerCase() : "";
}

function artifactIcon(path: string) {
  const ext = fileExt(path);
  if (["html", "htm", "svg", "js", "ts", "css", "json", "xml"].includes(ext))
    return FileCode;
  if (["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "avif"].includes(ext))
    return ImageIcon;
  if (["csv", "tsv", "xlsx", "xls"].includes(ext)) return Table;
  if (["md", "markdown", "txt", "pdf"].includes(ext)) return FileText;
  return FileIcon;
}

const app = useAppStore();
const skillsStore = useSkillsStore();
const artifactsStore = useArtifactsStore();

/** 点击成品文件 chip → 展开右侧抽屉并预览 */
function openArtifact(path: string) {
  app.drawerCollapsed = false;
  artifactsStore.open(path);
}

const input = ref("");
const bubbles = ref<Bubble[]>([]);
const sending = ref(false);
const currentReq = ref<string | null>(null);
const showPermDropdown = ref(false);
const permMode = ref<PermissionMode>("manual");
const showSkillPanel = ref(false);
const skillSearch = ref("");
const skillsList = ref<Skill[]>([]);
const scrollEl = ref<HTMLDivElement | null>(null);

let unlisten: (() => void) | null = null;

// ─────────── 拖拽上传附件到当前对话 ───────────
const attachments = ref<AttachedFile[]>([]);
/** 上传中的占位（大文件复制需要时间，显示转圈） */
const pendingAttach = ref<{ name: string }[]>([]);

function attachIcon(kind: string) {
  if (kind === "image") return ImageIcon;
  if (kind === "pdf") return FileText;
  if (kind === "office") return Table;
  if (kind === "text") return FileCode;
  return FileIcon;
}

function humanSize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

async function onDropFiles(paths: string[]) {
  const convId = await ensureConversation();
  const placeholders = paths.map((p) => ({
    name: p.split(/[\\/]/).pop() || p,
  }));
  pendingAttach.value.push(...placeholders);
  try {
    const res = await chat.attachFiles(convId ?? undefined, paths);
    for (const r of res) {
      if (r.ok) attachments.value.push(r);
      else
        bubbles.value.push({
          role: "assistant",
          text: `[附件失败] ${r.name}:${r.error ?? ""}`,
        });
    }
  } catch (e: any) {
    bubbles.value.push({
      role: "assistant",
      text: `[附件失败] ${e?.message ?? e}`,
    });
  } finally {
    for (const ph of placeholders) {
      const idx = pendingAttach.value.indexOf(ph);
      if (idx >= 0) pendingAttach.value.splice(idx, 1);
    }
  }
}

function removeAttachment(i: number) {
  attachments.value.splice(i, 1);
}

const { isOver: dropOver } = useFileDrop({
  active: () => app.view === "chat",
  onDrop: onDropFiles,
});

const permLabel: Record<PermissionMode, string> = {
  manual: "手动授权",
  auto_current: "自动 · 仅当前会话",
  auto_all: "自动 · 所有会话",
  deny: "拒绝授权",
};

// Load skills for panel
async function loadSkills() {
  try {
    skillsList.value = await skillsApi.list();
  } catch {
    skillsList.value = [
      {
        id: "deep-research",
        name: "深度搜索",
        description:
          "使用 LLM 大规模联网搜索相关内容，自动检索、汇总、交叉验证多来源信息",
        source: "third-party",
      },
      {
        id: "skill-creator",
        name: "Skill 创建向导",
        description: "引导用户创建自定义 Skill，自动生成模板和配置文件",
        source: "official",
      },
    ];
  }
}

function filteredSkills() {
  if (!skillSearch.value.trim()) return skillsList.value;
  const q = skillSearch.value.toLowerCase();
  return skillsList.value.filter(
    (s) =>
      s.name.toLowerCase().includes(q) ||
      s.description.toLowerCase().includes(q)
  );
}

function skillIcon(id: string) {
  const map: Record<string, any> = {
    "deep-research": Globe,
    "skill-creator": Wrench,
    pdf: FileText,
    xlsx: Table,
    "edge-tts": AudioLines,
    hyperframes: Clapperboard,
    "web-search": Search,
    "image-gen": ImageIcon,
    "cloak-browser": Ghost,
  };
  return map[id] ?? Sparkles;
}

function goToSkillCenter() {
  showSkillPanel.value = false;
  app.setView("skill_center");
}

function toggleSkill(id: string) {
  skillsStore.toggle(id);
  showSkillPanel.value = false;
}

function clearActiveSkill(id: string) {
  skillsStore.remove(id);
}

async function loadHistory(convId: string | null) {
  if (!convId) {
    bubbles.value = [];
    return;
  }
  try {
    const msgs = await convApi.getMessages(convId);
    bubbles.value = msgs.map((m) => {
      if (m.role === "assistant") {
        const { text, artifacts } = parseArtifacts(m.content);
        return { role: m.role, text, artifacts };
      }
      return { role: m.role, text: m.content };
    });
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
    } else if (ev.kind === "artifact") {
      const path = ev.text;
      if (path) {
        // 挂到最近一个 assistant 气泡上（tool 气泡可能夹在中间）
        let target: Bubble | undefined;
        for (let i = bubbles.value.length - 1; i >= 0; i--) {
          if (bubbles.value[i].role === "assistant") {
            target = bubbles.value[i];
            break;
          }
        }
        if (!target) {
          target = { role: "assistant", text: "", artifacts: [] };
          bubbles.value.push(target);
        }
        if (!target.artifacts) target.artifacts = [];
        if (!target.artifacts.includes(path)) target.artifacts.push(path);
      }
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
  await loadSkills();
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
  const attached = attachments.value.slice();
  const hasAttach = attached.length > 0;
  if ((!text && !hasAttach) || sending.value) return;

  const convId = await ensureConversation();

  // 把附件绝对路径拼进 prompt，让 claude 能用 Read 等工具读取
  let prompt = text || "请查看我上传的附件。";
  if (hasAttach) {
    const lines = attached.map((a) => `- ${a.path}`).join("\n");
    prompt += `\n\n---\n[附件]（用户拖拽上传，可用 Read 等工具读取）：\n${lines}`;
  }

  bubbles.value.push({
    role: "user",
    text: text || "（仅附件）",
    files: hasAttach ? attached : undefined,
  });
  input.value = "";
  attachments.value = [];
  sending.value = true;
  try {
    const reqId = await chat.send({
      prompt,
      permissionMode: permMode.value,
      skillIds: Array.from(skillsStore.enabledSkills),
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
  <div class="chat" :class="{ 'drag-active': dropOver }">
    <!-- 拖拽上传覆盖层 -->
    <div v-if="dropOver" class="drop-overlay">
      <div class="drop-card">
        <Paperclip :size="30" :stroke-width="1.4" />
        <div class="drop-title">松开以上传到当前对话</div>
        <div class="drop-sub">文件作为附件，发送时供 Claude 读取</div>
      </div>
    </div>
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
          <div>
            · <strong>对话历史会自动保存到当前项目</strong>
          </div>
          <div>
            · 默认走宿主机 <code>claude</code>(已检测安装)
          </div>
          <div>· 首次用 <code>claude</code> 请确认已 <code>claude login</code></div>
        </div>
      </div>

      <div
        v-for="(b, i) in bubbles"
        :key="i"
        class="bubble"
        :class="b.role"
      >
        <div class="who">
          <template v-if="b.role === 'user'">你</template>
          <template v-else-if="b.role === 'tool'">⚙ 工具</template>
          <template v-else>北极星</template>
        </div>
        <div v-if="b.text" class="text">{{ b.text }}</div>
        <!-- 用户上传的附件 -->
        <div
          v-if="b.role === 'user' && b.files && b.files.length"
          class="attach-chips in-bubble"
        >
          <div
            v-for="f in b.files"
            :key="f.path"
            class="attach-chip readonly"
            :title="f.path"
          >
            <component :is="attachIcon(f.kind)" :size="14" :stroke-width="1.7" />
            <span class="ac-name">{{ f.name }}</span>
          </div>
        </div>
        <!-- 成品文件：点击在右侧抽屉预览 -->
        <div
          v-if="b.role === 'assistant' && b.artifacts && b.artifacts.length"
          class="artifacts"
        >
          <button
            v-for="a in b.artifacts"
            :key="a"
            class="artifact-chip"
            :class="{ active: artifactsStore.current?.path === a }"
            :title="a"
            @click="openArtifact(a)"
          >
            <component :is="artifactIcon(a)" :size="15" :stroke-width="1.7" />
            <span class="af-name">{{ fileName(a) }}</span>
            <ExternalLink :size="12" :stroke-width="1.8" class="af-open" />
          </button>
        </div>
      </div>
    </div>

    <!-- 输入区域 -->
    <div class="input-area">
      <!-- 技能选择弹窗 -->
      <div v-if="showSkillPanel" class="skill-panel">
        <div class="skill-panel-head">
          <span class="skill-panel-title">选择技能</span>
          <button class="skill-panel-close" @click="showSkillPanel = false">
            <X :size="14" :stroke-width="2" />
          </button>
        </div>
        <div class="skill-panel-search">
          <Search :size="14" :stroke-width="1.8" class="sp-search-icon" />
          <input v-model="skillSearch" placeholder="搜索技能..." type="text" />
        </div>
        <div class="skill-panel-list">
          <div
            v-for="s in filteredSkills()"
            :key="s.id"
            class="skill-panel-item"
            :class="{ active: skillsStore.has(s.id) }"
            @click="toggleSkill(s.id)"
          >
            <component
              :is="skillIcon(s.id)"
              :size="16"
              :stroke-width="1.6"
              class="sp-item-icon"
            />
            <div class="sp-item-info">
              <div class="sp-item-name">{{ s.name }}</div>
              <div class="sp-item-desc">{{ s.description }}</div>
            </div>
          </div>
        </div>
        <div class="skill-panel-foot">
          <button class="sp-manage" @click="goToSkillCenter">
            <ArrowRight :size="12" :stroke-width="2" />
            <span>探索和管理技能</span>
          </button>
        </div>
      </div>

      <!-- 输入卡片 -->
      <div class="input-card">
        <!-- Skill 标签 -->
        <div v-if="skillsStore.enabledSkills.size > 0" class="skill-tags">
          <div
            v-for="s in skillsList.filter((x) => skillsStore.has(x.id))"
            :key="s.id"
            class="skill-tag"
            @click="clearActiveSkill(s.id)"
          >
            <component :is="skillIcon(s.id)" :size="12" :stroke-width="1.8" />
            <span>{{ s.name }}</span>
            <X :size="10" :stroke-width="2" class="tag-close" />
          </div>
        </div>
        <!-- 待发送附件 -->
        <div
          v-if="attachments.length || pendingAttach.length"
          class="attach-chips"
        >
          <div
            v-for="(f, i) in attachments"
            :key="f.path"
            class="attach-chip"
            :title="f.path"
          >
            <component :is="attachIcon(f.kind)" :size="14" :stroke-width="1.7" />
            <span class="ac-name">{{ f.name }}</span>
            <span class="ac-size">{{ humanSize(f.size) }}</span>
            <button class="ac-remove" title="移除" @click="removeAttachment(i)">
              <X :size="11" :stroke-width="2" />
            </button>
          </div>
          <div
            v-for="(p, i) in pendingAttach"
            :key="'pending-' + i"
            class="attach-chip pending"
            :title="p.name"
          >
            <LoaderCircle :size="14" :stroke-width="2" class="spin" />
            <span class="ac-name">{{ p.name }}</span>
          </div>
        </div>
        <textarea
          v-model="input"
          placeholder="请输入消息 (Ctrl + Enter 发送，可拖文件进来作为附件) …"
          rows="3"
          @keydown="onKeydown"
        ></textarea>
        <div class="toolbar">
          <div class="toolbar-left">
            <button
              class="toolbar-btn"
              :class="{ active: showSkillPanel }"
              @click="showSkillPanel = !showSkillPanel"
            >
              <Puzzle :size="14" :stroke-width="1.8" />
              <span>技能</span>
            </button>
            <button
              class="toolbar-btn"
              :class="{ active: skillsStore.has('deep-research') }"
              @click="toggleSkill('deep-research')"
            >
              <Search :size="14" :stroke-width="1.8" />
              <span>深度搜索</span>
              <div class="btn-tooltip">
                <div class="btn-tooltip-inner">
                  使用 LLM 大规模联网搜索相关内容
                  <div class="btn-tooltip-sub">
                    激活后 Claude 会自动检索多来源信息并交叉验证
                  </div>
                </div>
              </div>
            </button>
          </div>
          <div class="toolbar-right">
            <button
              v-if="sending"
              class="send-btn stop"
              title="停止"
              @click="cancel"
            >
              <Square :size="14" :stroke-width="2" fill="currentColor" />
            </button>
            <button
              v-else
              class="send-btn"
              title="发送 (Ctrl+Enter)"
              :disabled="!input.trim()"
              @click="send"
            >
              <ArrowRight :size="16" :stroke-width="2" />
            </button>
          </div>
        </div>
      </div>

      <!-- 底部授权栏 -->
      <div class="auth-bar">
        <div class="perm-wrap" style="margin-right: 48px;">
          <button
            class="auth-btn"
            :class="{ deny: permMode === 'deny' }"
            @click="showPermDropdown = !showPermDropdown"
          >
            <img
              v-if="permMode !== 'deny'"
              src="../assets/perm-hand.png"
              class="auth-hand"
              alt="授权"
            />
            <span v-else class="auth-deny">⊘</span>
            <span class="auth-label">{{ permLabel[permMode] }}</span>
            <ChevronDown :size="12" :stroke-width="2" />
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
                  d: '所有会话放行非高危操作(不绕过权限确认)',
                },
                {
                  k: 'deny',
                  l: '拒绝授权(只读)',
                  d: '禁止写入/执行,只允许 Read/Grep/Glob',
                },
              ]"
              :key="m.k"
              class="perm-row"
              :class="{
                active: permMode === m.k,
                deny: m.k === 'deny',
              }"
              @click="pickPerm(m.k as PermissionMode)"
            >
              <div class="title">{{ m.l }}</div>
              <div class="desc">{{ m.d }}</div>
            </div>
          </div>
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
  position: relative;
}
.chat-top {
  padding: 12px 24px;
  display: flex;
  align-items: center;
  gap: 12px;
  border-bottom: 1px solid var(--border);
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
  cursor: pointer;
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
  background: var(--bg-soft);
  padding: 1px 5px;
  border-radius: 2px;
  font-family: var(--mono);
  font-size: 11px;
}

.bubble {
  max-width: 820px;
  margin: 0 auto 14px;
  background: var(--panel);
  border: 1px solid var(--border);
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
  color: var(--text-2);
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
  line-height: 1.6;
}

/* 成品文件 chips —— 回答末尾的可点击文件 */
.artifacts {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 10px;
}
.artifact-chip {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  max-width: 320px;
  padding: 6px 10px;
  background: var(--bg-soft);
  border: 1px solid var(--border);
  border-radius: 8px;
  color: var(--primary);
  font-size: 12.5px;
  cursor: pointer;
  transition: border-color 0.15s, background 0.15s;
}
.artifact-chip:hover {
  border-color: var(--primary);
  background: var(--primary-soft);
}
.artifact-chip.active {
  border-color: var(--primary);
  background: var(--primary-soft);
}
.artifact-chip .af-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 500;
}
.artifact-chip .af-open {
  opacity: 0.5;
  flex-shrink: 0;
}
.artifact-chip:hover .af-open {
  opacity: 0.9;
}

/* ─────────── 输入区域 ─────────── */
.input-area {
  padding: 12px 32px 16px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  position: relative;
}

/* 技能选择弹窗 */
.skill-panel {
  position: absolute;
  bottom: calc(100% - 8px);
  left: 32px;
  width: 360px;
  max-height: 420px;
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 12px;
  box-shadow: var(--shadow-lg);
  z-index: 30;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.skill-panel-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 14px 8px;
  border-bottom: 1px solid var(--border-soft);
}
.skill-panel-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
}
.skill-panel-close {
  width: 24px;
  height: 24px;
  border: none;
  background: transparent;
  color: var(--muted);
  border-radius: 4px;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}
.skill-panel-close:hover {
  background: var(--bg-soft);
  color: var(--text);
}
.skill-panel-search {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 10px 14px;
  padding: 6px 10px;
  background: var(--bg-soft);
  border: 1px solid var(--border-soft);
  border-radius: 6px;
}
.sp-search-icon {
  color: var(--muted);
  flex-shrink: 0;
}
.skill-panel-search input {
  border: none;
  outline: none;
  background: transparent;
  font-size: 12.5px;
  color: var(--text);
  width: 100%;
}
.skill-panel-search input::placeholder {
  color: var(--dim);
}
.skill-panel-list {
  flex: 1;
  overflow-y: auto;
  padding: 0 6px;
}
.skill-panel-item {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 8px 10px;
  border-radius: 6px;
  cursor: pointer;
}
.skill-panel-item:hover {
  background: var(--bg-soft);
}
.skill-panel-item.active {
  background: var(--primary-soft);
}
.sp-item-icon {
  color: var(--primary);
  margin-top: 1px;
  flex-shrink: 0;
}
.sp-item-info {
  flex: 1;
  min-width: 0;
}
.sp-item-name {
  font-size: 13px;
  font-weight: 500;
  color: var(--text);
}
.sp-item-desc {
  font-size: 11px;
  color: var(--muted);
  margin-top: 2px;
  line-height: 1.4;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.skill-panel-foot {
  padding: 8px 14px;
  border-top: 1px solid var(--border-soft);
}
.sp-manage {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 6px 12px;
  background: transparent;
  border: none;
  color: var(--primary);
  font-size: 12.5px;
  border-radius: 4px;
  cursor: pointer;
}
.sp-manage:hover {
  background: var(--primary-soft);
}

/* 输入卡片 */
.input-card {
  width: 100%;
  max-width: 820px;
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 12px;
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

/* 工具栏 */
.toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-top: 8px;
  padding-top: 8px;
  border-top: 1px solid var(--border-soft);
}
.toolbar-left {
  display: flex;
  align-items: center;
  gap: 6px;
}
.toolbar-btn {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 5px 10px;
  border-radius: 6px;
  font-size: 12px;
  color: var(--text-2);
  border: none;
  background: transparent;
  cursor: pointer;
  position: relative;
}
.toolbar-btn:hover {
  background: var(--bg-soft);
  color: var(--text);
}
.toolbar-btn.active {
  background: var(--primary-soft);
  color: var(--primary);
}

/* Tooltip — 放在按钮下方，避免顶部穿模 */
.btn-tooltip {
  position: absolute;
  top: calc(100% + 6px);
  left: 50%;
  transform: translateX(-50%);
  z-index: 25;
  opacity: 0;
  pointer-events: none;
  transition: opacity 0.15s;
}
.toolbar-btn:hover .btn-tooltip {
  opacity: 1;
}
.btn-tooltip-inner {
  background: var(--ink);
  color: #fafaf7;
  padding: 8px 12px;
  border-radius: 8px;
  font-size: 12px;
  white-space: nowrap;
  line-height: 1.5;
}
.btn-tooltip-sub {
  font-size: 11px;
  color: var(--dim);
}

/* Skill 标签 — 蓝色链接样式 */
.skill-tags {
  display: flex;
  gap: 12px;
  margin-bottom: 8px;
  padding: 0 2px;
  flex-wrap: wrap;
}
.skill-tag {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 12.5px;
  color: var(--primary);
  cursor: pointer;
  transition: opacity 0.15s;
}
.skill-tag:hover {
  opacity: 0.7;
  text-decoration: underline;
}
.tag-close {
  opacity: 0.5;
  width: 12px;
  height: 12px;
}

.toolbar-right {
  display: flex;
  align-items: center;
  gap: 6px;
}
.send-btn {
  width: 32px;
  height: 32px;
  background: var(--ink);
  color: #fafaf7;
  border: none;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}
.send-btn:hover {
  background: var(--primary);
}
.send-btn:disabled {
  background: var(--border);
  cursor: not-allowed;
}
.send-btn.stop {
  background: var(--vermilion);
}

/* ─────────── 底部授权栏 ─────────── */
.auth-bar {
  width: 100%;
  max-width: 820px;
  display: flex;
  justify-content: flex-end;
}
.perm-wrap {
  position: relative;
}
.auth-btn {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 4px 10px;
  border-radius: 6px;
  font-size: 12px;
  color: var(--text-2);
  border: 1px solid var(--border-soft);
  background: transparent;
  cursor: pointer;
}
.auth-btn:hover {
  border-color: var(--border);
  color: var(--text);
}
.auth-btn.deny {
  color: var(--vermilion);
  border-color: rgba(192, 57, 43, 0.2);
}
.auth-hand {
  width: 13px;
  height: 13px;
  object-fit: contain;
}
.auth-deny {
  color: var(--vermilion);
}
.auth-label {
  margin-right: 2px;
}

/* 授权下拉菜单 — 向上展开 */
.dropdown {
  position: absolute;
  right: 0;
  bottom: calc(100% + 6px);
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 8px;
  box-shadow: var(--shadow-lg);
  width: 280px;
  padding: 6px;
  z-index: 20;
}
.perm-row {
  padding: 8px 10px;
  border-radius: 6px;
  cursor: pointer;
}
.perm-row:hover {
  background: var(--bg-soft);
}
.perm-row.active {
  background: var(--primary-soft);
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

/* ─────────── 拖拽上传覆盖层 ─────────── */
.drop-overlay {
  position: absolute;
  inset: 10px;
  z-index: 50;
  background: rgba(44, 70, 97, 0.06);
  border: 2px dashed var(--primary);
  border-radius: 14px;
  display: flex;
  align-items: center;
  justify-content: center;
  backdrop-filter: blur(1px);
  pointer-events: none;
}
.drop-card {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  color: var(--primary);
}
.drop-title {
  font-family: var(--serif);
  font-size: 16px;
  font-weight: 600;
  letter-spacing: 1px;
}
.drop-sub {
  font-size: 12px;
  color: var(--muted);
}

/* ─────────── 附件 chips ─────────── */
.attach-chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-bottom: 8px;
}
.attach-chips.in-bubble {
  margin-top: 8px;
  margin-bottom: 0;
}
.attach-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  max-width: 260px;
  padding: 4px 8px;
  background: var(--bg-soft);
  border: 1px solid var(--border);
  border-radius: 8px;
  font-size: 12px;
  color: var(--text-2);
}
.attach-chip .ac-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 500;
  color: var(--text);
}
.attach-chip .ac-size {
  color: var(--dim);
  font-size: 11px;
  flex-shrink: 0;
}
.attach-chip.readonly {
  background: transparent;
  color: var(--primary-deep);
}
.attach-chip.pending {
  color: var(--muted);
}
.ac-remove {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  border: none;
  background: transparent;
  color: var(--muted);
  border-radius: 4px;
  cursor: pointer;
  flex-shrink: 0;
}
.ac-remove:hover {
  background: var(--border);
  color: var(--text);
}
.spin {
  animation: spin 0.9s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
