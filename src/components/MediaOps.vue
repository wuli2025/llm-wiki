<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import {
  Megaphone,
  Newspaper,
  BookMarked,
  Sparkles,
  Zap,
  Upload,
  FileText,
  X,
  Loader,
  Send,
  LayoutDashboard,
  Library,
  Radar,
  Plus,
  Trash2,
  Check,
  ChevronRight,
  KeyRound,
  RefreshCw,
  LogOut,
} from "@lucide/vue";
import { useAppStore } from "../stores/app";
import { useChatStore } from "../stores/chat";
import {
  chat as chatApi,
  skills as skillsApi,
  mediaAccounts,
  type AttachedFile,
  type Skill,
  type MediaAccountStatus,
} from "../tauri";
import { useFileDrop } from "../composables/useFileDrop";

const app = useAppStore();
const chat = useChatStore();

const PROJECT_NAME = "自媒体运营";

// ───────── 分区 ─────────
type Zone = "plan" | "dashboard" | "library" | "accounts";
const zone = ref<Zone>("plan");
const error = ref<string | null>(null);

// ───────── 运营规划：平台 / 模式 ─────────
type Platform = "wechat" | "xhs";
const platform = ref<Platform>("wechat");
const autoMode = ref(false);

// ═══════════ V5：第二步「只选技能」（风格 = 技能） ═══════════
interface SkillCard { id: string; label: string; hint: string; icon: string; isSkill: boolean; wx?: boolean }

// 文风技能（单选）—— 作者沉淀的写作风格，挑一个定腔调
const AUTO_WRITE: SkillCard = { id: "__auto", label: "让 AI 定文风", hint: "AI 按平台+选题挑最合适的腔调，不再问你", icon: "🎲", isSkill: false };
const WX_WRITE: SkillCard[] = [
  { id: "deep", label: "深度评论体", hint: "冷静、有数据、就事论事", icon: "📐", isSkill: false },
  { id: "mao", label: "毛选雄辩体", hint: "矛盾分析、排比有力、立场鲜明", icon: "🔥", isSkill: false },
  { id: "ip", label: "个人 IP 故事体", hint: "第一人称、有温度、带经历", icon: "💬", isSkill: false },
  { id: "hot", label: "热点蹭刀体", hint: "蹭热点但有自己的角度", icon: "⚡", isSkill: false },
  AUTO_WRITE,
];
const XHS_WRITE: SkillCard[] = [
  { id: "real", label: "真诚分享体", hint: "像朋友安利、真实不端着", icon: "🤝", isSkill: false },
  { id: "contrast", label: "反差种草体", hint: "先抑后扬、制造反差钩子", icon: "🎢", isSkill: false },
  { id: "list", label: "干货清单体", hint: "分点、可收藏、信息密度高", icon: "📋", isSkill: false },
  { id: "emotion", label: "情绪共鸣体", hint: "戳痛点、引共鸣、带情绪", icon: "💗", isSkill: false },
  AUTO_WRITE,
];
const writeList = computed(() => (platform.value === "wechat" ? WX_WRITE : XHS_WRITE));
const selectedWrite = ref<string>("deep");           // 单选：文风
const customWriteSkillId = ref<string | null>(null); // 选「我沉淀的风格」时的真实 skill id
const writeLabel = computed(
  () => writeList.value.find((s) => s.id === selectedWrite.value)?.label ?? ""
);

// 选题 / 调研技能（多选）
const RESEARCH: SkillCard[] = [
  { id: "hot-topic-radar", label: "选题雷达", hint: "联网抓热点，给你几个选题挑", icon: "📡", isSkill: true },
  { id: "deep-research", label: "深度搜索", hint: "多源调研、查证事实", icon: "🔬", isSkill: true },
  { id: "__kb", label: "知识库补料", hint: "用你 KB 里的事实/数据补充", icon: "📚", isSkill: false },
];

// 排版 / 产出技能（多选，按平台切换）
const WX_OUTPUT: SkillCard[] = [
  { id: "wechat-md-typesetter", label: "壹伴排版优化", hint: "套主题压内联样式，进微信不乱版", icon: "🖋", isSkill: true, wx: true },
  { id: "cloak-browser", label: "CloakBrowser 直传", hint: "直注公众号编辑器存草稿，不格式错", icon: "🌐", isSkill: true, wx: true },
  { id: "image-gen", label: "AI 配图", hint: "自动配封面/插图，失败有兜底", icon: "🖼", isSkill: true },
  { id: "__deai", label: "去 AI 痕", hint: "把机翻腔改成人话", icon: "🪶", isSkill: false },
];
const XHS_OUTPUT: SkillCard[] = [
  { id: "gz-notion-infographic", label: "图卡渲染", hint: "把文案排成小红书方形图卡", icon: "🖼", isSkill: true, wx: true },
  { id: "cloak-browser", label: "CloakBrowser 待发包", hint: "浏览器自动登录，导出待发/半自动发布", icon: "🌐", isSkill: true, wx: true },
  { id: "image-gen", label: "AI 配图", hint: "自动配封面/插图，失败有兜底", icon: "🎨", isSkill: true },
  { id: "__deai", label: "去 AI 痕", hint: "把机翻腔改成人话", icon: "🪶", isSkill: false },
];
const outputList = computed(() => (platform.value === "wechat" ? WX_OUTPUT : XHS_OUTPUT));

const selResearch = ref<Set<string>>(new Set(["hot-topic-radar", "deep-research"]));
const selOutput = ref<Set<string>>(new Set(["wechat-md-typesetter", "cloak-browser"]));
const selCustom = ref<Set<string>>(new Set());

function toggleResearch(id: string) {
  const next = new Set(selResearch.value);
  next.has(id) ? next.delete(id) : next.add(id);
  selResearch.value = next;
}
function toggleOutput(id: string) {
  const next = new Set(selOutput.value);
  next.has(id) ? next.delete(id) : next.add(id);
  selOutput.value = next;
}
function toggleCustom(id: string) {
  const next = new Set(selCustom.value);
  next.has(id) ? next.delete(id) : next.add(id);
  selCustom.value = next;
}

// 自选 / 沉淀风格：从技能中心拉真实 skill 列表
const allSkills = ref<Skill[]>([]);
const showPicker = ref(false);
const pickerMode = ref<"write" | "custom">("custom");
const BUILTIN_IDS = new Set([
  "hot-topic-radar", "deep-research", "wechat-md-typesetter", "cloak-browser",
  "image-gen", "gz-notion-infographic", "wechat-pipeline", "xiaohongshu-pipeline",
]);
const pickableSkills = computed(() => allSkills.value.filter((s) => !BUILTIN_IDS.has(s.id)));
function openPicker(mode: "write" | "custom") {
  pickerMode.value = mode;
  showPicker.value = true;
}
function pickSkill(s: Skill) {
  if (pickerMode.value === "write") {
    customWriteSkillId.value = s.id;
    selectedWrite.value = ""; // 用沉淀的风格替代内置文风
  } else {
    toggleCustom(s.id);
  }
}

function pickPlatform(p: Platform) {
  if (platform.value === p) return;
  platform.value = p;
  selectedWrite.value = (p === "wechat" ? WX_WRITE : XHS_WRITE)[0].id;
  customWriteSkillId.value = null;
  selResearch.value = new Set(["hot-topic-radar", "deep-research"]);
  selOutput.value = p === "wechat"
    ? new Set(["wechat-md-typesetter", "cloak-browser"])
    : new Set(["gz-notion-infographic", "cloak-browser"]);
}
function pickWrite(id: string) {
  selectedWrite.value = id;
  customWriteSkillId.value = null;
}

// 一键推荐配置（小白兜底）
function applyRecommended() {
  selectedWrite.value = writeList.value[0].id;
  customWriteSkillId.value = null;
  selResearch.value = new Set(["hot-topic-radar", "deep-research"]);
  selOutput.value = platform.value === "wechat"
    ? new Set(["wechat-md-typesetter", "cloak-browser"])
    : new Set(["gz-notion-infographic", "cloak-browser"]);
  selCustom.value = new Set();
}

// id → 展示名（跨各组 + 真实 skill 列表解析）
function cardLabel(id: string): string {
  const all = [...writeList.value, ...RESEARCH, ...outputList.value];
  const c = all.find((x) => x.id === id);
  if (c) return c.label;
  return allSkills.value.find((s) => s.id === id)?.name ?? id;
}

