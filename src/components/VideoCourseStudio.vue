<script setup lang="ts">
import { ref, computed, watch, onUnmounted } from "vue";
import {
  Clapperboard,
  FileText,
  Palette,
  Loader,
  CheckCircle2,
  Circle,
  Sparkles,
  Mic,
  Video as VideoIcon,
  Layers,
  Clock,
  Eye,
  Upload,
  X,
  Music,
  Gauge,
  Zap,
  RefreshCw,
  FolderOpen,
  ExternalLink,
  ChevronRight,
  Languages,
  Captions,
  AlertTriangle,
} from "@lucide/vue";
import { useAppStore } from "../stores/app";
import { useChatStore } from "../stores/chat";
import { artifacts as artifactsApi, chat as chatApi, type AttachedFile } from "../tauri";
import { useFileDrop } from "../composables/useFileDrop";
import { DECK_THEMES_WITH_AUTO, findTheme, type DeckTheme } from "../lib/deckThemes";

const app = useAppStore();
const chat = useChatStore();

const STUDIO_PROJECT_NAME = "课件视频";

// ───────── 流程阶段 ─────────
// config   填要求
// planning AI 正在生成三份规划文件
// review   三份文件就绪，等用户确认
// executing AI 正在执行全流程出片
// done     完成
type Phase = "config" | "planning" | "review" | "executing" | "done";
const phase = ref<Phase>("config");
const autoMode = ref(false); // 全自动：规划完不停，直接出片
const error = ref<string | null>(null);
const convId = ref<string | null>(null);

// ───────── 配置项 ─────────
const scriptText = ref("");
const charCount = computed(() => scriptText.value.length);

// 上传文件（作为素材给 AI Read）
const uploads = ref<AttachedFile[]>([]);
const uploading = ref(false);

// 时长：可填写的秒数 + 快捷预设
const durationSec = ref(180);
const durationPresets = [
  { label: "短", sec: 60 },
  { label: "中", sec: 180 },
  { label: "长", sec: 480 },
];
const durationText = computed(() => {
  const s = Math.max(15, durationSec.value || 0);
  const m = Math.floor(s / 60);
  const r = s % 60;
  return m > 0 ? `${m} 分 ${r ? r + " 秒" : ""}`.trim() : `${r} 秒`;
});

// PPT 风格：复用共享主题目录（与「演示工坊」一致，预览更精致；id 仅作 AI 设计提示）
const selectedTheme = ref("tokyo-night");
const themes = DECK_THEMES_WITH_AUTO;
const curTheme = computed<DeckTheme>(() => findTheme(selectedTheme.value));
const themeName = computed(() => curTheme.value.name);

// 配音：语速 + 音色
const speed = ref(1.0);
const VOICES = [
  { id: "male-qn-qingse", name: "青涩青年（男）" },
  { id: "male-qn-jingying", name: "精英青年（男）" },
  { id: "male-qn-badao", name: "霸道青年（男）" },
  { id: "presenter_male", name: "主持人（男）" },
  { id: "audiobook_male_1", name: "有声书（男）" },
  { id: "female-shaonv", name: "少女音（女）" },
  { id: "female-yujie", name: "御姐音（女）" },
  { id: "female-chengshu", name: "成熟女性（女）" },
  { id: "female-tianmei", name: "甜美女性（女）" },
  { id: "presenter_female", name: "主持人（女）" },
  { id: "audiobook_female_1", name: "有声书（女）" },
];
const voice = ref("male-qn-jingying");

// ───────── 配音语言（MiniMax language_boost）─────────
// boost 值对齐 MiniMax T2A v2 的 language_boost 取值
type Lang = { code: string; name: string; boost: string };
const DUB_LANGS: Lang[] = [
  { code: "zh-Hans", name: "中文 · 普通话", boost: "Chinese" },
  { code: "yue", name: "粤语", boost: "Chinese,Yue" },
  { code: "en", name: "English 英语", boost: "English" },
  { code: "ja", name: "日本語 日语", boost: "Japanese" },
  { code: "ko", name: "한국어 韩语", boost: "Korean" },
  { code: "es", name: "Español 西班牙语", boost: "Spanish" },
  { code: "fr", name: "Français 法语", boost: "French" },
  { code: "de", name: "Deutsch 德语", boost: "German" },
  { code: "ru", name: "Русский 俄语", boost: "Russian" },
  { code: "pt", name: "Português 葡萄牙语", boost: "Portuguese" },
  { code: "it", name: "Italiano 意大利语", boost: "Italian" },
  { code: "ar", name: "العربية 阿拉伯语", boost: "Arabic" },
  { code: "hi", name: "हिन्दी 印地语", boost: "Hindi" },
  { code: "th", name: "ไทย 泰语", boost: "Thai" },
  { code: "vi", name: "Tiếng Việt 越南语", boost: "Vietnamese" },
  { code: "id", name: "Bahasa Indonesia 印尼语", boost: "Indonesian" },
];
const dubLang = ref("zh-Hans");
const dubInfo = computed(() => DUB_LANGS.find((l) => l.code === dubLang.value) ?? DUB_LANGS[0]);

// ───────── 字幕语言（含繁体中文；文本由 AI 翻译，不受 TTS 限制）─────────
const SUB_LANGS: { code: string; name: string }[] = [
  { code: "zh-Hans", name: "简体中文" },
  { code: "zh-Hant", name: "繁體中文" },
  { code: "en", name: "English" },
  { code: "yue", name: "粤语字幕" },
  { code: "ja", name: "日本語" },
  { code: "ko", name: "한국어" },
  { code: "es", name: "Español" },
  { code: "fr", name: "Français" },
  { code: "de", name: "Deutsch" },
  { code: "ru", name: "Русский" },
  { code: "pt", name: "Português" },
  { code: "it", name: "Italiano" },
  { code: "ar", name: "العربية" },
  { code: "hi", name: "हिन्दी" },
  { code: "th", name: "ไทย" },
  { code: "vi", name: "Tiếng Việt" },
  { code: "id", name: "Indonesia" },
];
const subLangName = (code: string) => SUB_LANGS.find((l) => l.code === code)?.name ?? code;
// 选中的字幕语言，保持点选顺序：前 1–2 种会烧进画面
const subLangs = ref<string[]>(["zh-Hans"]);
const burnSubs = ref(true); // 烧录（硬字幕）：前 1–2 种压进画面，任何播放器都能看到
function toggleSub(code: string) {
  const i = subLangs.value.indexOf(code);
  if (i >= 0) subLangs.value.splice(i, 1);
  else subLangs.value.push(code);
}
const burnList = computed(() => (burnSubs.value ? subLangs.value.slice(0, 2) : []));

