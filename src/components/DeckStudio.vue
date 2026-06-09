<script setup lang="ts">
import { ref, computed, watch, onUnmounted } from "vue";
import {
  Presentation,
  FileText,
  Loader,
  Sparkles,
  Upload,
  X,
  Eye,
  FolderOpen,
  ExternalLink,
  Monitor,
  FileType2,
  Zap,
  Wand2,
  RefreshCw,
} from "@lucide/vue";
import { useAppStore } from "../stores/app";
import { useChatStore } from "../stores/chat";
import { artifacts as artifactsApi, chat as chatApi, type AttachedFile } from "../tauri";
import { useFileDrop } from "../composables/useFileDrop";
import { groupedThemes, findTheme, type DeckTheme } from "../lib/deckThemes";

const app = useAppStore();
const chat = useChatStore();

const STUDIO_PROJECT_NAME = "演示工坊";
const VIEW_KEY = "deck";

const outputMode = ref<"html" | "pptx">("html");
const isPpt = computed(() => outputMode.value === "pptx");

type Phase = "config" | "generating" | "done";
const phase = ref<Phase>("config");
const error = ref<string | null>(null);
const convId = ref<string | null>(null);
const lastAction = ref<"create" | "revise">("create");

// ───────── 配置 ─────────
const contentText = ref("");
const charCount = computed(() => contentText.value.length);
const uploads = ref<AttachedFile[]>([]);
const uploading = ref(false);

const selectedTheme = ref("minimal-white");
const groups = groupedThemes(true);
const curTheme = computed<DeckTheme>(() => findTheme(selectedTheme.value));

const slideCount = ref(12);
const autoSlides = ref(false); // AI 自己决定页数
const aspect = ref<"16:9" | "4:3">("16:9");
type Density = "low" | "med" | "high";
const density = ref<Density>("med");
const DENSITIES: { id: Density; label: string; hint: string }[] = [
  { id: "low", label: "极简", hint: "每页一句话 · 大字 · 演讲投影型" },
  { id: "med", label: "适中", hint: "标题 + 3-4 个要点 · 通用" },
  { id: "high", label: "信息密", hint: "图表/对比/多卡片 · 阅读型" },
];

// 自定义风格：在所选主题基础上叠加用户的风格描述
const customStyle = ref("");

// 可叠加的「增强技能」——勾选后随对话一起注入，让 AI 制作时调用
const ENHANCERS: { id: string; label: string; hint: string }[] = [
  { id: "deep-research", label: "深度搜索", hint: "先联网研究、把内容补全/查证" },
  { id: "image-gen", label: "AI 配图", hint: "为页面生成插图/配图" },
  { id: "pdf", label: "读 PDF", hint: "解析上传的 PDF 素材" },
];
const extraSkills = ref<string[]>([]);
function toggleSkill(id: string) {
  const i = extraSkills.value.indexOf(id);
  if (i >= 0) extraSkills.value.splice(i, 1);
  else extraSkills.value.push(id);
}
const skillIds = computed(() => ["polaris-deck-studio", ...extraSkills.value]);

const canGenerate = computed(
  () => (contentText.value.trim().length >= 10 || uploads.value.length > 0) && phase.value !== "generating"
);

// ───────── 上传 ─────────
async function addPaths(paths: string[]) {
  if (!paths.length) return;
  uploading.value = true;
  error.value = null;
  try {
    const res = await chatApi.attachFiles(convId.value ?? undefined, paths);
    for (const r of res) {
      if (r.ok && !uploads.value.some((u) => u.path === r.path)) uploads.value.push(r);
    }
  } catch (e: any) {
    error.value = e?.message ?? String(e);
  } finally {
    uploading.value = false;
  }
}
async function pickFiles() {
  try {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const sel = await open({
      multiple: true,
      filters: [{ name: "素材", extensions: ["md", "txt", "docx", "pdf", "pptx", "html", "json", "csv"] }],
    });
    if (!sel) return;
    await addPaths(Array.isArray(sel) ? sel : [sel]);
  } catch (e: any) {
    error.value = e?.message ?? String(e);
  }
}
function removeUpload(i: number) {
  uploads.value.splice(i, 1);
}
const { isOver: dropOver } = useFileDrop({
  active: () => app.view === VIEW_KEY && phase.value !== "generating",
  onDrop: addPaths,
});