// 最终 skillIds：基础链路 + 选中可作 skill 的项 + 沉淀风格 + 自选
const finalSkillIds = computed(() => {
  const ids = new Set<string>();
  ids.add(platform.value === "wechat" ? "wechat-pipeline" : "xiaohongshu-pipeline");
  selResearch.value.forEach((r) => { if (!r.startsWith("__")) ids.add(r); });
  selOutput.value.forEach((o) => { if (!o.startsWith("__")) ids.add(o); });
  if (customWriteSkillId.value) ids.add(customWriteSkillId.value);
  selCustom.value.forEach((c) => ids.add(c));
  return Array.from(ids);
});
const useKbFlag = computed(() => selResearch.value.has("__kb"));
const deaiFlag = computed(() => selOutput.value.has("__deai"));
const selectedCount = computed(
  () => 1 + selResearch.value.size + selOutput.value.size + selCustom.value.size
);
const selectedNames = computed(() => {
  const names: string[] = [
    customWriteSkillId.value ? cardLabel(customWriteSkillId.value) : writeLabel.value,
  ];
  selResearch.value.forEach((id) => names.push(cardLabel(id)));
  selOutput.value.forEach((id) => names.push(cardLabel(id)));
  selCustom.value.forEach((id) => names.push(cardLabel(id)));
  return names.filter(Boolean);
});

// ───────── 选题 / 方向：对话输入框 + 文件上传 ─────────
const topicText = ref("");
const uploads = ref<AttachedFile[]>([]);
const uploading = ref(false);
const convId = ref<string | null>(null);

async function addPaths(paths: string[], bucket: "plan" | "dash") {
  if (!paths.length) return;
  uploading.value = true;
  error.value = null;
  const target = bucket === "plan" ? uploads : dashUploads;
  try {
    const res = await chatApi.attachFiles(convId.value ?? undefined, paths);
    for (const r of res) {
      if (r.ok && !target.value.some((u) => u.path === r.path)) target.value.push(r);
    }
  } catch (e: any) {
    error.value = e?.message ?? String(e);
  } finally {
    uploading.value = false;
  }
}
async function pickFiles(bucket: "plan" | "dash") {
  try {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const sel = await open({
      multiple: true,
      filters: [
        { name: "素材", extensions: ["md", "txt", "docx", "pdf", "pptx", "html", "json", "csv", "png", "jpg", "jpeg"] },
      ],
    });
    if (!sel) return;
    await addPaths(Array.isArray(sel) ? sel : [sel], bucket);
  } catch (e: any) {
    error.value = e?.message ?? String(e);
  }
}
function removeUpload(i: number) {
  uploads.value.splice(i, 1);
}

// 原生拖拽落区：仅在「运营规划」分区生效
const { isOver: dropOver } = useFileDrop({
  active: () => app.view === "media_ops" && zone.value === "plan",
  onDrop: (p) => addPaths(p, "plan"),
});

// ───────── 模仿库（爆款参照，localStorage 持久化）─────────
interface RefItem { id: string; platform: Platform; title: string; content: string }
const REFS_KEY = "polaris:media-refs:v1";
// 首次进页面自动种入的「爆款文案模板」——种的是结构+钩子公式，非抄某篇原文。
// 用 marker 保证只种一次：用户删掉的不回种。
const REFS_SEED_KEY = "polaris:media-refs-seeded:v1";
const SEED_REFS: Array<Omit<RefItem, "id">> = [
  // ───── 微信公众号 ─────
  {
    platform: "wechat",
    title: "悬念冲突体｜标题埋钩子，开头三句留住人",
    content:
      "【标题公式】身份反差 / 数字悬念 / 认知颠覆 任选一：\n· 「我做了X年Y，今天说句得罪人的话」\n· 「90%的人都搞错了：关于X的3个真相」\n· 「那个被所有人看衰的X，最后赢了」\n【开头钩子】前3句必须制造缺口：抛一个反常识结论 或 一个具体到扎心的场景，先不给答案。\n【主体】结论先行 → 拆3个分论点（各配小标题）→ 每论点配1个故事或数据 → 金句收尾。\n【结尾】把观点拔高到价值观，留一句可转发的金句 + 一个互动提问。",
  },
  {
    platform: "wechat",
    title: "故事切入体｜用一个人的经历讲一个道理",
    content:
      "【钩子】开头白描一个具体瞬间：时间、地点、动作、一句对白，像电影第一帧。只给画面，不评论。\n【转折】故事走到一个反差点（失败/意外/顿悟），这里是情绪最高点。\n【升华】从这一个人推到一类人、一个普遍困境，让读者代入自己。\n【落点】给出观点或方法，但裹在故事里说，不说教。\n【节奏】每约800字埋一句可截图转发的话。",
  },
  {
    platform: "wechat",
    title: "雄辩排比体｜短句+排比+设问，气势压人",
    content:
      "【调性】判断句开局，不绕弯，毛选式雄辩。\n【节奏】三句一组排比，长短句交替，每段不超过4行。\n【句式库】「不是…而是…」「越是…越要…」「我们要问：…？答案是…」\n【结构】立靶（现象）→ 破（反驳流行看法）→ 立（给出真正答案）→ 号召。\n【收尾】一句斩钉截铁的判断 + 一个面向未来的动作号召。",
  },
  {
    platform: "wechat",
    title: "痛点共鸣体｜先扎心，再反转，最后给解药",
    content:
      "【开头】精准描述目标读者的一个具体痛点场景（越细越好：几点在干嘛、心里什么感受）。\n【共鸣】连续2-3个「你是不是也…」把读者钉在椅子上。\n【反转】「但问题其实不在你」——把锅从读者身上卸下，指向真正的结构性原因。\n【解药】给3步可立刻执行的方法，每步配一句话原理。\n【收尾】「从今天起，先做第一步就够了。」降低行动门槛。",
  },
  {
    platform: "wechat",
    title: "热点评论体｜蹭热点但有自己的刀",
    content:
      "【时效】事件发生24-48h内出。开头一句话交代事件，不复述细节（读者已知）。\n【角度】别人骂A，你找第二落点：「大家都在说X，但真正值得警惕的是Y」。\n【深挖】用1个历史类比或1组数据，把热点拉到更大的框架里。\n【观点】给出一个能被引用、被站队的鲜明判断。\n【克制】不蹭脏热点、不站危险队，落点回到对读者有用的启示。",
  },
  {
    platform: "wechat",
    title: "干货清单体｜N个方法，收藏率拉满",
    content:
      "【标题】数字+收益+人群：「整理了8年，这7个X方法，新手直接抄」\n【开头】一句话承诺价值 + 建议收藏：「全程干货，建议先收藏再看」。\n【主体】每条 = 小标题（动词开头）+ 1句原理 + 1个具体例子或模板，控制在5-9条。\n【格式】多用序号、加粗、短段，方便手机扫读。\n【收尾】「以上7条，挑1条今天就试。」+ 引导收藏转发。",
  },
  // ───── 小红书 ─────
  {
    platform: "xhs",
    title: "标题公式｜数字+身份+痛点+情绪词",
    content:
      "【万能公式】数字 + 身份标签 + 结果/痛点 + 情绪炸点 + emoji\n· 「普通人逆袭｜30天瘦8斤的5个习惯，第3个绝了😭」\n· 「打工人必看‼️这样做副业，我月入多了3000」\n· 「求求别再踩雷了😩 新手化妆这6步顺序千万别错」\n【情绪词库】绝了 / 救命 / 谁懂啊 / 血泪教训 / 后悔没早看 / 手把手 / 保姆级\n【限制】标题≤20字，1-2个emoji，一眼看到「我能得到什么」。",
  },
  {
    platform: "xhs",
    title: "痛点开头体｜前两行决定生死",
    content:
      "【机制】小红书只露前两行，必须在这里钩住。\n【开头】直接戳痛点或抛结果：「我真的会谢，踩了这个坑白花2000块」/「不是我吹，这方法谁用谁知道」。\n【正文】emoji分段，每段一个点，3-5个：\n✅ 第一步 …\n✅ 第二步 …\n每段≤3行，多用口语「家人们」「真的」「亲测」。\n【结尾】引导互动：「有用扣1，我出下一篇」+ 3-5个话题标签 #。",
  },
  {
    platform: "xhs",
    title: "测评对比体｜帮你做选择，信任感拉满",
    content:
      "【钩子】「买了8款X，只有2款值得无脑冲」——制造筛选感。\n【正文】表格化对比：每款 = 名字 + 价格 + 优点 + 缺点 + 适合谁，明确打分或排名。\n【真实感】必须有缺点，「踩雷」的也写出来，越敢说差越可信。\n【结论】给出「闭眼入 / 理性避雷 / 看情况」三档建议。\n【标签】#好物测评 #避雷 #平价替代。",
  },
  {
    platform: "xhs",
    title: "保姆级教程体｜步骤拆到手把手",
    content:
      "【标题】「保姆级教程｜0基础也能学会X，照着做就行」\n【开头】一句话说清「学完你能做到什么」+「需要准备什么」。\n【正文】Step1/Step2/Step3 编号，每步配一句话动作 + 一个易错点「⚠️这里千万别…」。\n【配图位】每步标注「（配图：…）」提示截图。\n【结尾】「收藏起来跟着做，卡住了评论区问我」+ 标签。",
  },
  {
    platform: "xhs",
    title: "逆袭故事体｜真实经历最带货",
    content:
      "【钩子】身份反差开局：「专科逆袭进大厂」「200斤到120斤」——先亮结果。\n【正文】时间线叙事：之前有多惨（具体细节）→ 转折点做了什么 → 现在怎样。\n【干货】把「我做对的3件事」抽成可复制的方法，别只晒结果。\n【情绪】真诚不端着，承认走过弯路，拉近距离。\n【收尾】「你也可以，从今天第一步开始」+ 鼓励性互动。",
  },
  {
    platform: "xhs",
    title: "种草安利体｜场景+情绪+理由",
    content:
      "【钩子】「闺蜜逼我安利的X，用完真的回不去了」——借第三方背书。\n【场景】把产品放进一个具体生活场景（什么时候用、解决了什么尴尬）。\n【卖点】挑1个核心卖点说透，别堆参数，讲「它让我的生活有什么不同」。\n【真实】加一句小缺点避免广告感：「唯一缺点是…但能接受」。\n【促动】「趁有活动冲一波」+ 价格锚点 + 标签。",
  },
];
const refs = ref<RefItem[]>([]);
const selectedRefIds = ref<Set<string>>(new Set());
const newRef = ref<{ platform: Platform; title: string; content: string }>({
  platform: "wechat",
  title: "",
  content: "",
});
function loadRefs() {
  try {
    const raw = localStorage.getItem(REFS_KEY);
    if (raw) refs.value = JSON.parse(raw);
  } catch {
    /* ignore */
  }
  // 首次进页面种入爆款模板（删除后不回种）
  if (!localStorage.getItem(REFS_SEED_KEY)) {
    const seeded = SEED_REFS.map((s) => ({ ...s, id: Math.random().toString(36).slice(2, 9) }));
    refs.value = [...seeded, ...refs.value];
    localStorage.setItem(REFS_SEED_KEY, "1");
    persistRefs();
  }
}
function persistRefs() {
  localStorage.setItem(REFS_KEY, JSON.stringify(refs.value));
}
function addRef() {
  const t = newRef.value.title.trim();
  const c = newRef.value.content.trim();
  if (!t && !c) return;
  refs.value.unshift({
    id: Math.random().toString(36).slice(2, 9),
    platform: newRef.value.platform,
    title: t || "（未命名爆款）",
    content: c,
  });
  newRef.value = { platform: platform.value, title: "", content: "" };
  persistRefs();
}
function removeRef(id: string) {
  refs.value = refs.value.filter((r) => r.id !== id);
  selectedRefIds.value.delete(id);
  persistRefs();
}
function toggleRef(id: string) {
  if (selectedRefIds.value.has(id)) selectedRefIds.value.delete(id);
  else selectedRefIds.value.add(id);
  selectedRefIds.value = new Set(selectedRefIds.value); // 触发响应
}
const selectedRefs = computed(() =>
  refs.value.filter((r) => selectedRefIds.value.has(r.id) && r.platform === platform.value)
);
const platformRefs = computed(() => refs.value.filter((r) => r.platform === platform.value));