// 背景音乐
const bgmPath = ref<string>("");
const bgmName = computed(() => bgmPath.value.split(/[\\/]/).pop() || "");
const bgmVolume = ref(0.18); // 0–1，相对人声

// ───────── 规划产物（三份文件） ─────────
interface PlanFile {
  key: "script" | "style" | "narration";
  label: string;
  match: RegExp;
  path: string | null;
  text: string;
}
const planFiles = ref<PlanFile[]>([
  { key: "script", label: "逐字稿", match: /逐字稿|script/i, path: null, text: "" },
  { key: "style", label: "PPT 风格 / 格式 / 动效", match: /风格|动效|style|theme/i, path: null, text: "" },
  { key: "narration", label: "口播稿", match: /口播|narration|voiceover/i, path: null, text: "" },
]);
const activePlanTab = ref<PlanFile["key"]>("script");
const activePlanFile = computed(() => planFiles.value.find((f) => f.key === activePlanTab.value));
const planReady = computed(() => planFiles.value.every((f) => f.path));

// ───────── 校验 ─────────
const canPlan = computed(
  () => (scriptText.value.trim().length >= 20 || uploads.value.length > 0) && phase.value === "config"
);

// ───────── 上传文件（按钮选择 + 拖拽，共用 addPaths）─────────
async function addPaths(paths: string[]) {
  if (!paths.length) return;
  uploading.value = true;
  error.value = null;
  try {
    const res = await chatApi.attachFiles(convId.value ?? undefined, paths);
    for (const r of res) {
      // 去重：同一文件拖/选多次只留一份
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
      filters: [
        { name: "素材", extensions: ["md", "txt", "docx", "pdf", "pptx", "html", "json", "csv"] },
      ],
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

// 原生拖拽落区（基于 Tauri drag-drop，给绝对路径）——仅在本视图的「配置页」生效
const { isOver: dropOver } = useFileDrop({
  active: () => app.view === "video_course" && phase.value === "config",
  onDrop: addPaths,
});

async function pickBgm() {
  try {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const sel = await open({
      multiple: false,
      filters: [{ name: "音频", extensions: ["mp3", "wav", "m4a", "aac", "flac", "ogg"] }],
    });
    if (typeof sel === "string") bgmPath.value = sel;
  } catch {
    /* 取消 */
  }
}

// ───────── prompt 构建 ─────────
function configBlock(): string {
  const lines = [
    "## 制作配置",
    `- 目标时长：约 ${durationSec.value} 秒（${durationText.value}）—— 口播节奏、章节数、每章信息量都要据此调配`,
    `- PPT 风格：${selectedTheme.value === "auto" ? "由你根据内容自由设计最合适的视觉风格" : `${themeName.value}（主题 id=${selectedTheme.value}）`}`,
    `- 配音音色：${VOICES.find((v) => v.id === voice.value)?.name}（voice_id=${voice.value}）`,
    `- 语速：${speed.value.toFixed(2)}（MiniMax voice_setting.speed，1.0=正常）`,
  ];
  if (bgmPath.value) {
    lines.push(
      `- 背景音乐：${bgmPath.value}（相对人声音量约 ${Math.round(bgmVolume.value * 100)}%，用 ffmpeg 混入，循环铺底并对人声做 ducking）`
    );
  } else {
    lines.push("- 背景音乐：无");
  }
  if (uploads.value.length) {
    lines.push("", "## 上传的素材文件（请先 Read 这些文件作为内容来源）");
    for (const u of uploads.value) lines.push(`- ${u.path}`);
  }
  return lines.join("\n");
}

// 多语言配音 + 多语言字幕的统一指令块（拼到每个 prompt）
function i18nBlock(): string {
  const lines = ["## 多语言配音与字幕"];

  // —— 配音语言 ——
  if (dubLang.value === "zh-Hans") {
    lines.push("- 配音语言：中文 · 普通话。MiniMax 合成时 language_boost=Chinese。");
  } else {
    lines.push(
      `- 配音语言：${dubInfo.value.name}（code=${dubLang.value}）。`,
      `  · **逐字稿、口播稿、以及 narrations.ts 里的台词，全部用「${dubInfo.value.name}」书写**——把中文内容翻译成该语言，要地道、口语化、适合朗读，而不是逐字硬翻。`,
      `  · 配音合成时务必启用 MiniMax language_boost=${dubInfo.value.boost}：给 audio-segments.json 每一段加 "language_boost": "${dubInfo.value.boost}"，或运行配音脚本前设环境变量 MINIMAX_LANGUAGE_BOOST="${dubInfo.value.boost}"。`,
    );
  }

  // —— 字幕语言 ——
  if (!subLangs.value.length) {
    lines.push("- 字幕：不需要。");
  } else {
    const codes = subLangs.value.join(",");
    const burn = burnList.value.join(",");
    lines.push(
      `- 字幕语言：${subLangs.value.map(subLangName).join("、")}。`,
      "  · 配音并提取出 audio-segments.json 后，给**每一段**补一个 subtitles 字段，形如：",
      `    "subtitles": { ${subLangs.value
        .map((c) => `"${c}": "<该段台词的${subLangName(c)}文本>"`)
        .join(", ")} }`,
      `    与配音同语言那一档可直接用该段 text；其余语言据 text 翻译（简/繁中文要分别给）。`,
      burnList.value.length
        ? `  · 出片时给 03-record.mjs 传：--subtitles=${codes} --burn=${burn}` +
          `（前 ${burnList.value.length} 种「${burnList.value.map(subLangName).join(" + ")}」烧进画面，` +
          "其余作为可切换软字幕轨，并在 MP4 旁生成同名 .srt 文件）。"
        : `  · 出片时给 03-record.mjs 传：--subtitles=${codes} --no-burn` +
          "（全部作为可切换软字幕轨 + 同名 .srt，不烧进画面）。",
    );
  }
  return lines.join("\n");
}

function planPrompt(): string {
  return [
    "请使用 polaris-video-studio skill 制作课件类网页演示视频。",
    "现在是 **第一步：规划**。只做规划，先不要开发 PPT、不要配音、不要录屏。",
    "",
    "## 输入文案",
    scriptText.value.trim() || "（见下方上传素材）",
    "",
    configBlock(),
    "",
    i18nBlock(),
    "",
    "## 本步要产出的三份文件（保存到产物目录，文件名严格如下）",
    "1. `逐字稿.md` —— 把素材整理成完整、连贯、口语化的逐字稿（按目标时长控制篇幅）。",
    "2. `PPT风格与动效.md` —— 一份给「PPT 开发」用的提示词文件：明确视觉风格/配色/版式、每页布局规则、" +
      "进出场与强调动效、字体与信息密度（结合上面选定的风格）。",
    "3. `口播稿.md` —— 把逐字稿切成按页/按段的口播片段，逐段标注 voice_id 与 speed（用上面的音色与语速）。",
    "",
    "## 要求",
    "- 三份文件都用绝对路径保存到产物目录，并在回答末尾逐一列出它们的绝对路径。",
    "- 产出三份文件后**立即停下**，等待我确认，不要继续后面的开发与合成。",
  ].join("\n");
}

function executePrompt(): string {
  return [
    "已确认三份规划文件（逐字稿.md / PPT风格与动效.md / 口播稿.md）。",
    "现在是 **第二步：执行**。请严格按这三份文件，用 polaris-video-studio skill 一路跑完，中途不要停下来问我：",
    "",
    configBlock(),
    "",
    i18nBlock(),
    "",
    "## 执行步骤",
    "1. 读取产物目录里的三份规划文件。",
    `2. 用 Node 版脚手架创建 presentation 项目（风格：${selectedTheme.value === "auto" ? "按 PPT风格与动效.md 设计" : selectedTheme.value}），逐章开发 16:9 网页演示，动效照 PPT风格与动效.md。`,
    "3. 配音：按 口播稿.md 逐段合成。**务必让 MiniMax voice_setting.speed=" +
      speed.value.toFixed(2) +
      "、voice_id=" +
      voice.value +
      "**（必要时改 audio-segments.json / minimax-tts.mjs 的 voice_setting）。",
    bgmPath.value
      ? `4. 背景音乐：用 ffmpeg 把 ${bgmPath.value} 混入最终视频，循环铺底，相对人声音量约 ${Math.round(
          bgmVolume.value * 100
        )}%，对人声段做 ducking。`
      : "4. 不加背景音乐。",
    "5. Playwright 无头逐帧截图 + ffmpeg 按音频时长对齐拼接，合成最终 MP4，保存到产物目录。",
    "6. 完成后用绝对路径列出最终 MP4。",
  ].join("\n");
}

function autoPrompt(): string {
  return [
    "请使用 polaris-video-studio skill 制作课件类网页演示视频，**全自动模式**：",
    "从规划到出片一路跑完，全程自动决策，除硬错误外绝不中途停下来等我确认。",
    "",
    "## 输入文案",
    scriptText.value.trim() || "（见下方上传素材）",
    "",
    configBlock(),
    "",
    i18nBlock(),
    "",
    "## 全流程",
    "1. 把素材整理成逐字稿（按目标时长控制篇幅），存 `逐字稿.md`。",
    "2. 拟定 PPT 风格/版式/动效提示词，存 `PPT风格与动效.md`。",
    "3. 切分口播稿并标注每段 voice_id/speed，存 `口播稿.md`。",
    `4. Node 脚手架建 presentation（风格：${selectedTheme.value === "auto" ? "自行设计" : selectedTheme.value}），逐章开发网页演示。`,
    `5. 配音：MiniMax voice_setting.speed=${speed.value.toFixed(2)}、voice_id=${voice.value}，逐段合成。`,
    bgmPath.value
      ? `6. ffmpeg 混入背景音乐 ${bgmPath.value}（相对人声约 ${Math.round(bgmVolume.value * 100)}%，循环+ducking）。`
      : "6. 不加背景音乐。",
    "7. Playwright 无头截图 + ffmpeg 合成最终 MP4，保存到产物目录并列出绝对路径。",
    "",
    "三份规划文件与最终 MP4 都用绝对路径保存到产物目录。",
  ].join("\n");
}

// ───────── 动作 ─────────
async function ensureConv(): Promise<string> {
  let project = app.projects.find((p) => p.name === STUDIO_PROJECT_NAME);
  let projectId: string | null = project?.id ?? null;
  if (!projectId) {
    await app.createProject(STUDIO_PROJECT_NAME);
    projectId = app.currentProjectId;
    if (!projectId) throw new Error("创建课件视频项目失败");
  }
  const conv = await app.createConversation(projectId);
  // createConversation 内部会 setView("chat")；视图最终由 startPlan 决定（跳到对话框）
  return conv.id;
}

async function startPlan() {
  if (!canPlan.value) return;
  error.value = null;
  try {
    const id = await ensureConv();
    convId.value = id;
    // 把已挑选但还没归属会话的上传文件，重新归属到该会话目录
    if (uploads.value.length) {
      try {
        const res = await chatApi.attachFiles(id, uploads.value.map((u) => u.path));
        uploads.value = res.filter((r) => r.ok);
      } catch {
        /* 已在 uploads 目录则忽略 */
      }
    }

    // 开始生成后，自动跳到正在生成的对话框看实时进度
    app.setView("chat");

    if (autoMode.value) {
      phase.value = "executing";
      const display = `🎬 课件视频（全自动）·${durationText.value}：${preview()}`;
      await chat.send(id, autoPrompt(), display, undefined, {
        permissionMode: "auto_current",
        skillIds: ["polaris-video-studio"],
        goal: "把这段课件文案做成最终 MP4 视频并保存到产物目录",
      });
    } else {
      phase.value = "planning";
      const display = `🎬 课件视频·规划：${preview()}`;
      await chat.send(id, planPrompt(), display, undefined, {
        permissionMode: "auto_current",
        skillIds: ["polaris-video-studio"],
      });
    }
  } catch (e: any) {
    error.value = e?.message ?? String(e);
    phase.value = "config";
    app.setView("video_course"); // 出错时切回工坊显示错误
  }
}

function preview(): string {
  const t = scriptText.value.trim();
  if (t) return t.slice(0, 28) + (t.length > 28 ? "…" : "");
  if (uploads.value.length) return uploads.value[0].name;
  return "未命名";
}

async function confirmExecute() {
  if (!convId.value) return;
  error.value = null;
  phase.value = "executing";
  try {
    await chat.send(convId.value, executePrompt(), "✅ 已确认规划，开始执行出片", undefined, {
      permissionMode: "auto_current",
      skillIds: ["polaris-video-studio"],
      goal: "按已确认的三份规划文件，制作出最终 MP4 视频并保存到产物目录",
    });
  } catch (e: any) {
    error.value = e?.message ?? String(e);
    phase.value = "review";
  }
}

function replan() {
  // 回到配置，保留输入；清空旧规划文件
  for (const f of planFiles.value) {
    f.path = null;
    f.text = "";
  }
  phase.value = "config";
}

function reset() {
  phase.value = "config";
  convId.value = null;
  for (const f of planFiles.value) {
    f.path = null;
    f.text = "";
  }
}

// ───────── 完成检测 + 拉取产物 ─────────
const sending = computed(() => chat.isSending(convId.value));
// 规划阶段已结束但三份文件没凑齐 —— 不再强行跳进 review（看得见点不了），
// 而是停在规划页给明确「未凑齐」提示与重试入口。
const planFilesReadyCount = computed(() => planFiles.value.filter((f) => f.path).length);
const planStalled = computed(
  () => phase.value === "planning" && !sending.value && !planReady.value
);

async function loadPlanFiles() {
  if (!convId.value) return;
  try {
    const list = await artifactsApi.list(convId.value);
    for (const f of planFiles.value) {
      const hit = list.find((e) => f.match.test(e.name) && /\.(md|txt)$/i.test(e.name));
      if (hit && hit.path !== f.path) {
        f.path = hit.path;
        try {
          const payload = await artifactsApi.read(hit.path);
          f.text = payload.text ?? "";
        } catch {
          f.text = "";
        }
      }
    }
  } catch {
    /* ignore */
  }
}

// 最终产物（MP4）
const outputs = ref<{ path: string; name: string }[]>([]);
async function loadOutputs() {
  if (!convId.value) return;
  try {
    const list = await artifactsApi.list(convId.value);
    outputs.value = list
      .filter((e) => /\.(mp4|mov|webm)$/i.test(e.name))
      .map((e) => ({ path: e.path, name: e.name }));
  } catch {
    /* ignore */
  }
}

// 监听发送状态：planning/executing 结束时拉产物
watch(sending, async (now, before) => {
  if (before && !now) {
    if (phase.value === "planning") {
      await loadPlanFiles();
      if (planReady.value) {
        activePlanTab.value = "script";
        phase.value = "review";
      }
      // 没凑齐三份：留在 planning，由 planStalled 渲染「未凑齐」面板，不强行进 review。
    } else if (phase.value === "executing") {
      await loadPlanFiles();
      await loadOutputs();
      phase.value = "done";
    }
  }
});

// 规划中也轮询，让文件一就绪就显示
let poll: ReturnType<typeof setInterval> | null = null;
watch(phase, (p) => {
  if (poll) {
    clearInterval(poll);
    poll = null;
  }
  if (p === "planning" || p === "executing") {
    poll = setInterval(() => {
      if (phase.value === "executing") loadOutputs();
      loadPlanFiles();
    }, 4000);
  }
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
function fillDemo() {
  scriptText.value =
    "AI 正在重画职业地图。到 2030 年，全球将有 9200 万个岗位消失、1.7 亿个新岗位诞生。" +
    "但很少有专业会被整体消灭——每个专业的任务构成都会被改写。这对志愿填报意味着什么？" +
    "时代趋势 → 专业四象限 → X 技能配方 → 娃的画像 → 该不该报、凭什么。北极星 Polaris 替你把未来算清楚。";
}
</script>

<template>
  <div class="vc">
    <header class="vc-head">
      <Clapperboard :size="20" :stroke-width="1.7" class="vc-icon" />
      <h1 class="vc-title">生成课件类视频</h1>
      <span class="vc-sub">先规划三份文件 → 确认 → 自动出片</span>

      <label class="vc-auto" :class="{ on: autoMode }">
        <Zap :size="14" :stroke-width="1.9" />
        <span>全自动模式</span>
        <input type="checkbox" v-model="autoMode" />
        <span class="vc-switch"><span class="vc-knob"></span></span>
      </label>
    </header>

    <!-- 流程进度条 -->
    <nav class="vc-flow">
      <div class="vc-flow-step" :class="{ active: phase === 'config', done: phase !== 'config' }">
        <FileText :size="15" /> <span>1 · 填要求</span>
      </div>
      <ChevronRight :size="14" class="vc-flow-sep" />
      <div
        class="vc-flow-step"
        :class="{
          active: phase === 'planning' || phase === 'review',
          done: phase === 'executing' || phase === 'done',
          skip: autoMode,
        }"
      >
        <Sparkles :size="15" /> <span>2 · 规划三文件{{ autoMode ? "（已跳过）" : "" }}</span>
      </div>
      <ChevronRight :size="14" class="vc-flow-sep" />
      <div class="vc-flow-step" :class="{ active: phase === 'executing', done: phase === 'done' }">
        <VideoIcon :size="15" /> <span>3 · 出片</span>
      </div>
    </nav>

    <div class="vc-body">
      <!-- ════════ 配置页 ════════ -->
      <section v-if="phase === 'config'" class="vc-grid">
        <!-- 左：内容输入 -->
        <div class="vc-card">
          <h3 class="vc-card-title"><FileText :size="15" :stroke-width="1.7" /><span>课件内容</span></h3>
          <textarea
            v-model="scriptText"
            class="vc-textarea"
            placeholder="把课件内容贴在这里，或在下方上传文件作为素材…"
            rows="10"
          />
          <div class="vc-meta-row">
            <span :class="{ warn: charCount < 20 && uploads.length === 0 }">
              {{ charCount }} 字{{ charCount < 20 && uploads.length === 0 ? " · 至少 20 字或上传文件" : "" }}
            </span>
            <button class="vc-ghost-btn" @click="fillDemo">填入示例</button>
          </div>

          <!-- 上传 -->
          <div class="vc-upload">
            <button class="vc-ghost-btn wide" :disabled="uploading" @click="pickFiles">
              <Loader v-if="uploading" :size="13" class="spin" /><Upload v-else :size="13" />
              <span>上传文件（md / docx / pdf / pptx / txt…）</span>
            </button>
            <div v-if="uploads.length" class="vc-files">
              <div v-for="(u, i) in uploads" :key="u.path" class="vc-file">
                <FileText :size="12" />
                <span class="vc-file-name">{{ u.name }}</span>
                <button class="vc-file-x" @click="removeUpload(i)"><X :size="12" /></button>
              </div>
            </div>
          </div>
        </div>

        <!-- 右：参数 -->
        <div class="vc-card">
          <h3 class="vc-card-title"><Gauge :size="15" :stroke-width="1.7" /><span>视频参数</span></h3>

          <!-- 时长 -->
          <div class="vc-field">
            <label class="vc-field-label"><Clock :size="13" /> 视频时长</label>
            <div class="vc-dur">
              <input type="number" min="15" max="3600" step="15" v-model.number="durationSec" class="vc-num" />
              <span class="vc-unit">秒</span>
              <span class="vc-dur-txt">≈ {{ durationText }}</span>
              <div class="vc-presets">
                <button
                  v-for="p in durationPresets"
                  :key="p.sec"
                  class="vc-chip"
                  :class="{ active: durationSec === p.sec }"
                  @click="durationSec = p.sec"
                >{{ p.label }}</button>
              </div>
            </div>
          </div>

          <!-- 语速 -->
          <div class="vc-field">
            <label class="vc-field-label"><Gauge :size="13" /> 语速 <b>{{ speed.toFixed(2) }}×</b></label>
            <input type="range" min="0.5" max="2" step="0.05" v-model.number="speed" class="vc-range" />
          </div>

          <!-- 配音语言 -->
          <div class="vc-field">
            <label class="vc-field-label"><Languages :size="13" /> 配音语言</label>
            <select v-model="dubLang" class="vc-select">
              <option v-for="l in DUB_LANGS" :key="l.code" :value="l.code">{{ l.name }}</option>
            </select>
            <span v-if="dubLang !== 'zh-Hans'" class="vc-field-note">
              口播稿与台词会改用「{{ dubInfo.name }}」书写并合成（language_boost={{ dubInfo.boost }}）
            </span>
          </div>

          <!-- 音色 -->
          <div class="vc-field">
            <label class="vc-field-label"><Mic :size="13" /> 配音音色</label>
            <select v-model="voice" class="vc-select">
              <option v-for="v in VOICES" :key="v.id" :value="v.id">{{ v.name }}</option>
            </select>
          </div>

          <!-- 背景音乐 -->
          <div class="vc-field">
            <label class="vc-field-label"><Music :size="13" /> 背景音乐</label>
            <div class="vc-bgm">
              <button class="vc-ghost-btn" @click="pickBgm">
                <Music :size="12" /><span>{{ bgmName || "选择音频…" }}</span>
              </button>
              <button v-if="bgmPath" class="vc-file-x" @click="bgmPath = ''"><X :size="12" /></button>
            </div>
            <div v-if="bgmPath" class="vc-bgm-vol">
              <span>音量 {{ Math.round(bgmVolume * 100) }}%</span>
              <input type="range" min="0" max="0.6" step="0.02" v-model.number="bgmVolume" class="vc-range sm" />
            </div>
          </div>
        </div>

        <!-- 风格选择：整行 -->
        <div class="vc-card vc-span2">
          <h3 class="vc-card-title">
            <Palette :size="15" :stroke-width="1.7" /><span>PPT 风格</span>
            <span class="vc-pill">当前：{{ themeName }}</span>
          </h3>
          <div class="vc-theme-wrap">
            <!-- 选中主题实时预览 -->
            <div
              class="vc-theme-preview"
              :style="{ background: curTheme.bg, color: curTheme.text, borderColor: curTheme.dark ? 'rgba(255,255,255,.14)' : 'rgba(0,0,0,.1)' }"
            >
              <div
                class="vc-pv-kicker"
                :style="{ color: curTheme.accent, fontFamily: curTheme.font === 'serif' ? 'var(--serif)' : curTheme.font === 'mono' ? 'monospace' : 'inherit' }"
              >PREVIEW · {{ curTheme.name }}</div>
              <div class="vc-pv-title" :style="{ fontFamily: curTheme.font === 'serif' ? 'var(--serif)' : 'inherit' }">课件标题示意</div>
              <div class="vc-pv-lede">这套主题会作为 PPT 视觉风格的设计提示。</div>
              <div class="vc-pv-row">
                <span class="vc-pv-dot" :style="{ background: curTheme.accent }"></span>
                <span class="vc-pv-bar" :style="{ background: curTheme.accent, opacity: .85 }"></span>
                <span class="vc-pv-bar sm" :style="{ background: curTheme.text, opacity: .25 }"></span>
              </div>
            </div>
            <!-- 主题网格 -->
            <div class="vc-themes">
              <button
                v-for="t in themes"
                :key="t.id"
                class="vc-theme"
                :class="{ active: selectedTheme === t.id, auto: t.id === 'auto' }"
                @click="selectedTheme = t.id"
              >
                <div class="vc-theme-sw" :style="{ background: t.bg }">
                  <span v-if="t.id === 'auto'" class="vc-theme-auto-i"><Sparkles :size="15" /></span>
                  <span v-else class="vc-theme-accent" :style="{ background: t.accent }"></span>
                </div>
                <div class="vc-theme-name">{{ t.name }}</div>
                <CheckCircle2 v-if="selectedTheme === t.id" :size="14" class="vc-theme-check" />
              </button>
            </div>
          </div>
        </div>

        <!-- 字幕语言：整行 -->
        <div class="vc-card vc-span2">
          <h3 class="vc-card-title">
            <Captions :size="15" :stroke-width="1.7" /><span>字幕</span>
            <span class="vc-pill">{{ subLangs.length ? subLangs.length + " 种语言" : "不加字幕" }}</span>
          </h3>
          <div class="vc-subs">
            <button
              v-for="l in SUB_LANGS"
              :key="l.code"
              class="vc-sub-chip"
              :class="{ active: subLangs.includes(l.code) }"
              @click="toggleSub(l.code)"
            >
              <span
                v-if="subLangs.includes(l.code)"
                class="vc-sub-order"
              >{{ subLangs.indexOf(l.code) + 1 }}</span>
              <span>{{ l.name }}</span>
            </button>
          </div>
          <div class="vc-sub-foot">
            <label class="vc-sub-burn" :class="{ on: burnSubs }">
              <input type="checkbox" v-model="burnSubs" />
              <span class="vc-switch sm"><span class="vc-knob"></span></span>
              <span>烧录硬字幕</span>
            </label>
            <span class="vc-sub-hint">
              {{
                !subLangs.length
                  ? "不点选则不加字幕。"
                  : burnSubs
                    ? `前 ${burnList.length} 种（${burnList.map(subLangName).join(" + ")}）压进画面任何播放器可见，其余作可切换软字幕 + .srt。`
                    : "全部作为可切换软字幕轨 + 同名 .srt，不压进画面。"
              }}
            </span>
          </div>
        </div>

        <!-- 操作 -->
        <div class="vc-actions vc-span2">
          <div v-if="error" class="vc-error">{{ error }}</div>
          <button class="vc-primary" :disabled="!canPlan" @click="startPlan">
            <Zap v-if="autoMode" :size="16" :stroke-width="1.9" />
            <Sparkles v-else :size="16" :stroke-width="1.8" />
            <span>{{ autoMode ? "全自动一键出片" : "开始规划" }}</span>
          </button>
          <p class="vc-hint">
            {{ autoMode
              ? "全自动：从规划到出片一路跑完，不停下来确认。"
              : "先生成「逐字稿 / PPT 风格与动效 / 口播稿」三份文件供你查看确认。" }}
          </p>
        </div>
      </section>

      <!-- ════════ 规划中 ════════ -->
      <section v-else-if="phase === 'planning'" class="vc-center">
        <!-- 进行中 -->
        <template v-if="!planStalled">
          <Loader :size="34" class="spin vc-big-spin" />
          <h2 class="vc-center-title">正在规划三份文件…</h2>
          <p class="vc-center-sub">逐字稿 · PPT 风格与动效 · 口播稿，就绪后会自动出现在这里。</p>
        </template>
        <!-- 已结束但没凑齐：明确提示 + 重试，而不是跳进点不了的 review -->
        <template v-else>
          <AlertTriangle :size="34" class="vc-warn-i" />
          <h2 class="vc-center-title">规划未凑齐三份文件</h2>
          <p class="vc-center-sub">
            已生成 {{ planFilesReadyCount }} / 3。可以重新规划，或先查看已生成的部分、在对话里继续。
          </p>
        </template>

        <div class="vc-plan-pending">
          <div
            v-for="f in planFiles"
            :key="f.key"
            class="vc-plan-dot"
            :class="{ ready: f.path }"
          >
            <CheckCircle2 v-if="f.path" :size="14" /><Circle v-else :size="14" />
            <span>{{ f.label }}</span>
          </div>
        </div>

        <div v-if="!planStalled">
          <button class="vc-ghost-btn" @click="openConv">在对话里看实时进度 →</button>
        </div>
        <div v-else class="vc-done-acts">
          <button class="vc-ghost-btn" @click="replan"><RefreshCw :size="13" /> 重新规划</button>
          <button
            v-if="planFilesReadyCount > 0"
            class="vc-ghost-btn"
            @click="phase = 'review'"
          ><Eye :size="13" /> 查看已生成的</button>
          <button class="vc-ghost-btn" @click="openConv"><Eye :size="13" /> 在对话里看</button>
        </div>
      </section>

      <!-- ════════ 规划确认 ════════ -->
      <section v-else-if="phase === 'review'" class="vc-review">
        <!-- 左：文件标签 -->
        <div class="vc-review-side">
          <div class="vc-review-head">规划产物</div>
          <button
            v-for="f in planFiles"
            :key="f.key"
            class="vc-review-tab"
            :class="{ active: activePlanTab === f.key, missing: !f.path }"
            @click="activePlanTab = f.key"
          >
            <FileText :size="14" />
            <div class="vc-review-tab-meta">
              <span class="vc-review-tab-label">{{ f.label }}</span>
              <span class="vc-review-tab-status">{{ f.path ? "已生成" : "未生成" }}</span>
            </div>
            <CheckCircle2 v-if="f.path" :size="14" class="ok" />
          </button>

          <div class="vc-review-acts">
            <button class="vc-primary" :disabled="!planReady" @click="confirmExecute">
              <CheckCircle2 :size="15" /><span>确认无误 · 开始执行</span>
            </button>
            <button class="vc-ghost-btn wide" @click="replan"><RefreshCw :size="13" /> 重新规划</button>
            <button class="vc-ghost-btn wide" @click="openConv"><Eye :size="13" /> 在对话里看</button>
          </div>
          <p v-if="!planReady" class="vc-warn-txt">部分文件尚未生成，可在对话里查看或重新规划。</p>
        </div>

        <!-- 右：文件内容 -->
        <div class="vc-review-viewer">
          <div class="vc-viewer-bar">
            <span class="vc-viewer-title">{{ activePlanFile?.label }}</span>
            <button
              v-if="activePlanFile?.path"
              class="vc-ghost-btn"
              @click="openFile(activePlanFile!.path!)"
            ><ExternalLink :size="12" /> 外部打开</button>
          </div>
          <pre v-if="activePlanFile?.text" class="vc-viewer-body">{{ activePlanFile.text }}</pre>
          <div v-else class="vc-viewer-empty">
            <FileText :size="28" />
            <span>{{ activePlanFile?.path ? "（空文件）" : "尚未生成" }}</span>
          </div>
        </div>
      </section>

      <!-- ════════ 执行中 ════════ -->
      <section v-else-if="phase === 'executing'" class="vc-center">
        <Loader :size="34" class="spin vc-big-spin" />
        <h2 class="vc-center-title">正在制作视频…</h2>
        <p class="vc-center-sub">开发 PPT → 配音 → 录屏 → 合成 MP4，约需几分钟。</p>
        <div class="vc-exec-steps">
          <div class="vc-exec-step"><Layers :size="14" /> 开发 HTML PPT</div>
          <div class="vc-exec-step"><Mic :size="14" /> MiniMax 配音（{{ VOICES.find(v=>v.id===voice)?.name }} · {{ speed.toFixed(2) }}×）</div>
          <div class="vc-exec-step"><Eye :size="14" /> 无头录屏</div>
          <div class="vc-exec-step"><VideoIcon :size="14" /> ffmpeg 合成{{ bgmPath ? " + 背景音乐" : "" }}</div>
        </div>
        <button class="vc-ghost-btn" @click="openConv">在对话里看实时进度 →</button>
      </section>

      <!-- ════════ 完成 ════════ -->
      <section v-else class="vc-center">
        <CheckCircle2 :size="40" class="vc-done-i" />
        <h2 class="vc-center-title">视频已生成</h2>
        <div v-if="outputs.length" class="vc-outputs">
          <button v-for="o in outputs" :key="o.path" class="vc-output" @click="openFile(o.path)">
            <VideoIcon :size="16" /><span>{{ o.name }}</span><ExternalLink :size="13" />
          </button>
        </div>
        <p v-else class="vc-center-sub">没在产物目录里探到 MP4，可在对话或目录里确认。</p>
        <div class="vc-done-acts">
          <button class="vc-ghost-btn" @click="openDir"><FolderOpen :size="13" /> 打开产物目录</button>
          <button class="vc-ghost-btn" @click="openConv"><Eye :size="13" /> 在对话里看</button>
          <button class="vc-ghost-btn" @click="reset"><RefreshCw :size="13" /> 再做一个</button>
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.vc {
  height: 100%;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--bg);
}
.vc-head {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 14px 22px;
  border-bottom: 1px solid var(--border-soft);
  background: var(--panel);
}
.vc-icon { color: var(--primary); }
.vc-title { font-family: var(--serif); font-size: 17px; font-weight: 600; color: var(--text); }
.vc-sub { font-size: 12.5px; color: var(--muted); margin-left: 6px; }

/* 全自动开关 */
.vc-auto {
  margin-left: auto;
  display: inline-flex;
  align-items: center;
  gap: 7px;
  font-size: 12.5px;
  font-weight: 600;
  color: var(--muted);
  cursor: pointer;
  user-select: none;
}
.vc-auto.on { color: var(--primary-deep); }
.vc-auto input { display: none; }
.vc-switch {
  position: relative;
  width: 34px;
  height: 19px;
  border-radius: 999px;
  background: var(--border-strong);
  transition: background 0.18s;
}
.vc-auto.on .vc-switch { background: var(--primary); }
.vc-knob {
  position: absolute;
  top: 2px; left: 2px;
  width: 15px; height: 15px;
  border-radius: 50%;
  background: #fff;
  transition: transform 0.18s;
}
.vc-auto.on .vc-knob { transform: translateX(15px); }

/* 流程条 */
.vc-flow {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 10px 22px;
  background: var(--bg-soft);
  border-bottom: 1px solid var(--border-soft);
}
.vc-flow-step {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 6px 12px;
  border-radius: 999px;
  font-size: 12.5px;
  font-weight: 600;
  color: var(--muted);
  background: transparent;
}
.vc-flow-step.active { color: #fff; background: var(--primary); }
.vc-flow-step.done { color: var(--primary-deep); background: var(--primary-soft); }
.vc-flow-step.skip { opacity: 0.45; }
.vc-flow-sep { color: var(--border-strong); }

.vc-body { flex: 1; overflow: auto; padding: 18px 22px; }

/* 网格 */
.vc-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 16px;
}
.vc-span2 { grid-column: 1 / -1; }
@media (max-width: 880px) { .vc-grid { grid-template-columns: 1fr; } .vc-span2 { grid-column: auto; } }

.vc-card {
  padding: 16px 18px;
  border: 1px solid var(--border-soft);
  border-radius: 12px;
  background: var(--panel);
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.vc-card-title {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
  margin: 0;
}
.vc-pill {
  margin-left: auto;
  font-size: 11px;
  font-weight: 500;
  color: var(--muted);
  padding: 2px 9px;
  background: var(--bg-soft);
  border-radius: 999px;
}

.vc-textarea {
  width: 100%;
  resize: vertical;
  min-height: 180px;
  padding: 12px 14px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--bg);
  color: var(--text);
  font-size: 13.5px;
  line-height: 1.7;
}
.vc-textarea:focus { outline: none; border-color: var(--primary); }
.vc-meta-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 11.5px;
  color: var(--muted);
}
.vc-meta-row .warn { color: var(--vermilion); }

.vc-ghost-btn {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 6px 11px;
  border: 1px solid var(--border);
  border-radius: 7px;
  background: transparent;
  color: var(--text-2);
  font-size: 12px;
  cursor: pointer;
  transition: border-color 0.15s, color 0.15s;
}
.vc-ghost-btn:hover:not(:disabled) { border-color: var(--primary); color: var(--primary); }
.vc-ghost-btn:disabled { opacity: 0.5; cursor: default; }
.vc-ghost-btn.wide { width: 100%; justify-content: center; }

/* 上传 */
.vc-upload { display: flex; flex-direction: column; gap: 8px; }
.vc-files { display: flex; flex-direction: column; gap: 5px; }
.vc-file {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 9px;
  background: var(--bg-soft);
  border-radius: 6px;
  font-size: 12px;
  color: var(--text-2);
}
.vc-file-name { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.vc-file-x {
  border: none;
  background: transparent;
  color: var(--muted);
  cursor: pointer;
  display: inline-flex;
  padding: 2px;
}
.vc-file-x:hover { color: var(--vermilion); }

/* 参数字段 */
.vc-field { display: flex; flex-direction: column; gap: 7px; }
.vc-field-label {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 12px;
  font-weight: 500;
  color: var(--muted);
}
.vc-field-label b { color: var(--primary-deep); margin-left: 2px; }
.vc-dur { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.vc-num {
  width: 84px;
  padding: 7px 10px;
  border: 1px solid var(--border);
  border-radius: 7px;
  background: var(--bg);
  color: var(--text);
  font-size: 13px;
}
.vc-num:focus { outline: none; border-color: var(--primary); }
.vc-unit { font-size: 12px; color: var(--muted); }
.vc-dur-txt { font-size: 12px; color: var(--primary-deep); font-weight: 500; }
.vc-presets { display: flex; gap: 4px; margin-left: auto; }
.vc-chip {
  padding: 5px 11px;
  border: 1px solid var(--border);
  border-radius: 7px;
  background: var(--bg);
  color: var(--text-2);
  font-size: 12px;
  cursor: pointer;
}
.vc-chip.active { border-color: var(--primary); background: var(--primary-soft); color: var(--primary-deep); }

.vc-range { width: 100%; accent-color: var(--primary); }
.vc-range.sm { flex: 1; }
.vc-select {
  padding: 8px 11px;
  border: 1px solid var(--border);
  border-radius: 7px;
  background: var(--bg);
  color: var(--text);
  font-size: 13px;
}
.vc-select:focus { outline: none; border-color: var(--primary); }
.vc-field-note { font-size: 11px; color: var(--primary-deep); line-height: 1.5; }

/* 字幕语言多选 */
.vc-subs { display: flex; flex-wrap: wrap; gap: 7px; }
.vc-sub-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 6px 12px;
  border: 1px solid var(--border);
  border-radius: 999px;
  background: var(--bg);
  color: var(--text-2);
  font-size: 12px;
  cursor: pointer;
  transition: border-color 0.15s, background 0.15s, color 0.15s;
}
.vc-sub-chip:hover { border-color: var(--primary); }
.vc-sub-chip.active { border-color: var(--primary); background: var(--primary-soft); color: var(--primary-deep); }
.vc-sub-order {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: var(--primary);
  color: #fff;
  font-size: 10px;
  font-weight: 700;
}
.vc-sub-foot { display: flex; align-items: center; gap: 14px; flex-wrap: wrap; }
.vc-sub-burn {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  font-size: 12px;
  font-weight: 600;
  color: var(--muted);
  cursor: pointer;
  user-select: none;
  white-space: nowrap;
}
.vc-sub-burn.on { color: var(--primary-deep); }
.vc-sub-burn input { display: none; }
.vc-switch.sm { width: 30px; height: 17px; }
.vc-sub-burn .vc-knob { width: 13px; height: 13px; }
.vc-sub-burn.on .vc-switch { background: var(--primary); }
.vc-sub-burn.on .vc-knob { transform: translateX(13px); }
.vc-sub-hint { font-size: 11.5px; color: var(--muted); line-height: 1.5; }

.vc-bgm { display: flex; align-items: center; gap: 8px; }
.vc-bgm .vc-ghost-btn { flex: 1; }
.vc-bgm-vol { display: flex; align-items: center; gap: 10px; font-size: 11.5px; color: var(--muted); }

/* 主题 */
.vc-themes {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(118px, 1fr));
  gap: 8px;
}
.vc-theme {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 8px;
  border: 1px solid var(--border);
  border-radius: 9px;
  background: var(--bg);
  cursor: pointer;
  text-align: left;
  transition: border-color 0.15s, transform 0.1s;
}
.vc-theme:hover { border-color: var(--primary); }
.vc-theme.active { border-color: var(--primary); background: var(--primary-soft); }
.vc-theme-sw {
  height: 38px;
  border-radius: 6px;
  border: 1px solid rgba(0,0,0,0.08);
  position: relative;
  overflow: hidden;
  display: flex;
  align-items: center;
  justify-content: center;
}
.vc-theme-accent {
  position: absolute;
  bottom: 0; left: 0; right: 0;
  height: 32%;
  opacity: 0.92;
}
.vc-theme-auto-i { color: #fff; }
.vc-theme-name { font-size: 11.5px; font-weight: 500; color: var(--text); }
.vc-theme-check { position: absolute; top: 6px; right: 6px; color: var(--primary); }

/* 主题区：左预览 + 右网格 */
.vc-theme-wrap { display: grid; grid-template-columns: 280px 1fr; gap: 16px; align-items: start; }
@media (max-width: 760px) { .vc-theme-wrap { grid-template-columns: 1fr; } }
.vc-theme-preview {
  border: 1px solid var(--border); border-radius: 10px; padding: 16px 18px;
  display: flex; flex-direction: column; gap: 7px; min-height: 150px; justify-content: center; overflow: hidden;
}
.vc-pv-kicker { font-size: 10.5px; font-weight: 700; letter-spacing: .14em; text-transform: uppercase; }
.vc-pv-title { font-size: 24px; font-weight: 800; letter-spacing: -.01em; }
.vc-pv-lede { font-size: 12px; opacity: .72; }
.vc-pv-row { display: flex; align-items: center; gap: 8px; margin-top: 8px; }
.vc-pv-dot { width: 13px; height: 13px; border-radius: 50%; }
.vc-pv-bar { height: 8px; width: 96px; border-radius: 4px; }
.vc-pv-bar.sm { width: 52px; }
.vc-warn-i { color: var(--warn, #b07a1f); }

/* 操作 */
.vc-actions {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding-top: 4px;
}
.vc-primary {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 12px 28px;
  border: none;
  border-radius: 10px;
  background: var(--primary);
  color: #fff;
  font-size: 14px;
  font-weight: 600;
  cursor: pointer;
  transition: filter 0.15s;
}
.vc-primary:hover:not(:disabled) { filter: brightness(1.07); }
.vc-primary:disabled { opacity: 0.5; cursor: default; }
.vc-hint { font-size: 12px; color: var(--muted); text-align: center; margin: 0; }
.vc-error {
  padding: 10px 12px;
  border-radius: 8px;
  background: var(--vermilion-soft);
  color: var(--vermilion);
  font-size: 12.5px;
  width: 100%;
}

/* 居中态（规划中 / 执行中 / 完成） */
.vc-center {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  min-height: 360px;
  text-align: center;
}
.vc-big-spin { color: var(--primary); }
.vc-done-i { color: #2e7d32; }
.vc-center-title { font-size: 18px; font-weight: 600; color: var(--text); margin: 4px 0 0; }
.vc-center-sub { font-size: 13px; color: var(--muted); margin: 0; max-width: 440px; }
.vc-plan-pending { display: flex; gap: 10px; margin: 6px 0; flex-wrap: wrap; justify-content: center; }
.vc-plan-dot {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 7px 13px;
  border: 1px solid var(--border-soft);
  border-radius: 999px;
  font-size: 12px;
  color: var(--muted);
}
.vc-plan-dot.ready { color: #2e7d32; border-color: rgba(46,125,50,0.4); }
.vc-exec-steps { display: flex; flex-direction: column; gap: 6px; margin: 6px 0; }
.vc-exec-step {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  font-size: 12.5px;
  color: var(--text-2);
}

/* 完成产物 */
.vc-outputs { display: flex; flex-direction: column; gap: 8px; margin: 4px 0; }
.vc-output {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 10px 16px;
  border: 1px solid var(--primary);
  border-radius: 9px;
  background: var(--primary-soft);
  color: var(--primary-deep);
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
}
.vc-output:hover { filter: brightness(1.03); }
.vc-done-acts { display: flex; gap: 8px; margin-top: 6px; flex-wrap: wrap; justify-content: center; }

/* 规划确认布局 */
.vc-review {
  display: grid;
  grid-template-columns: 240px 1fr;
  gap: 16px;
  min-height: 420px;
}
@media (max-width: 880px) { .vc-review { grid-template-columns: 1fr; } }
.vc-review-side { display: flex; flex-direction: column; gap: 8px; }
.vc-review-head { font-size: 12px; font-weight: 600; color: var(--muted); margin-bottom: 2px; }
.vc-review-tab {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 11px 12px;
  border: 1px solid var(--border-soft);
  border-radius: 9px;
  background: var(--panel);
  cursor: pointer;
  text-align: left;
  transition: border-color 0.15s;
}
.vc-review-tab:hover { border-color: var(--primary); }
.vc-review-tab.active { border-color: var(--primary); background: var(--primary-soft); }
.vc-review-tab.missing { opacity: 0.6; }
.vc-review-tab-meta { display: flex; flex-direction: column; flex: 1; min-width: 0; }
.vc-review-tab-label { font-size: 12.5px; font-weight: 600; color: var(--text); }
.vc-review-tab-status { font-size: 10.5px; color: var(--muted); }
.vc-review-tab .ok { color: #2e7d32; }
.vc-review-acts { display: flex; flex-direction: column; gap: 8px; margin-top: 8px; }
.vc-review-acts .vc-primary { width: 100%; padding: 10px; font-size: 13px; }
.vc-warn-txt { font-size: 11.5px; color: var(--vermilion); margin: 0; }

.vc-review-viewer {
  border: 1px solid var(--border-soft);
  border-radius: 12px;
  background: var(--panel);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.vc-viewer-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 14px;
  border-bottom: 1px solid var(--border-soft);
  background: var(--bg-soft);
}
.vc-viewer-title { font-size: 12.5px; font-weight: 600; color: var(--text); }
.vc-viewer-body {
  flex: 1;
  margin: 0;
  padding: 16px 18px;
  overflow: auto;
  font-family: var(--mono, monospace);
  font-size: 12.5px;
  line-height: 1.75;
  color: var(--text-2);
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 520px;
}
.vc-viewer-empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  color: var(--muted);
  font-size: 13px;
  min-height: 200px;
}

.spin { animation: vc-spin 0.9s linear infinite; }
@keyframes vc-spin { to { transform: rotate(360deg); } }
</style>