// ───────── prompt ─────────
function densityText(): string {
  return DENSITIES.find((d) => d.id === density.value)?.hint ?? "";
}
function buildPrompt(): string {
  const themeLine =
    selectedTheme.value === "auto"
      ? "由你根据内容自挑最合适的主题（17 套里选，或自行配色）"
      : `${curTheme.value.name}（data-theme id=${selectedTheme.value}）`;
  const lines = [
    "请使用 polaris-deck-studio skill 制作一份演示。",
    "",
    "## 制作配置",
    `- 输出模式：${isPpt.value ? "pptx（最终交付 .pptx，整版图嵌入，像素级还原主题；同时保留自包含 deck.html）" : "html（最终交付自包含单文件 .html）"}`,
    `- 主题：${themeLine}`,
    `- 画幅比例：${aspect.value}`,
    autoSlides.value
      ? "- 页数：由你按内容自动决定（内容多则多页、少则少页，别硬凑也别硬砍）"
      : `- 页数：约 ${slideCount.value} 页（含封面与结尾，按内容增减）`,
    `- 信息密度：${density.value} —— ${densityText()}`,
  ];
  if (customStyle.value.trim()) {
    lines.push(`- 自定义风格补充：${customStyle.value.trim()}（在所选主题基础上按此调整，与主题冲突时以此为准）`);
  }
  if (extraSkills.value.length) {
    const names = ENHANCERS.filter((e) => extraSkills.value.includes(e.id)).map((e) => e.label).join("、");
    lines.push(`- 已启用增强技能：${names}——制作时按需调用（如先研究补全内容、为页面配图、解析 PDF）。`);
  }
  if (uploads.value.length) {
    lines.push("", "## 素材文件（先 Read 它们作为内容来源）");
    for (const u of uploads.value) lines.push(`- ${u.path}`);
  }
  lines.push("", "## 正文内容");
  lines.push(contentText.value.trim() || "（见上方素材文件）");
  lines.push(
    "",
    "## 要求",
    "- 严格按 SKILL.md：把 base.css + themes.css 内联进 <style>、runtime.js 内联进 <script>，产出**自包含** deck.html，存到产物目录。",
    isPpt.value
      ? "- 然后运行 install-deps.mjs（首次）+ export-pptx.mjs，把 deck.html 导成 .pptx。依赖缺失则按 SKILL.md 兜底（打印 PDF 或 python-pptx），并说明。"
      : "- 网页模式到此即可，无需导出。",
    "- 回答末尾用**绝对路径**列出最终产物文件。",
  );
  return lines.join("\n");
}
function revisePrompt(text: string): string {
  return [
    "对刚才生成的这份演示做如下修改：",
    "",
    text.trim(),
    "",
    "## 要求",
    "- 直接在**原产物文件上修改**（保持文件名不变，别另起新文件），改完重新保存。",
    isPpt.value
      ? "- 若交付物是 .pptx，改完 deck.html 后重新运行 export-pptx.mjs 覆盖导出。"
      : "- 网页模式：改完自包含 .html 即可。",
    "- 回答末尾用绝对路径列出更新后的产物文件。",
  ].join("\n");
}

// ───────── 动作 ─────────
async function ensureConv(): Promise<string> {
  let project = app.projects.find((p) => p.name === STUDIO_PROJECT_NAME);
  let projectId: string | null = project?.id ?? null;
  if (!projectId) {
    await app.createProject(STUDIO_PROJECT_NAME);
    projectId = app.currentProjectId;
    if (!projectId) throw new Error("创建演示工坊项目失败");
  }
  const conv = await app.createConversation(projectId);
  return conv.id;
}
function preview(): string {
  const t = contentText.value.trim();
  if (t) return t.slice(0, 24) + (t.length > 24 ? "…" : "");
  if (uploads.value.length) return uploads.value[0].name;
  return "未命名";
}