// ───────── 数据看板（运营周报）─────────
const dashData = ref("");
const dashUploads = ref<AttachedFile[]>([]);
function removeDashUpload(i: number) {
  dashUploads.value.splice(i, 1);
}

// ───────── prompt 构建 ─────────
function planPrompt(): string {
  const plat = platform.value === "wechat" ? "微信公众号" : "小红书";
  const lines: string[] = [];
  lines.push(
    `我要运营【${plat}】。请按你的「全链路运营」技能跑，但**全程只保留一个需要我拍板的决策点：选题**。其余所有决定你自己拿主意，合并成一份《执行规划》一次性发我过目，我认可后一口气做到出稿+渲染+存草稿，中途别再逐项来回问我。平台、文风、所用技能我已经在面板上选好了（见下），不要再重复问我这些。`
  );
  const topic = topicText.value.trim();
  if (autoMode.value) {
    lines.push(
      "",
      "【模式 · 全自动】连选题也不用问我：你自己挑最优选题，先用一两句说清为什么选它，然后直接进规划 → 成稿 → 渲染 → 存草稿，全程不停。每步的关键判断都写出来供我复盘。"
    );
    if (topic) lines.push(`（我给了个大方向，优先围绕它选题：${topic}）`);
  } else {
    lines.push("", "【流程 · 只在选题处停一次】");
    if (topic) {
      lines.push(`1) 选题：方向我已经定了，直接用、不用再问我 —— ${topic}`);
    } else {
      lines.push(
        "1) 选题（**唯一**停下来等我的地方）：先做选题雷达，联网抓最近热点 + 对标爆文，给我 3-5 个具体选题（每个一句话点明角度 + 为什么值得写），编号让我挑；我也可能直接打字给方向，以我的输入为准。"
      );
    }
    lines.push(
      "2) 执行规划（不是逐项问我，是一次性给我一整份）：选题定了之后，把后续所有决定**合并成一份《执行规划》一次发我**——核心角度与论点、结构大纲、3 个备选标题、配图/封面方案、排版与投递方式。我回「继续 / 可以」就往下走，或我直接提修改；除此之外不要再分步征求我意见。",
      "3) 我认可规划后，一路成稿 → 渲染 → 存草稿，中途不再停。"
    );
  }
  lines.push("", "【写作风格】");
  if (selectedWrite.value === "__auto" && !customWriteSkillId.value) {
    lines.push("由你根据平台和选定的选题，自己挑最合适的文风，写进规划里告诉我即可，别单独拿风格来问我。");
  } else if (customWriteSkillId.value) {
    lines.push(`固定按我选定的写作技能「${cardLabel(customWriteSkillId.value)}」的风格来，不用再问我确认。`);
  } else {
    const w = writeList.value.find((s) => s.id === selectedWrite.value);
    if (w) lines.push(`固定用「${w.label}」（${w.hint}），不用再给我变体挑、也不用问我确认。`);
  }
  if (selectedRefs.value.length) {
    lines.push("", "【风格参照（模仿库）—— 对标其结构与钩子，但不要照抄】");
    for (const r of selectedRefs.value) {
      lines.push(`- ${r.title}：${r.content.slice(0, 140)}${r.content.length > 140 ? "…" : ""}`);
    }
  }
  if (uploads.value.length) {
    lines.push("", "【上传素材（请先 Read 这些文件作为内容来源）】");
    for (const u of uploads.value) lines.push(`- ${u.path}`);
  }
  if (deaiFlag.value) {
    lines.push(
      "",
      "【去 AI 痕 · 已选定，按此执行】成稿后做一遍口语化润色：消除机翻腔 / 八股腔，长短句交错，读起来像人写的。"
    );
  }
  // 排版 + 投递（仅公众号，且勾了对应技能时显式编排，确保"出文件→直送草稿"链路）
  if (platform.value === "wechat") {
    const wantTypeset = selOutput.value.has("wechat-md-typesetter");
    const wantCloak = selOutput.value.has("cloak-browser");
    if (wantTypeset || wantCloak) {
      lines.push("", "【排版 / 投递 · 已在面板选定，写进规划并按此执行，不用再问我】");
      if (wantTypeset)
        lines.push(
          "- 成稿后用「壹伴排版优化」技能把正文渲成微信兼容的内联样式 HTML（套主题、移动端字号、标题色块/引用块/分割线全内联），存成 .html 文件并报绝对路径，让我能先预览。"
        );
      if (wantCloak)
        lines.push(
          "- 然后用 CloakBrowser 打开公众号后台图文编辑器，把排好的 HTML 直接注入编辑器、正文图走素材库上传，保存为草稿（绝不自动发布），窗口留着让我核对后自己点发布。"
        );
    }
  }
  return lines.join("\n");
}

function dashboardPrompt(): string {
  const lines: string[] = [];
  lines.push(
    "请用你的「数据复盘 · 运营周报」技能，把我下面的运营数据做成一份周报：逐篇打优劣势、找出「哪类选题 / 标题 / 发布时机」数据好的规律、给下一轮主攻方向，并把可复用的结论回写知识库反哺选题。"
  );
  if (dashData.value.trim()) {
    lines.push("", "【运营数据】", dashData.value.trim());
  }
  if (dashUploads.value.length) {
    lines.push("", "【数据文件（请 Read）】");
    for (const u of dashUploads.value) lines.push(`- ${u.path}`);
  }
  if (!dashData.value.trim() && !dashUploads.value.length) {
    lines.push(
      "",
      "（我还没贴数据。请先告诉我：从公众号 / 小红书后台导出哪些字段、什么文件给你最方便复盘，再教我怎么定期喂给你。）"
    );
  }
  return lines.join("\n");
}

// ───────── 动作 ─────────
async function ensureConv(): Promise<string> {
  let project = app.projects.find((p) => p.name === PROJECT_NAME);
  let projectId: string | null = project?.id ?? null;
  if (!projectId) {
    await app.createProject(PROJECT_NAME);
    projectId = app.currentProjectId;
    if (!projectId) throw new Error("创建自媒体运营项目失败");
  }
  const conv = await app.createConversation(projectId);
  return conv.id;
}

const launching = ref(false);
const canStart = computed(() => !launching.value);

async function startPlan() {
  if (!canStart.value) return;
  error.value = null;
  launching.value = true;
  try {
    const id = await ensureConv();
    convId.value = id;
    let files: AttachedFile[] | undefined;
    if (uploads.value.length) {
      try {
        const res = await chatApi.attachFiles(id, uploads.value.map((u) => u.path));
        files = res.filter((r) => r.ok);
        uploads.value = files;
      } catch {
        files = uploads.value;
      }
    }
    app.setView("chat"); // 跳进对话框，看 AI 思考与决策
    const plat = platform.value === "wechat" ? "公众号" : "小红书";
    // 用户在「选技能」里点亮的技能（覆盖写死默认）：基础链路 + 选题/调研 + 排版/产出 + 自选
    const skillIds = finalSkillIds.value;
    const display = `📡 ${plat}·全链路${autoMode.value ? "(全自动)" : ""}：${preview()}`;
    await chat.send(id, planPrompt(), display, files, {
      permissionMode: "auto_current",
      skillIds,
      useKb: useKbFlag.value,
      goal: autoMode.value
        ? `把这条${plat}选题从选题→成稿→渲染一路做完并存草稿箱`
        : undefined,
    });
  } catch (e: any) {
    error.value = e?.message ?? String(e);
    app.setView("media_ops");
  } finally {
    launching.value = false;
  }
}

async function startDashboard() {
  if (!canStart.value) return;
  error.value = null;
  launching.value = true;
  try {
    const id = await ensureConv();
    convId.value = id;
    let files: AttachedFile[] | undefined;
    if (dashUploads.value.length) {
      try {
        const res = await chatApi.attachFiles(id, dashUploads.value.map((u) => u.path));
        files = res.filter((r) => r.ok);
        dashUploads.value = files;
      } catch {
        files = dashUploads.value;
      }
    }
    app.setView("chat");
    await chat.send(id, dashboardPrompt(), "📊 运营数据复盘 · 生成周报", files, {
      permissionMode: "auto_current",
      skillIds: ["content-analytics-report", "deep-research"],
      useKb: true,
    });
  } catch (e: any) {
    error.value = e?.message ?? String(e);
    app.setView("media_ops");
  } finally {
    launching.value = false;
  }
}

function preview(): string {
  const t = topicText.value.trim();
  if (t) return t.slice(0, 24) + (t.length > 24 ? "…" : "");
  if (uploads.value.length) return uploads.value[0].name;
  return "AI 来选题";
}

// ───────── 账号管理 ─────────
const accounts = ref<MediaAccountStatus[]>([]);
const accLoading = ref(false);
const accBusy = ref<string | null>(null); // 正在操作的 platform id
const accMsg = ref<string | null>(null);

async function loadAccounts() {
  accLoading.value = true;
  try {
    accounts.value = await mediaAccounts.status();
  } catch (e: any) {
    error.value = e?.message ?? String(e);
  } finally {
    accLoading.value = false;
  }
}