async function start() {
  if (!canGenerate.value) return;
  error.value = null;
  try {
    const id = await ensureConv();
    convId.value = id;
    if (uploads.value.length) {
      try {
        const res = await chatApi.attachFiles(id, uploads.value.map((u) => u.path));
        uploads.value = res.filter((r) => r.ok);
      } catch {
        /* 已在目录则忽略 */
      }
    }
    lastAction.value = "create";
    phase.value = "generating";
    const icon = isPpt.value ? "📊" : "🖥️";
    const display = `${icon} PPT·${curTheme.value.name}：${preview()}`;
    await chat.send(id, buildPrompt(), display, undefined, {
      permissionMode: "auto_current",
      skillIds: skillIds.value,
      goal: `制作一份「${curTheme.value.name}」主题的${isPpt.value ? "PPT(.pptx)" : "网页PPT(.html)"}并保存到产物目录`,
    });
  } catch (e: any) {
    error.value = e?.message ?? String(e);
    phase.value = hasResult.value ? "done" : "config";
  }
}

const reviseText = ref("");
async function revise() {
  const text = reviseText.value.trim();
  if (!text || !convId.value) return;
  error.value = null;
  try {
    lastAction.value = "revise";
    phase.value = "generating";
    await chat.send(convId.value, revisePrompt(text), `✏️ 修改 PPT：${text.slice(0, 20)}`, undefined, {
      permissionMode: "auto_current",
      skillIds: skillIds.value,
      goal: "按要求修改已生成的演示并覆盖更新产物文件",
    });
    reviseText.value = "";
  } catch (e: any) {
    error.value = e?.message ?? String(e);
    phase.value = "done";
  }
}

function reset() {
  phase.value = "config";
  convId.value = null;
  outputs.value = [];
  previewHtml.value = "";
  reviseText.value = "";
}

// ───────── 产物 + 实时预览 ─────────
const sending = computed(() => chat.isSending(convId.value));
const outputs = ref<{ path: string; name: string }[]>([]);
const hasResult = computed(() => outputs.value.length > 0);
const previewHtml = ref<string>("");
const previewPath = ref<string>("");
const outRe = computed(() => (isPpt.value ? /\.(pptx|html?)$/i : /\.html?$/i));

async function loadOutputs() {
  if (!convId.value) return;
  try {
    const list = await artifactsApi.list(convId.value);
    const hits = list.filter((e) => outRe.value.test(e.name)).map((e) => ({ path: e.path, name: e.name }));
    const want = isPpt.value ? ".pptx" : ".html";
    hits.sort((a, b) => Number(b.name.toLowerCase().endsWith(want)) - Number(a.name.toLowerCase().endsWith(want)));
    outputs.value = hits;
    await loadPreview();
  } catch {
    /* ignore */
  }
}
// 读取自包含 .html 喂 iframe srcdoc 做实时预览
async function loadPreview() {
  const htmlOut = outputs.value.find((o) => /\.html?$/i.test(o.name));
  if (!htmlOut) return;
  if (htmlOut.path === previewPath.value && previewHtml.value) return;
  try {
    const p = await artifactsApi.read(htmlOut.path);
    if (p?.text) {
      previewHtml.value = p.text;
      previewPath.value = htmlOut.path;
    }
  } catch {
    /* ignore */
  }
}

watch(sending, async (now, before) => {
  if (before && !now && phase.value === "generating") {
    await loadOutputs();
    phase.value = "done";
  }
});
let poll: ReturnType<typeof setInterval> | null = null;
watch(phase, (p) => {
  if (poll) {
    clearInterval(poll);
    poll = null;
  }
  if (p === "generating") poll = setInterval(loadOutputs, 4000);
});
onUnmounted(() => {
  if (poll) clearInterval(poll);
});

function openConv() {
  if (convId.value) app.setView("chat");
}
function openDir() {
  const proj = app.projects.find((p) => p.name === STUDIO_PROJECT_NAME);
  if (proj) app.openProjectDir(proj.id);
}
function openFile(path: string) {
  artifactsApi.openExternal(path);
}
const pptxOut = computed(() => outputs.value.find((o) => /\.pptx$/i.test(o.name)));
function fillDemo() {
  contentText.value =
    "主题：Polaris 是什么。一句话——把 AI 变成你的创作生产线。" +
    "三个要点：① 对话即创作，文案/PPT/视频一站出；② 知识库沉淀，越用越懂你；③ 全本地，数据不出门。" +
    "结尾：现在就开始，让未来替你打工。";
}
</script>