function fmtLastActive(secs: number | null): string {
  if (!secs) return "";
  const diff = Date.now() / 1000 - secs;
  if (diff < 3600) return `${Math.max(1, Math.floor(diff / 60))} 分钟前`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前`;
  return `${Math.floor(diff / 86400)} 天前`;
}

// 扫码绑定 / 检测登录态：拉起对话让 claude 跑对应技能（登录态由技能持久化到固定 profile）
async function runAccountTask(platform: "wechat" | "xhs", mode: "login" | "check") {
  if (accBusy.value) return;
  accBusy.value = platform;
  accMsg.value = null;
  error.value = null;
  try {
    const id = await ensureConv();
    convId.value = id;
    const isWx = platform === "wechat";
    const plat = isWx ? "微信公众号" : "小红书";
    const skill = isWx ? "post-to-wechat" : "post-to-xhs";
    let prompt: string;
    if (mode === "check") {
      prompt = `请用「${skill}」技能检测我的${plat}登录态：跑它的 check-login 命令，明确告诉我现在是「已登录」还是「未登录 / 已过期」，以及登录态持久化在哪个 profile 目录。不要登录、不要发布任何内容。`;
    } else if (isWx) {
      prompt =
        "请用「post-to-wechat」技能帮我登录微信公众号后台并把登录态存好：先跑 `python scripts/mp_draft.py check-login`；若显示已登录，直接告诉我「已登录、无需重扫」即可；若未登录，跑 `python scripts/mp_draft.py login` 打开浏览器让我扫码，登录态会持久化到 `~/.polaris-mp-profile`，**扫这一次，以后发文都复用这个登录态、不用再扫**。完成后用一句话确认状态。全程不要发布任何内容。";
    } else {
      prompt =
        "请用「post-to-xhs」技能帮我登录小红书并把登录态存好：先检测登录态（check-login）；若已登录，直接告诉我「已登录、无需重扫」；若未登录，走扫码登录（get-login-qrcode / login）让我扫码，登录态会持久化到 Chrome Profile，**扫这一次，以后发文都复用、不用再扫**。完成后用一句话确认状态。全程不要发布任何内容。";
    }
    const display = mode === "check" ? `🔎 检测${plat}登录态` : `🔑 扫码绑定${plat}账号`;
    app.setView("chat");
    await chat.send(id, prompt, display, undefined, { permissionMode: "auto_current", skillIds: [] });
  } catch (e: any) {
    error.value = e?.message ?? String(e);
    app.setView("media_ops");
  } finally {
    accBusy.value = null;
  }
}

async function forgetAccount(platform: "wechat" | "xhs") {
  if (accBusy.value) return;
  accBusy.value = platform;
  accMsg.value = null;
  error.value = null;
  try {
    accMsg.value = await mediaAccounts.forget(platform);
    await loadAccounts();
  } catch (e: any) {
    error.value = e?.message ?? String(e);
  } finally {
    accBusy.value = null;
  }
}

onMounted(async () => {
  loadRefs();
  loadAccounts();
  app.refreshProjects?.();
  try {
    allSkills.value = await skillsApi.list();
  } catch {
    allSkills.value = [];
  }
});
</script>

<template>
  <div class="mo">
    <!-- 顶栏 -->
    <header class="mo-head">
      <Megaphone :size="20" :stroke-width="1.7" class="mo-icon" />
      <h1 class="mo-title">自媒体运营</h1>
      <span class="mo-sub">AI 全链路：选题 → 风格 → 成稿 → 渲染 → 投递</span>
    </header>

    <div class="mo-body">
      <!-- 左：分区导航 -->
      <nav class="mo-nav">
        <button class="mo-nav-item" :class="{ active: zone === 'plan' }" @click="zone = 'plan'">
          <Sparkles :size="16" /><span>运营规划</span>
        </button>
        <button class="mo-nav-item" :class="{ active: zone === 'dashboard' }" @click="zone = 'dashboard'">
          <LayoutDashboard :size="16" /><span>数据看板</span>
        </button>
        <button class="mo-nav-item" :class="{ active: zone === 'library' }" @click="zone = 'library'">
          <Library :size="16" /><span>模仿库</span>
        </button>
        <button class="mo-nav-item" :class="{ active: zone === 'accounts' }" @click="zone = 'accounts'; loadAccounts()">
          <KeyRound :size="16" /><span>账号管理</span>
        </button>
        <div class="mo-nav-foot">
          <p>对话里<b>只在选题处停一次</b>，其余合并成一份规划过目，<b>全程看得见 AI 思考</b>。</p>
        </div>
      </nav>

      <!-- 右：工作区 -->
      <div class="mo-work">
        <!-- ════════ 运营规划 ════════ -->
        <section v-if="zone === 'plan'" class="mo-plan">
          <!-- 1 平台 -->
          <div class="mo-block">
            <div class="mo-block-h"><span class="mo-step">1</span> 选平台</div>
            <div class="mo-plats">
              <button
                class="mo-plat wx"
                :class="{ active: platform === 'wechat' }"
                @click="pickPlatform('wechat')"
              >
                <Newspaper :size="22" :stroke-width="1.6" />
                <div class="mo-plat-name">微信公众号</div>
                <div class="mo-plat-desc">深度长文 → 排版 HTML → 长图 / 草稿箱</div>
                <Check v-if="platform === 'wechat'" :size="15" class="mo-plat-check" />
              </button>
              <button
                class="mo-plat xhs"
                :class="{ active: platform === 'xhs' }"
                @click="pickPlatform('xhs')"
              >
                <BookMarked :size="22" :stroke-width="1.6" />
                <div class="mo-plat-name">小红书</div>
                <div class="mo-plat-desc">钩子文案 + 图卡 → 待发包</div>
                <Check v-if="platform === 'xhs'" :size="15" class="mo-plat-check" />
              </button>
            </div>
          </div>

          <!-- 一键推荐（小白兜底） -->
          <div class="mo-reco">
            <div class="mo-reco-ico"><Sparkles :size="17" /></div>
            <div class="mo-reco-txt">
              <div class="mo-reco-t">第一次用？一键套推荐配置</div>
              <div class="mo-reco-d">深度评论体 + 选题雷达 + 深度搜索 + 壹伴排版 + CloakBrowser 直传，够发一篇了。</div>
            </div>
            <button class="mo-reco-btn" @click="applyRecommended"><Check :size="13" /> 一键推荐</button>
          </div>

          <!-- 2 选题对话框 + 文件上传 -->
          <div class="mo-block">
            <div class="mo-block-h">
              <span class="mo-step">2</span> 说说想写什么（可留空让 AI 选题）
            </div>
            <div class="mo-compose" :class="{ over: dropOver }">
              <textarea
                v-model="topicText"
                class="mo-textarea"
                rows="4"
                placeholder="例：写写 OpenAI 首次盈利，从普通人能赚到什么切入… 　或留空，让 AI 先抓热点给你几个选题挑。可拖拽文件到这里作为素材。"
              />
              <div class="mo-compose-bar">
                <button class="mo-ghost" :disabled="uploading" @click="pickFiles('plan')">
                  <Loader v-if="uploading" :size="13" class="spin" /><Upload v-else :size="13" />
                  <span>上传素材</span>
                </button>
                <span v-if="dropOver" class="mo-drop-tip">松手即添加素材</span>
                <div class="mo-spacer" />
                <span class="mo-count">{{ topicText.length }} 字</span>
              </div>
              <div v-if="uploads.length" class="mo-files">
                <div v-for="(u, i) in uploads" :key="u.path" class="mo-file">
                  <FileText :size="12" />
                  <span class="mo-file-name">{{ u.name }}</span>
                  <button class="mo-file-x" @click="removeUpload(i)"><X :size="12" /></button>
                </div>
              </div>
            </div>
          </div>

          <!-- 3 选技能（风格=技能，全在这点亮） -->
          <div class="mo-block mo-skills-block">
            <div class="mo-block-h">
              <span class="mo-step">3</span> 选技能
              <span class="mo-h-hint">风格也是技能 · 点亮即用，灰的=没开</span>
            </div>

            <!-- A 文风技能（单选） -->
            <div class="mo-grp">
              <div class="mo-grp-h">
                <span class="mo-grp-i" style="background: rgba(111,176,255,.14)">🖊</span>
                <span class="mo-grp-t">文风技能</span>
                <span class="mo-grp-badge one">单选</span>
                <span class="mo-grp-tip">决定文章腔调，挑一个</span>
              </div>
              <div class="mo-cards">
                <button
                  v-for="w in writeList"
                  :key="w.id"
                  class="mo-card"
                  :class="{ on: !customWriteSkillId && selectedWrite === w.id }"
                  @click="pickWrite(w.id)"
                >
                  <span class="mo-card-i">{{ w.icon }}</span>
                  <span class="mo-card-b">
                    <span class="mo-card-n">{{ w.label }}</span>
                    <span class="mo-card-d">{{ w.hint }}</span>
                  </span>
                  <Check v-if="!customWriteSkillId && selectedWrite === w.id" :size="13" class="mo-card-ck" />
                </button>
                <button
                  class="mo-card add"
                  :class="{ on: !!customWriteSkillId }"
                  @click="openPicker('write')"
                >
                  <span class="mo-card-i">＋</span>
                  <span class="mo-card-b">
                    <span class="mo-card-n">{{ customWriteSkillId ? cardLabel(customWriteSkillId) : "我沉淀的风格…" }}</span>
                    <span class="mo-card-d">从技能中心挑你导入的写作 skill</span>
                  </span>
                  <Check v-if="customWriteSkillId" :size="13" class="mo-card-ck" />
                </button>
              </div>
            </div>

            <!-- B 选题 / 调研技能（多选） -->
            <div class="mo-grp">
              <div class="mo-grp-h">
                <span class="mo-grp-i" style="background: rgba(183,148,255,.14)">🔭</span>
                <span class="mo-grp-t">选题 / 调研技能</span>
                <span class="mo-grp-badge many">多选</span>
                <span class="mo-grp-tip">帮你找题、查料，可叠加</span>
              </div>
              <div class="mo-cards">
                <button
                  v-for="r in RESEARCH"
                  :key="r.id"
                  class="mo-card"
                  :class="{ on: selResearch.has(r.id) }"
                  @click="toggleResearch(r.id)"
                >
                  <span class="mo-card-i">{{ r.icon }}</span>
                  <span class="mo-card-b">
                    <span class="mo-card-n">{{ r.label }}</span>
                    <span class="mo-card-d">{{ r.hint }}</span>
                  </span>
                  <Check v-if="selResearch.has(r.id)" :size="13" class="mo-card-ck" />
                </button>
              </div>
            </div>

            <!-- C 排版 / 产出技能（多选） -->
            <div class="mo-grp">
              <div class="mo-grp-h">
                <span class="mo-grp-i" style="background: rgba(57,208,154,.14)">🎨</span>
                <span class="mo-grp-t">排版 / 产出技能</span>
                <span class="mo-grp-badge many">多选</span>
                <span class="mo-grp-tip">把文字变成能发的成品</span>
              </div>
              <div class="mo-cards">
                <button
                  v-for="o in outputList"
                  :key="o.id"
                  class="mo-card"
                  :class="{ on: selOutput.has(o.id), wx: o.wx }"
                  @click="toggleOutput(o.id)"
                >
                  <span class="mo-card-i">{{ o.icon }}</span>
                  <span class="mo-card-b">
                    <span class="mo-card-n">{{ o.label }}</span>
                    <span class="mo-card-d">{{ o.hint }}</span>
                  </span>
                  <Check v-if="selOutput.has(o.id)" :size="13" class="mo-card-ck" />
                </button>
              </div>
            </div>

            <!-- D 自选技能 -->
            <div class="mo-grp">
              <div class="mo-grp-h">
                <span class="mo-grp-i" style="background: rgba(230,184,115,.14)">➕</span>
                <span class="mo-grp-t">自选技能</span>
                <span class="mo-grp-tip">技能中心里任何一个都能挂上链</span>
              </div>
              <div class="mo-cards">
                <button
                  v-for="id in selCustom"
                  :key="id"
                  class="mo-card on"
                  @click="toggleCustom(id)"
                >
                  <span class="mo-card-i">🧩</span>
                  <span class="mo-card-b">
                    <span class="mo-card-n">{{ cardLabel(id) }}</span>
                    <span class="mo-card-d">已挂上，点取消</span>
                  </span>
                  <Check :size="13" class="mo-card-ck" />
                </button>
                <button class="mo-card add" @click="openPicker('custom')">
                  <span class="mo-card-i">＋</span>
                  <span class="mo-card-b">
                    <span class="mo-card-n">从技能中心挑…</span>
                    <span class="mo-card-d">搜名字勾上即用</span>
                  </span>
                </button>
              </div>
            </div>
          </div>

          <!-- 模仿库参照（本平台已存的爆款，可勾选） -->
          <div v-if="platformRefs.length" class="mo-block">
            <div class="mo-block-h">
              <span class="mo-step">·</span> 风格参照（可选 · 来自模仿库）
            </div>
            <div class="mo-refchips">
              <button
                v-for="r in platformRefs"
                :key="r.id"
                class="mo-refchip"
                :class="{ active: selectedRefIds.has(r.id) }"
                @click="toggleRef(r.id)"
              >
                <Check v-if="selectedRefIds.has(r.id)" :size="12" />
                <span>{{ r.title }}</span>
              </button>
            </div>
          </div>

          <!-- 已点亮技能汇总 -->
          <div class="mo-sel">
            <span class="mo-sel-n">已点亮 <b>{{ selectedCount }}</b> 个技能</span>
            <span class="mo-sel-names">{{ selectedNames.join(" · ") }}</span>
          </div>

          <!-- 启动 -->
          <div class="mo-launch">
            <div v-if="error" class="mo-error">{{ error }}</div>
            <label class="mo-auto" :class="{ on: autoMode }">
              <Zap :size="14" :stroke-width="1.9" />
              <span>全自动</span>
              <input type="checkbox" v-model="autoMode" />
              <span class="mo-switch"><span class="mo-knob"></span></span>
            </label>
            <button class="mo-primary" :disabled="!canStart" @click="startPlan">
              <Loader v-if="launching" :size="16" class="spin" /><Send v-else :size="15" />
              <span>{{ autoMode ? "一键全自动跑链路" : "进对话框 · 先挑选题" }}</span>
              <ChevronRight :size="15" />
            </button>
          </div>
          <p class="mo-foot-hint">
            进对话框后<b>只在「选题」处停一次</b>等你拍板，其余决定 AI 合并成<b>一份《执行规划》</b>给你过目，你回「继续」就一路成稿→渲染→存草稿。风格选「让 AI 定」则连风格也由 AI 判断，不再问你。
          </p>

          <!-- 技能选择器（沉淀风格 / 自选） -->
          <div v-if="showPicker" class="mo-picker-mask" @click.self="showPicker = false">
            <div class="mo-picker">
              <div class="mo-picker-h">
                <span>{{ pickerMode === "write" ? "选一个写作技能作为文风" : "从技能中心挑技能挂上链" }}</span>
                <button class="mo-file-x" @click="showPicker = false"><X :size="15" /></button>
              </div>
              <div class="mo-picker-list">
                <div v-if="!pickableSkills.length" class="mo-empty">
                  技能中心暂无可挂的额外技能，去「技能中心」导入后再来。
                </div>
                <button
                  v-for="s in pickableSkills"
                  :key="s.id"
                  class="mo-picker-item"
                  :class="{ on: pickerMode === 'write' ? customWriteSkillId === s.id : selCustom.has(s.id) }"
                  @click="pickSkill(s)"
                >
                  <span class="mo-card-i">🧩</span>
                  <span class="mo-card-b">
                    <span class="mo-card-n">{{ s.name }}</span>
                    <span class="mo-card-d">{{ s.description }}</span>
                  </span>
                  <Check
                    v-if="pickerMode === 'write' ? customWriteSkillId === s.id : selCustom.has(s.id)"
                    :size="14"
                    class="mo-card-ck"
                  />
                </button>
              </div>
              <div class="mo-picker-foot">
                <button class="mo-primary sm" @click="showPicker = false">完成</button>
              </div>
            </div>
          </div>
        </section>

        <!-- ════════ 数据看板 ════════ -->
        <section v-else-if="zone === 'dashboard'" class="mo-dash">
          <div class="mo-block">
            <div class="mo-block-h"><Radar :size="15" /> 运营数据复盘 · 一键周报</div>
            <p class="mo-desc">
              贴上后台数据（或导出文件），AI 逐篇打优劣势、找出数据好的选题/标题/时机规律、给下一轮主攻方向，并把结论回写知识库反哺选题。
              <br /><span class="mo-muted">每日自动爬取将在后续版本接入（走「自动化」板块定时调度）。</span>
            </p>
            <textarea
              v-model="dashData"
              class="mo-textarea"
              rows="6"
              placeholder="把公众号/小红书后台数据贴这里：标题、阅读/曝光、点赞、收藏、评论、涨粉、发布时间… 一行一篇即可。"
            />
            <div class="mo-compose-bar">
              <button class="mo-ghost" @click="pickFiles('dash')">
                <Upload :size="13" /><span>上传数据文件（csv / xlsx 导出）</span>
              </button>
            </div>
            <div v-if="dashUploads.length" class="mo-files">
              <div v-for="(u, i) in dashUploads" :key="u.path" class="mo-file">
                <FileText :size="12" />
                <span class="mo-file-name">{{ u.name }}</span>
                <button class="mo-file-x" @click="removeDashUpload(i)"><X :size="12" /></button>
              </div>
            </div>
          </div>
          <div class="mo-launch">
            <div v-if="error" class="mo-error">{{ error }}</div>
            <div class="mo-spacer" />
            <button class="mo-primary" :disabled="!canStart" @click="startDashboard">
              <Loader v-if="launching" :size="16" class="spin" /><LayoutDashboard v-else :size="15" />
              <span>生成运营周报</span>
            </button>
          </div>
        </section>

        <!-- ════════ 模仿库 ════════ -->
        <section v-else-if="zone === 'library'" class="mo-lib">
          <div class="mo-block">
            <div class="mo-block-h"><Plus :size="15" /> 收一条爆款参照</div>
            <div class="mo-ref-form">
              <div class="mo-ref-row">
                <select v-model="newRef.platform" class="mo-select">
                  <option value="wechat">公众号</option>
                  <option value="xhs">小红书</option>
                </select>
                <input v-model="newRef.title" class="mo-input flex" placeholder="标题 / 一句话标记这条为什么爆" />
              </div>
              <textarea
                v-model="newRef.content"
                class="mo-textarea"
                rows="3"
                placeholder="粘贴爆款正文 / 结构 / 钩子，成稿时供 AI 对标（不照抄）"
              />
              <div class="mo-compose-bar">
                <div class="mo-spacer" />
                <button class="mo-primary sm" @click="addRef"><Plus :size="14" /> 存入模仿库</button>
              </div>
            </div>
          </div>

          <div class="mo-block">
            <div class="mo-block-h"><Library :size="15" /> 已收藏（{{ refs.length }}）</div>
            <div v-if="!refs.length" class="mo-empty">还没收藏爆款。收几条小红书 / 公众号的爆款做风格对标，成稿更有据可依。</div>
            <div v-else class="mo-ref-list">
              <div v-for="r in refs" :key="r.id" class="mo-ref-item">
                <span class="mo-ref-tag" :class="r.platform">{{ r.platform === "wechat" ? "公众号" : "小红书" }}</span>
                <div class="mo-ref-main">
                  <div class="mo-ref-title">{{ r.title }}</div>
                  <div class="mo-ref-content">{{ r.content }}</div>
                </div>
                <button class="mo-file-x" @click="removeRef(r.id)"><Trash2 :size="14" /></button>
              </div>
            </div>
          </div>
        </section>

        <!-- ════════ 账号管理 ════════ -->
        <section v-else class="mo-acct">
          <div class="mo-acct-intro">
            <KeyRound :size="16" />
            <p>扫一次码，登录态就<b>存进这里</b>，之后发文自动复用、不用每次重扫。公众号 session 会过期，过期后回来重新扫一次即可。</p>
          </div>

          <div v-if="error" class="mo-error">{{ error }}</div>
          <div v-if="accMsg" class="mo-acct-msg">{{ accMsg }}</div>

          <div class="mo-acct-cards">
            <div v-for="a in accounts" :key="a.platform" class="mo-acct-card" :class="a.platform">
              <div class="mo-acct-head">
                <Newspaper v-if="a.platform === 'wechat'" :size="20" :stroke-width="1.6" />
                <BookMarked v-else :size="20" :stroke-width="1.6" />
                <span class="mo-acct-name">{{ a.label }}</span>
                <span class="mo-acct-badge" :class="{ on: a.bound }">
                  <Check v-if="a.bound" :size="12" />{{ a.bound ? " 已绑定" : "未绑定" }}
                </span>
              </div>
              <div v-if="a.bound && a.lastActive" class="mo-acct-meta">最近活动：{{ fmtLastActive(a.lastActive) }}</div>
              <p class="mo-acct-detail">{{ a.detail }}</p>
              <div class="mo-acct-path" :title="a.profileDir">登录态目录：{{ a.profileDir }}</div>
              <div class="mo-acct-actions">
                <button class="mo-primary sm" :disabled="!!accBusy" @click="runAccountTask(a.platform, 'login')">
                  <Loader v-if="accBusy === a.platform" :size="14" class="spin" /><KeyRound v-else :size="14" />
                  <span>{{ a.bound ? "重新扫码登录" : "扫码绑定" }}</span>
                </button>
                <button class="mo-ghost" :disabled="!!accBusy" @click="runAccountTask(a.platform, 'check')">
                  <RefreshCw :size="13" /><span>检测登录态</span>
                </button>
                <button v-if="a.bound" class="mo-ghost danger" :disabled="!!accBusy" @click="forgetAccount(a.platform)">
                  <LogOut :size="13" /><span>解绑</span>
                </button>
              </div>
            </div>
          </div>

          <button class="mo-ghost" :disabled="accLoading" @click="loadAccounts">
            <Loader v-if="accLoading" :size="13" class="spin" /><RefreshCw v-else :size="13" /><span>刷新状态</span>
          </button>
        </section>
      </div>
    </div>
  </div>
</template>

<style scoped>
.mo { height: 100%; display: flex; flex-direction: column; overflow: hidden; background: var(--bg); }
.mo-head {
  display: flex; align-items: center; gap: 10px;
  padding: 14px 22px; border-bottom: 1px solid var(--border-soft); background: var(--panel);
}
.mo-icon { color: var(--primary); }
.mo-title { font-family: var(--serif); font-size: 17px; font-weight: 600; color: var(--text); }
.mo-sub { font-size: 12.5px; color: var(--muted); margin-left: 6px; }

.mo-body { flex: 1; display: grid; grid-template-columns: 188px 1fr; overflow: hidden; }

/* 左导航 */
.mo-nav {
  border-right: 1px solid var(--border-soft); background: var(--bg-soft);
  padding: 14px 10px; display: flex; flex-direction: column; gap: 4px;
}
.mo-nav-item {
  display: flex; align-items: center; gap: 9px;
  padding: 10px 12px; border: none; border-radius: 9px; background: transparent;
  color: var(--text-2); font-size: 13.5px; font-weight: 500; cursor: pointer;
  transition: background 0.15s, color 0.15s; text-align: left;
}
.mo-nav-item:hover { background: var(--panel); color: var(--text); }
.mo-nav-item.active { background: var(--primary-soft); color: var(--primary-deep); font-weight: 600; }
.mo-nav-foot { margin-top: auto; padding: 10px 12px; }
.mo-nav-foot p { font-size: 11.5px; color: var(--muted); line-height: 1.6; margin: 0; }
.mo-nav-foot b { color: var(--primary-deep); }

/* 工作区 */
.mo-work { overflow: auto; padding: 18px 24px; }
.mo-plan, .mo-dash, .mo-lib { display: flex; flex-direction: column; gap: 16px; max-width: 820px; }

.mo-block {
  border: 1px solid var(--border-soft); border-radius: 12px; background: var(--panel);
  padding: 15px 17px; display: flex; flex-direction: column; gap: 12px;
}
.mo-block-h {
  display: flex; align-items: center; gap: 8px;
  font-size: 13.5px; font-weight: 600; color: var(--text);
}
.mo-step {
  display: inline-flex; align-items: center; justify-content: center;
  width: 20px; height: 20px; border-radius: 6px;
  background: var(--primary); color: #fff; font-size: 12px; font-weight: 700;
}
.mo-h-hint {
  margin-left: auto; display: inline-flex; align-items: center; gap: 4px;
  font-size: 11.5px; font-weight: 400; color: var(--muted);
}

/* 平台卡 */
.mo-plats { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
.mo-plat {
  position: relative; display: flex; flex-direction: column; gap: 5px;
  padding: 16px; border: 1.5px solid var(--border); border-radius: 11px;
  background: var(--bg); cursor: pointer; text-align: left; transition: border-color 0.15s, background 0.15s;
}
.mo-plat:hover { border-color: var(--primary); }
.mo-plat.active { border-color: var(--primary); background: var(--primary-soft); }
.mo-plat.wx { color: var(--text); }
.mo-plat .mo-plat-name { font-size: 15px; font-weight: 600; color: var(--text); }
.mo-plat-desc { font-size: 11.5px; color: var(--muted); line-height: 1.5; }
.mo-plat-check { position: absolute; top: 12px; right: 12px; color: var(--primary); }
.mo-plat.wx > svg:first-child { color: #1bbf83; }
.mo-plat.xhs > svg:first-child { color: #ff2e51; }

/* 风格 */
.mo-styles { display: flex; flex-wrap: wrap; gap: 8px; }
.mo-style {
  display: flex; flex-direction: column; gap: 2px; align-items: flex-start;
  padding: 8px 13px; border: 1px solid var(--border); border-radius: 9px;
  background: var(--bg); color: var(--text-2); cursor: pointer; transition: border-color 0.15s, background 0.15s;
}
.mo-style:hover { border-color: var(--primary); }
.mo-style.active { border-color: var(--primary); background: var(--primary-soft); }
.mo-style-name { font-size: 12.5px; font-weight: 600; color: var(--text); }
.mo-style-hint { font-size: 10.5px; color: var(--muted); }
.mo-style.auto, .mo-style.custom {
  flex-direction: row; align-items: center; gap: 6px; font-size: 12.5px; font-weight: 500;
}
.mo-style.auto.active, .mo-style.custom.active { color: var(--primary-deep); }

/* 输入 */
.mo-input {
  width: 100%; padding: 9px 12px; border: 1px solid var(--border); border-radius: 8px;
  background: var(--bg); color: var(--text); font-size: 13px;
}
.mo-input:focus { outline: none; border-color: var(--primary); }
.mo-input.flex { flex: 1; }

/* 选题对话框 */
.mo-compose {
  border: 1px solid var(--border); border-radius: 10px; background: var(--bg);
  padding: 4px; transition: border-color 0.15s, box-shadow 0.15s;
}
.mo-compose.over { border-color: var(--primary); box-shadow: 0 0 0 3px var(--primary-soft); }
.mo-textarea {
  width: 100%; resize: vertical; min-height: 84px;
  padding: 10px 12px; border: none; border-radius: 8px;
  background: transparent; color: var(--text); font-size: 13.5px; line-height: 1.7;
}
.mo-textarea:focus { outline: none; }
.mo-compose-bar { display: flex; align-items: center; gap: 8px; padding: 4px 8px 6px; }
.mo-spacer { flex: 1; }
.mo-count { font-size: 11px; color: var(--muted); }
.mo-drop-tip { font-size: 11.5px; color: var(--primary-deep); }

.mo-ghost {
  display: inline-flex; align-items: center; gap: 5px;
  padding: 6px 11px; border: 1px solid var(--border); border-radius: 7px;
  background: transparent; color: var(--text-2); font-size: 12px; cursor: pointer;
  transition: border-color 0.15s, color 0.15s;
}
.mo-ghost:hover:not(:disabled) { border-color: var(--primary); color: var(--primary); }
.mo-ghost:disabled { opacity: 0.5; cursor: default; }

.mo-files { display: flex; flex-wrap: wrap; gap: 6px; padding: 0 8px 6px; }
.mo-file {
  display: flex; align-items: center; gap: 6px;
  padding: 4px 9px; background: var(--bg-soft); border-radius: 6px; font-size: 12px; color: var(--text-2);
}
.mo-file-name { max-width: 180px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.mo-file-x {
  border: none; background: transparent; color: var(--muted); cursor: pointer;
  display: inline-flex; padding: 2px;
}
.mo-file-x:hover { color: var(--vermilion); }

/* 参照 chips */
.mo-refchips { display: flex; flex-wrap: wrap; gap: 7px; }
.mo-refchip {
  display: inline-flex; align-items: center; gap: 5px;
  padding: 6px 11px; border: 1px solid var(--border); border-radius: 999px;
  background: var(--bg); color: var(--text-2); font-size: 12px; cursor: pointer; transition: all 0.15s;
}
.mo-refchip:hover { border-color: var(--primary); }
.mo-refchip.active { border-color: var(--primary); background: var(--primary-soft); color: var(--primary-deep); }

/* 启动条 */
.mo-launch { display: flex; align-items: center; gap: 12px; }
.mo-auto {
  display: inline-flex; align-items: center; gap: 7px;
  font-size: 12.5px; font-weight: 600; color: var(--muted); cursor: pointer; user-select: none;
}
.mo-auto.on { color: var(--primary-deep); }
.mo-auto input { display: none; }
.mo-switch {
  position: relative; width: 34px; height: 19px; border-radius: 999px;
  background: var(--border-strong); transition: background 0.18s;
}
.mo-auto.on .mo-switch { background: var(--primary); }
.mo-knob {
  position: absolute; top: 2px; left: 2px; width: 15px; height: 15px;
  border-radius: 50%; background: #fff; transition: transform 0.18s;
}
.mo-auto.on .mo-knob { transform: translateX(15px); }
.mo-primary {
  margin-left: auto;
  display: inline-flex; align-items: center; justify-content: center; gap: 8px;
  padding: 11px 22px; border: none; border-radius: 10px;
  background: var(--primary); color: #fff; font-size: 14px; font-weight: 600;
  cursor: pointer; transition: filter 0.15s;
}
.mo-primary.sm { padding: 8px 15px; font-size: 13px; margin-left: 0; }
.mo-primary:hover:not(:disabled) { filter: brightness(1.07); }
.mo-primary:disabled { opacity: 0.55; cursor: default; }
.mo-foot-hint { font-size: 11.5px; color: var(--muted); line-height: 1.6; margin: -4px 0 0; }
.mo-error {
  padding: 9px 12px; border-radius: 8px; background: var(--vermilion-soft);
  color: var(--vermilion); font-size: 12.5px;
}

.mo-desc { font-size: 12.5px; color: var(--text-2); line-height: 1.7; margin: 0; }
.mo-muted { color: var(--muted); }

/* 模仿库 */
.mo-ref-form { display: flex; flex-direction: column; gap: 10px; }
.mo-ref-row { display: flex; gap: 8px; }
.mo-select {
  padding: 8px 11px; border: 1px solid var(--border); border-radius: 8px;
  background: var(--bg); color: var(--text); font-size: 13px;
}
.mo-select:focus { outline: none; border-color: var(--primary); }
.mo-empty { font-size: 12.5px; color: var(--muted); padding: 8px 2px; line-height: 1.7; }
.mo-ref-list { display: flex; flex-direction: column; gap: 8px; }
.mo-ref-item {
  display: flex; align-items: flex-start; gap: 10px;
  padding: 11px 13px; border: 1px solid var(--border-soft); border-radius: 10px; background: var(--bg);
}
.mo-ref-tag {
  flex: 0 0 auto; font-size: 10.5px; font-weight: 600; padding: 2px 8px; border-radius: 999px;
}
.mo-ref-tag.wechat { background: #11231c; color: #6fe0b6; }
.mo-ref-tag.xhs { background: #241016; color: #ff9aaa; }
.mo-ref-main { flex: 1; min-width: 0; }
.mo-ref-title { font-size: 13px; font-weight: 600; color: var(--text); }
.mo-ref-content {
  font-size: 12px; color: var(--muted); line-height: 1.6; margin-top: 3px;
  display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden;
}

/* ═══ V5 一键推荐 ═══ */
.mo-reco {
  display: flex; align-items: center; gap: 13px;
  padding: 12px 16px; border-radius: 12px;
  border: 1px solid rgba(57, 208, 154, 0.3);
  background: linear-gradient(100deg, rgba(57, 208, 154, 0.1), rgba(57, 208, 154, 0.02));
}
.mo-reco-ico {
  width: 36px; height: 36px; border-radius: 10px; flex: 0 0 auto;
  display: flex; align-items: center; justify-content: center;
  background: rgba(57, 208, 154, 0.16); color: #39d09a;
}
.mo-reco-t { font-size: 13.5px; font-weight: 600; color: #6fe0b6; }
.mo-reco-d { font-size: 12px; color: var(--muted); margin-top: 1px; }
.mo-reco-btn {
  margin-left: auto; flex: 0 0 auto; display: inline-flex; align-items: center; gap: 5px;
  padding: 8px 14px; border: none; border-radius: 9px; white-space: nowrap;
  background: linear-gradient(135deg, #5fe3b5, #39d09a); color: #06140d;
  font-size: 12.5px; font-weight: 600; cursor: pointer; transition: filter 0.15s;
}
.mo-reco-btn:hover { filter: brightness(1.06); }

/* ═══ V5 选技能 ═══ */
.mo-skills-block { gap: 18px; }
.mo-grp { display: flex; flex-direction: column; gap: 10px; }
.mo-grp-h { display: flex; align-items: center; gap: 9px; }
.mo-grp-i {
  width: 25px; height: 25px; border-radius: 7px; flex: 0 0 auto;
  display: flex; align-items: center; justify-content: center; font-size: 13px;
}
.mo-grp-t { font-size: 14px; font-weight: 600; color: var(--text); }
.mo-grp-badge { font-size: 10px; font-weight: 700; padding: 2px 8px; border-radius: 6px; }
.mo-grp-badge.one { background: rgba(111, 176, 255, 0.14); color: #6fb6ff; }
.mo-grp-badge.many { background: rgba(183, 148, 255, 0.14); color: #b794ff; }
.mo-grp-tip { font-size: 11.5px; color: var(--muted); }

.mo-cards {
  display: grid; grid-template-columns: repeat(auto-fill, minmax(214px, 1fr)); gap: 9px;
}
.mo-card {
  position: relative; display: flex; gap: 10px; align-items: flex-start; text-align: left;
  padding: 11px 12px; border-radius: 11px; border: 1px solid var(--border);
  background: var(--bg); cursor: pointer; transition: border-color 0.15s, background 0.15s;
}
.mo-card:hover { border-color: var(--primary); }
.mo-card-i {
  width: 30px; height: 30px; border-radius: 8px; flex: 0 0 auto;
  display: flex; align-items: center; justify-content: center; font-size: 15px;
  background: rgba(255, 255, 255, 0.04);
}
.mo-card-b { display: flex; flex-direction: column; min-width: 0; }
.mo-card-n { font-size: 13px; font-weight: 600; color: var(--text); }
.mo-card-d { font-size: 11px; color: var(--muted); line-height: 1.45; margin-top: 1px; }
.mo-card-ck { position: absolute; top: 9px; right: 10px; color: var(--primary-deep); }
.mo-card.on { border-color: var(--primary); background: var(--primary-soft); }
.mo-card.on .mo-card-i { background: rgba(217, 140, 63, 0.18); }
.mo-card.on.wx { border-color: #39d09a; background: rgba(57, 208, 154, 0.08); }
.mo-card.on.wx .mo-card-i { background: rgba(57, 208, 154, 0.18); }
.mo-card.on.wx .mo-card-ck { color: #6fe0b6; }
.mo-card.add { border-style: dashed; align-items: center; }
.mo-card.add .mo-card-i { color: var(--primary-deep); }
.mo-card.add .mo-card-n { color: var(--text-2); }

/* 已点亮汇总 */
.mo-sel {
  display: flex; align-items: baseline; gap: 10px; flex-wrap: wrap;
  padding: 10px 14px; border-radius: 10px; background: var(--bg-soft);
  border: 1px solid var(--border-soft);
}
.mo-sel-n { font-size: 12.5px; color: var(--text-2); white-space: nowrap; }
.mo-sel-n b { color: var(--primary-deep); }
.mo-sel-names { font-size: 11.5px; color: var(--muted); line-height: 1.5; }

/* 技能选择器弹层 */
.mo-picker-mask {
  position: fixed; inset: 0; z-index: 50; background: rgba(0, 0, 0, 0.55);
  display: flex; align-items: center; justify-content: center; padding: 24px;
}
.mo-picker {
  width: 560px; max-width: 100%; max-height: 78vh; display: flex; flex-direction: column;
  border: 1px solid var(--border-strong); border-radius: 14px; background: var(--panel);
  box-shadow: 0 24px 60px rgba(0, 0, 0, 0.5); overflow: hidden;
}
.mo-picker-h {
  display: flex; align-items: center; justify-content: space-between;
  padding: 14px 18px; border-bottom: 1px solid var(--border-soft);
  font-size: 14px; font-weight: 600; color: var(--text);
}
.mo-picker-list { overflow: auto; padding: 12px; display: flex; flex-direction: column; gap: 7px; }
.mo-picker-item {
  position: relative; display: flex; gap: 11px; align-items: flex-start; text-align: left;
  padding: 11px 13px; border-radius: 10px; border: 1px solid var(--border-soft);
  background: var(--bg); cursor: pointer; transition: border-color 0.15s;
}
.mo-picker-item:hover { border-color: var(--primary); }
.mo-picker-item.on { border-color: var(--primary); background: var(--primary-soft); }
.mo-picker-foot {
  display: flex; justify-content: flex-end; padding: 12px 16px;
  border-top: 1px solid var(--border-soft);
}

/* ═══ 账号管理 ═══ */
.mo-acct { display: flex; flex-direction: column; gap: 14px; max-width: 820px; }
.mo-acct-intro {
  display: flex; align-items: flex-start; gap: 10px;
  padding: 12px 15px; border-radius: 11px;
  border: 1px solid rgba(111, 176, 255, 0.28);
  background: linear-gradient(100deg, rgba(111, 176, 255, 0.1), rgba(111, 176, 255, 0.02));
  color: var(--primary-deep);
}
.mo-acct-intro p { font-size: 12.5px; color: var(--text-2); line-height: 1.65; margin: 0; }
.mo-acct-intro b { color: var(--primary-deep); }
.mo-acct-msg {
  padding: 9px 12px; border-radius: 8px; font-size: 12.5px;
  background: rgba(57, 208, 154, 0.1); color: #6fe0b6; border: 1px solid rgba(57, 208, 154, 0.28);
}
.mo-acct-cards { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
.mo-acct-card {
  display: flex; flex-direction: column; gap: 8px;
  padding: 15px 16px; border-radius: 12px; border: 1px solid var(--border-soft);
  background: var(--panel);
}
.mo-acct-card.wechat > .mo-acct-head > svg:first-child { color: #1bbf83; }
.mo-acct-card.xhs > .mo-acct-head > svg:first-child { color: #ff2e51; }
.mo-acct-head { display: flex; align-items: center; gap: 8px; }
.mo-acct-name { font-size: 14.5px; font-weight: 600; color: var(--text); }
.mo-acct-badge {
  margin-left: auto; display: inline-flex; align-items: center; gap: 2px;
  font-size: 11px; font-weight: 600; padding: 2px 9px; border-radius: 999px;
  background: var(--bg-soft); color: var(--muted);
}
.mo-acct-badge.on { background: rgba(57, 208, 154, 0.16); color: #6fe0b6; }
.mo-acct-meta { font-size: 11.5px; color: var(--muted); }
.mo-acct-detail { font-size: 12px; color: var(--text-2); line-height: 1.6; margin: 0; }
.mo-acct-path {
  font-size: 10.5px; color: var(--muted); font-family: var(--mono, monospace);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  padding: 5px 8px; border-radius: 6px; background: var(--bg-soft);
}
.mo-acct-actions { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 4px; }
.mo-acct-actions .mo-primary.sm { margin-left: 0; }
.mo-ghost.danger { color: var(--vermilion); }
.mo-ghost.danger:hover:not(:disabled) { border-color: var(--vermilion); color: var(--vermilion); }

.spin { animation: mo-spin 0.9s linear infinite; }
@keyframes mo-spin { to { transform: rotate(360deg); } }
</style>