<template>
  <div class="dk">
    <!-- 顶栏 -->
    <header class="dk-head">
      <Presentation :size="19" :stroke-width="1.7" class="dk-icon" />
      <h1 class="dk-title">PPT 演示</h1>
      <span class="dk-sub">左侧配置 · 中间实时预览 · 底部继续修改</span>
      <div class="dk-toggle">
        <button :class="{ on: !isPpt }" @click="outputMode = 'html'"><Monitor :size="13" /> 网页 PPT</button>
        <button :class="{ on: isPpt }" @click="outputMode = 'pptx'"><FileType2 :size="13" /> 传统 PPT</button>
      </div>
    </header>

    <!-- 工作台：左配置 + 右主区 -->
    <div class="dk-work">
      <!-- 左：配置 -->
      <aside class="dk-side">
        <div class="dk-side-sec">
          <div class="dk-side-title">主题风格</div>
          <div class="dk-preview-mini" :style="{ background: curTheme.bg, color: curTheme.text }">
            <span :style="{ color: curTheme.accent, fontFamily: curTheme.font === 'serif' ? 'var(--serif)' : 'inherit' }">{{ curTheme.name }}</span>
          </div>
          <template v-for="g in groups" :key="g.group">
            <div class="dk-group-label">{{ g.group }}</div>
            <div class="dk-themes">
              <button
                v-for="t in g.items"
                :key="t.id"
                class="dk-theme"
                :class="{ active: selectedTheme === t.id }"
                :title="t.name"
                @click="selectedTheme = t.id"
              >
                <span class="dk-theme-sw" :style="{ background: t.bg }">
                  <Sparkles v-if="t.id === 'auto'" :size="12" :style="{ color: t.accent }" />
                  <span v-else class="dk-theme-acc" :style="{ background: t.accent }"></span>
                </span>
                <span class="dk-theme-name">{{ t.name }}</span>
              </button>
            </div>
          </template>
        </div>

        <div class="dk-side-sec">
          <div class="dk-side-title">参数</div>
          <div class="dk-lab-row">
            <label class="dk-lab">页数 <b v-if="!autoSlides">≈ {{ slideCount }}</b><b v-else>AI 决定</b></label>
            <label class="dk-check"><input type="checkbox" v-model="autoSlides" /> AI 决定</label>
          </div>
          <input type="range" min="4" max="30" step="1" v-model.number="slideCount" class="dk-range" :disabled="autoSlides" />
          <label class="dk-lab">画幅</label>
          <div class="dk-seg">
            <button :class="{ on: aspect === '16:9' }" @click="aspect = '16:9'">16:9</button>
            <button :class="{ on: aspect === '4:3' }" @click="aspect = '4:3'">4:3</button>
          </div>
          <label class="dk-lab">信息密度</label>
          <div class="dk-seg">
            <button v-for="d in DENSITIES" :key="d.id" :class="{ on: density === d.id }" @click="density = d.id">{{ d.label }}</button>
          </div>
          <span class="dk-note">{{ densityText() }}</span>
        </div>

        <div class="dk-side-sec">
          <div class="dk-side-title">自定义风格 · 可选</div>
          <textarea
            v-model="customStyle"
            class="dk-custom"
            rows="2"
            placeholder="用自己的话补充风格：如「黑金高级、圆角大卡片、衬线大标题」「极简留白、莫兰迪色」…会叠加在所选主题上"
          />
        </div>

        <div class="dk-side-sec">
          <div class="dk-side-title">增强技能 · 可选</div>
          <div class="dk-skills">
            <button
              v-for="e in ENHANCERS"
              :key="e.id"
              class="dk-skill"
              :class="{ on: extraSkills.includes(e.id) }"
              :title="e.hint"
              @click="toggleSkill(e.id)"
            >{{ e.label }}</button>
          </div>
          <span class="dk-note">
            勾选后 AI 制作时会调用这些技能（如先联网补全内容、为页面配图）。选「AI 自由发挥」风格时尤其好用。
          </span>
        </div>

        <div v-if="hasResult" class="dk-side-sec">
          <div class="dk-side-title">产物</div>
          <button v-for="o in outputs" :key="o.path" class="dk-out" @click="openFile(o.path)">
            <component :is="/\.pptx$/i.test(o.name) ? FileType2 : Monitor" :size="13" />
            <span>{{ o.name }}</span><ExternalLink :size="11" />
          </button>
          <div class="dk-side-acts">
            <button class="dk-ghost" @click="openDir"><FolderOpen :size="12" /> 目录</button>
            <button class="dk-ghost" @click="openConv"><Eye :size="12" /> 对话</button>
            <button class="dk-ghost" @click="reset"><RefreshCw :size="12" /> 重来</button>
          </div>
        </div>
      </aside>

      <!-- 右：主区（输入 / 预览）+ 底部 composer -->
      <main class="dk-main">
        <div class="dk-canvas" :class="{ drop: dropOver }">
          <!-- 无产物：内容输入 -->
          <div v-if="!hasResult" class="dk-input">
            <h3 class="dk-input-title"><FileText :size="16" :stroke-width="1.7" /> 演示内容</h3>
            <textarea
              v-model="contentText"
              class="dk-textarea"
              placeholder="把要做成演示的文案/大纲贴在这里，或上传文件作为素材，然后点下方「生成」…"
            />
            <div class="dk-input-foot">
              <span :class="{ warn: charCount < 10 && uploads.length === 0 }">
                {{ charCount }} 字{{ charCount < 10 && uploads.length === 0 ? " · 至少 10 字或上传文件" : "" }}
              </span>
              <div class="dk-input-btns">
                <button class="dk-ghost" @click="fillDemo">填入示例</button>
                <button class="dk-ghost" :disabled="uploading" @click="pickFiles">
                  <Loader v-if="uploading" :size="12" class="spin" /><Upload v-else :size="12" /> 上传
                </button>
              </div>
            </div>
            <div v-if="uploads.length" class="dk-files">
              <div v-for="(u, i) in uploads" :key="u.path" class="dk-file">
                <FileText :size="12" /><span class="dk-file-name">{{ u.name }}</span>
                <button class="dk-file-x" @click="removeUpload(i)"><X :size="12" /></button>
              </div>
            </div>
          </div>

          <!-- 有产物：实时预览 -->
          <div v-else class="dk-preview">
            <iframe v-if="previewHtml" class="dk-frame" :srcdoc="previewHtml" sandbox="allow-scripts allow-same-origin"></iframe>
            <div v-else class="dk-frame-empty"><Monitor :size="30" /><span>预览加载中…可在对话或目录查看</span></div>
            <div v-if="isPpt && pptxOut" class="dk-preview-tip">
              预览是网页 deck；最终 <b>.pptx</b> 已生成，点左侧产物或「打开」查看。
            </div>
          </div>

          <!-- 生成中遮罩 -->
          <div v-if="phase === 'generating'" class="dk-overlay">
            <Loader :size="30" class="spin" />
            <span>{{ lastAction === 'revise' ? '正在按修改重做…' : '正在制作 PPT…' }}</span>
            <button class="dk-ghost" @click="openConv">在对话里看进度 →</button>
          </div>
        </div>

        <!-- 底部 composer：未生成=生成；已生成=继续修改 -->
        <div class="dk-composer">
          <div v-if="error" class="dk-error">{{ error }}</div>
          <template v-if="!hasResult">
            <button class="dk-primary" :disabled="!canGenerate || phase === 'generating'" @click="start">
              <Zap :size="16" :stroke-width="1.9" /> 一键生成{{ isPpt ? "传统 PPT" : "网页 PPT" }}
            </button>
            <span class="dk-note">在「演示工坊」项目下新建对话注入技能全自动制作。</span>
          </template>
          <template v-else>
            <Wand2 :size="16" :stroke-width="1.7" class="dk-comp-i" />
            <textarea
              v-model="reviseText"
              class="dk-comp-input"
              rows="1"
              placeholder="继续修改：第 2 页换三栏卡片 / 换东京夜主题 / 标题改成『…』 / 再加一页总结…"
              @keydown.enter.exact.prevent="revise"
            />
            <button class="dk-primary sm" :disabled="!reviseText.trim() || phase === 'generating'" @click="revise">
              <Wand2 :size="14" /> 应用修改
            </button>
          </template>
        </div>
      </main>
    </div>
  </div>
</template>

<style scoped>
.dk { height: 100%; display: flex; flex-direction: column; overflow: hidden; background: var(--bg); }
.dk-head { display: flex; align-items: center; gap: 10px; padding: 12px 20px; border-bottom: 1px solid var(--border-soft); background: var(--panel); }
.dk-icon { color: var(--primary); }
.dk-title { font-family: var(--serif); font-size: 16px; font-weight: 600; color: var(--text); }
.dk-sub { font-size: 12px; color: var(--muted); margin-left: 4px; }
.dk-toggle { margin-left: auto; display: inline-flex; gap: 3px; padding: 3px; background: var(--bg-soft); border-radius: 9px; border: 1px solid var(--border-soft); }
.dk-toggle button { display: inline-flex; align-items: center; gap: 5px; padding: 6px 12px; border: none; background: transparent; color: var(--muted); font-size: 12.5px; font-weight: 600; border-radius: 7px; cursor: pointer; }
.dk-toggle button.on { background: var(--primary); color: #fff; }

.dk-work { flex: 1; display: grid; grid-template-columns: 252px 1fr; overflow: hidden; }
@media (max-width: 820px) { .dk-work { grid-template-columns: 200px 1fr; } }

/* 左侧配置 */
.dk-side { overflow-y: auto; border-right: 1px solid var(--border-soft); padding: 14px; display: flex; flex-direction: column; gap: 18px; background: var(--bg-soft); }
.dk-side-sec { display: flex; flex-direction: column; gap: 8px; }
.dk-side-title { font-size: 11px; font-weight: 700; letter-spacing: .1em; text-transform: uppercase; color: var(--dim); }
.dk-preview-mini { height: 48px; border-radius: 8px; border: 1px solid var(--border); display: flex; align-items: center; padding: 0 12px; font-size: 13px; font-weight: 800; }
.dk-group-label { font-size: 10.5px; color: var(--dim); margin-top: 2px; }
.dk-themes { display: grid; grid-template-columns: 1fr 1fr; gap: 6px; }
.dk-theme { display: flex; align-items: center; gap: 6px; padding: 5px 6px; border: 1px solid var(--border); border-radius: 7px; background: var(--bg); cursor: pointer; text-align: left; }
.dk-theme:hover { border-color: var(--primary); }
.dk-theme.active { border-color: var(--primary); background: var(--primary-soft); }
.dk-theme-sw { width: 20px; height: 20px; border-radius: 5px; flex-shrink: 0; border: 1px solid rgba(0,0,0,.08); position: relative; overflow: hidden; display: flex; align-items: center; justify-content: center; }
.dk-theme-acc { position: absolute; bottom: 0; left: 0; right: 0; height: 38%; }
.dk-theme-name { font-size: 11px; color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

.dk-lab { font-size: 12px; font-weight: 500; color: var(--muted); margin-top: 4px; }
.dk-lab b { color: var(--primary-deep); }
.dk-range { width: 100%; accent-color: var(--primary); }
.dk-seg { display: flex; gap: 4px; }
.dk-seg button { flex: 1; padding: 6px 4px; border: 1px solid var(--border); border-radius: 6px; background: var(--bg); color: var(--text-2); font-size: 11.5px; cursor: pointer; }
.dk-seg button.on { border-color: var(--primary); background: var(--primary-soft); color: var(--primary-deep); font-weight: 600; }
.dk-note { font-size: 10.5px; color: var(--muted); line-height: 1.5; }
.dk-lab-row { display: flex; align-items: center; justify-content: space-between; gap: 6px; }
.dk-check { display: inline-flex; align-items: center; gap: 4px; font-size: 11px; color: var(--muted); cursor: pointer; user-select: none; }
.dk-check input { accent-color: var(--primary); }
.dk-custom { resize: none; padding: 8px 10px; border: 1px solid var(--border); border-radius: 7px; background: var(--bg); color: var(--text); font-size: 11.5px; line-height: 1.5; }
.dk-custom:focus { outline: none; border-color: var(--primary); }
.dk-skills { display: flex; flex-wrap: wrap; gap: 5px; }
.dk-skill { padding: 5px 10px; border: 1px solid var(--border); border-radius: 999px; background: var(--bg); color: var(--text-2); font-size: 11px; cursor: pointer; }
.dk-skill.on { border-color: var(--primary); background: var(--primary-soft); color: var(--primary-deep); font-weight: 600; }

.dk-out { display: flex; align-items: center; gap: 6px; padding: 7px 9px; border: 1px solid var(--primary); border-radius: 7px; background: var(--primary-soft); color: var(--primary-deep); font-size: 11.5px; font-weight: 600; cursor: pointer; }
.dk-out span { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.dk-side-acts { display: flex; gap: 5px; margin-top: 4px; }
.dk-ghost { display: inline-flex; align-items: center; gap: 4px; padding: 6px 9px; border: 1px solid var(--border); border-radius: 6px; background: transparent; color: var(--text-2); font-size: 11.5px; cursor: pointer; transition: border-color .15s, color .15s; }
.dk-ghost:hover:not(:disabled) { border-color: var(--primary); color: var(--primary); }
.dk-ghost:disabled { opacity: .5; cursor: default; }

/* 右主区 */
.dk-main { display: flex; flex-direction: column; overflow: hidden; position: relative; }
.dk-canvas { flex: 1; overflow: auto; position: relative; padding: 18px; display: flex; }
.dk-canvas.drop { outline: 2px dashed var(--primary); outline-offset: -10px; }

/* 输入态 */
.dk-input { flex: 1; display: flex; flex-direction: column; gap: 10px; max-width: 860px; margin: 0 auto; width: 100%; }
.dk-input-title { display: inline-flex; align-items: center; gap: 7px; font-size: 14px; font-weight: 600; color: var(--text); margin: 0; }
.dk-textarea { flex: 1; min-height: 300px; resize: none; padding: 14px 16px; border: 1px solid var(--border); border-radius: 10px; background: var(--panel); color: var(--text); font-size: 14px; line-height: 1.75; }
.dk-textarea:focus { outline: none; border-color: var(--primary); }
.dk-input-foot { display: flex; align-items: center; justify-content: space-between; font-size: 12px; color: var(--muted); }
.dk-input-foot .warn { color: var(--vermilion); }
.dk-input-btns { display: flex; gap: 6px; }
.dk-files { display: flex; flex-wrap: wrap; gap: 6px; }
.dk-file { display: flex; align-items: center; gap: 5px; padding: 4px 8px; background: var(--bg-soft); border-radius: 6px; font-size: 11.5px; color: var(--text-2); }
.dk-file-name { max-width: 180px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.dk-file-x { border: none; background: transparent; color: var(--muted); cursor: pointer; display: inline-flex; padding: 1px; }
.dk-file-x:hover { color: var(--vermilion); }

/* 预览态 */
.dk-preview { flex: 1; display: flex; flex-direction: column; gap: 8px; min-height: 0; }
.dk-frame { flex: 1; width: 100%; border: 1px solid var(--border); border-radius: 10px; background: #fff; box-shadow: var(--shadow, 0 6px 24px rgba(0,0,0,.08)); }
.dk-frame-empty { flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 10px; color: var(--muted); border: 1px dashed var(--border); border-radius: 10px; }
.dk-preview-tip { font-size: 12px; color: var(--muted); text-align: center; }

/* 生成遮罩 */
.dk-overlay { position: absolute; inset: 18px; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px; background: color-mix(in srgb, var(--bg) 78%, transparent); backdrop-filter: blur(2px); border-radius: 10px; color: var(--text); font-size: 14px; font-weight: 600; }

/* 底部 composer */
.dk-composer { border-top: 1px solid var(--border-soft); background: var(--panel); padding: 12px 18px; display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
.dk-comp-i { color: var(--primary); flex-shrink: 0; }
.dk-comp-input { flex: 1; min-width: 200px; resize: none; padding: 10px 12px; border: 1px solid var(--border); border-radius: 9px; background: var(--bg); color: var(--text); font-size: 13px; line-height: 1.5; max-height: 110px; }
.dk-comp-input:focus { outline: none; border-color: var(--primary); }
.dk-primary { display: inline-flex; align-items: center; justify-content: center; gap: 8px; padding: 11px 26px; border: none; border-radius: 10px; background: var(--primary); color: #fff; font-size: 14px; font-weight: 600; cursor: pointer; transition: filter .15s; }
.dk-primary.sm { padding: 10px 18px; font-size: 13px; flex-shrink: 0; }
.dk-primary:hover:not(:disabled) { filter: brightness(1.07); }
.dk-primary:disabled { opacity: .5; cursor: default; }
.dk-error { flex-basis: 100%; padding: 8px 11px; border-radius: 8px; background: var(--vermilion-soft); color: var(--vermilion); font-size: 12px; }

.spin { animation: dk-spin .9s linear infinite; }
@keyframes dk-spin { to { transform: rotate(360deg); } }
</style>
