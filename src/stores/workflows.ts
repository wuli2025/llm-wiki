import { defineStore } from "pinia";
import { ref } from "vue";

/**
 * 工作流包（Workflow Pack）
 * ──────────────────────────────────────────────────────────────
 * 一个「结构化提示词」：由若干有序「环节(step)」编排而成。
 * 每个环节是一段带标题的提示词正文，调整环节顺序 / 增删即可重新编排，
 * 适合那些需要不断改变编排方式的任务。点「使用」会把所有环节拼装成
 * 一段完整提示词，填入对话输入框。本地持久化（localStorage）。
 */
export interface WorkflowStep {
  id: string;
  label: string; // 环节标题，如「角色」「任务」「约束」「输出格式」
  content: string; // 该环节的提示词正文
}

export interface WorkflowPack {
  id: string;
  name: string;
  description: string;
  color: string; // 强调色（取自 PACK_COLORS）
  steps: WorkflowStep[];
  createdAt: number;
  updatedAt: number;
}

/** 工作流编辑器提交的数据（id 为空 = 新建） */
export interface WorkflowDraft {
  id?: string;
  name: string;
  description: string;
  color: string;
  steps: WorkflowStep[];
}

const STORAGE_KEY = "polaris:workflow-packs:v1";
const SEED_KEY = "polaris:workflow-packs-seeded:v1";
// v2 增补的「写作 / 技术选型三件套」：对老用户也补一次，删除后不回种
const SEED_V2_KEY = "polaris:workflow-packs-seeded:v2";
// v3 增补「网页演示视频成片 / Claude Code Harness 工程实践」两套（源自 ConardLi 教程）
const SEED_V3_KEY = "polaris:workflow-packs-seeded:v3";

/** 墨蓝水墨主题取色 —— 每个包一抹强调色 */
export const PACK_COLORS = [
  "#2c4661", // 墨蓝
  "#a78c4f", // 金
  "#c0392b", // 朱
  "#3f6b5a", // 松绿
  "#6b4f7a", // 紫
  "#9c6b3f", // 赭石
];

function uid(): string {
  return Date.now().toString(36) + Math.random().toString(36).slice(2, 8);
}

export function newStep(label = "", content = ""): WorkflowStep {
  return { id: uid(), label, content };
}

/** 把一个工作流包的环节拼装成最终提示词 */
export function assemblePack(p: { steps: WorkflowStep[] }): string {
  return p.steps
    .map((s) => {
      const body = s.content.trim();
      if (!body) return "";
      const label = s.label.trim();
      return label ? `【${label}】\n${body}` : body;
    })
    .filter(Boolean)
    .join("\n\n");
}

/** 首启种入的示例包（只种一次） */
function seedPacks(): WorkflowPack[] {
  const now = Date.now();
  return [
    {
      id: uid(),
      name: "深度调研报告",
      description: "围绕一个主题做多源检索，交叉验证后产出结构化报告",
      color: PACK_COLORS[0],
      createdAt: now,
      updatedAt: now,
      steps: [
        newStep("角色", "你是一名严谨的研究分析师，擅长多源检索与交叉验证。"),
        newStep(
          "任务",
          "围绕主题「__________」展开调研：检索权威来源，提炼关键事实，标注分歧与不确定处。"
        ),
        newStep(
          "方法",
          "1. 列出 3–5 个核心子问题\n2. 逐个检索并记录来源\n3. 对冲突信息交叉验证\n4. 汇总成结论"
        ),
        newStep(
          "输出格式",
          "Markdown 报告：摘要 → 关键发现（带来源）→ 风险/分歧 → 结论与建议。"
        ),
      ],
    },
    {
      id: uid(),
      name: "代码审查",
      description: "对一段代码做高标准审查，聚焦正确性、安全与可维护性",
      color: PACK_COLORS[3],
      createdAt: now,
      updatedAt: now,
      steps: [
        newStep("角色", "你是一位资深工程师，做代码审查时直接、具体、不客套。"),
        newStep(
          "审查重点",
          "正确性与边界条件 · 安全隐患 · 性能 · 命名与可读性 · 是否与现有风格一致。"
        ),
        newStep(
          "输出",
          "按「严重 / 建议 / 小提示」分级，每条给出文件:行号与可操作的修改方案。"
        ),
      ],
    },
  ];
}

/**
 * v2 增补：三套「全流程」工作流包 —— 小红书、公众号、技术选型。
 * 每套环节都细化到可直接照着产出，并都内置「评审 / 自检」环节
 * （即用户说的「怎么评审这个文章」）。点「使用」即把整套拼成一段提示词。
 */
function seedPacksV2(): WorkflowPack[] {
  const now = Date.now();
  return [
    {
      id: uid(),
      name: "小红书爆款笔记 · 全流程",
      description:
        "从选题、起标题、写正文到排版与爆款自检，一条龙产出一篇小红书笔记",
      color: PACK_COLORS[2],
      createdAt: now,
      updatedAt: now,
      steps: [
        newStep(
          "角色",
          "你是一名深耕小红书的资深内容操盘手，熟悉平台调性、流量逻辑与女性向/生活方式表达。语气真实、有温度、像朋友分享，不端着、不说教。"
        ),
        newStep(
          "主题与人设",
          "本篇主题：「__________」。\n目标人群：__________（年龄/身份/痛点）。\n账号人设：__________（如「踩坑无数的打工人」「精致省钱博主」）。\n先用一句话说清：读者看完能获得什么、为什么要点进来。"
        ),
        newStep(
          "选题与卖点",
          "围绕主题给出 3 个候选切入角度（痛点型 / 教程型 / 反差种草型各一），每个标注适合的人群与预期互动点。\n挑出最优角度，提炼 1 个核心卖点 + 2 个支撑亮点，避免大而全。"
        ),
        newStep(
          "标题（钩子）",
          "产出 5 个备选标题，控制在 20 字内，至少覆盖这几类钩子：\n1) 数字/清单（如「3个」「7天」）\n2) 痛点共鸣（「别再…」「我后悔…」）\n3) 反差/悬念（「没想到…」「居然…」）\n4) 身份代入（「打工人必看」）\n善用 1–2 个 emoji 提升点击，不堆砌。标注推荐用哪个及理由。"
        ),
        newStep(
          "正文结构",
          "按「开头钩子 → 干货主体 → 收尾引导」写正文，控制在 300–600 字：\n· 开头 2 行制造停留（场景代入 / 戳痛点 / 抛结论）。\n· 主体分点，每点一个小标题 + emoji，短句、口语化、多换行，拒绝大段文字。\n· 适当加入个人真实体验/数据增强可信度。\n· 结尾引导互动（提问 / 求点赞收藏 / 预告下篇）。"
        ),
        newStep(
          "排版与标签",
          "输出最终可直接粘贴的版式：合理分段、每段前加点睛 emoji，关键句用「」或换行突出。\n末尾给 8–12 个话题标签（#），覆盖大词 + 精准长尾词 + 当下热点词，按相关度排序。"
        ),
        newStep(
          "爆款自检评审",
          "以平台流量视角给这篇笔记打分（满分10）并逐项点评：\n1) 标题点击欲是否够强？\n2) 前三行能否留住人（完播/停留）？\n3) 是否有明确价值点、读完有收获感？\n4) 是否引导了点赞收藏评论（互动率）？\n5) 标签是否精准、有无蹭到流量词？\n对每条给出「问题 + 具体改法」，并产出修订后的最终版。"
        ),
      ],
    },
    {
      id: uid(),
      name: "微信公众号长文 · 全流程",
      description:
        "选题—框架—成文—配图建议—评审润色，产出一篇结构完整的公众号推文",
      color: PACK_COLORS[0],
      createdAt: now,
      updatedAt: now,
      steps: [
        newStep(
          "角色",
          "你是一名资深公众号主编，擅长把复杂话题写成有深度又好读的长文。文风沉稳、有观点、逻辑清晰，兼顾信息密度与阅读节奏。"
        ),
        newStep(
          "选题与受众",
          "本篇选题：「__________」。\n目标读者：__________（他们关心什么、认知水平如何）。\n写作目的：__________（科普 / 观点输出 / 带货 / 涨粉）。\n用一句话写出本文的核心论点或独特价值，确保区别于同题材的泛泛之作。"
        ),
        newStep(
          "标题与开头",
          "产出 5 个备选标题（主标题，可含副标题），兼顾打开率与调性，避免标题党踩雷词。\n再写 2 版开头（150 字内）：一版用故事/场景切入，一版用问题/数据切入，目的都是 5 秒内让读者决定继续读。标注推荐组合。"
        ),
        newStep(
          "文章框架",
          "先给出整篇大纲（一级/二级小标题 + 每节要点），形成「提出问题 → 分析展开 → 给出方案/结论 → 升华或行动号召」的主线。\n确认每一节都为核心论点服务，删掉与主线无关的枝节。"
        ),
        newStep(
          "成文写作",
          "按大纲展开成文，目标 1500–3000 字：\n· 小标题清晰、每节 1 个中心意思，段落短、善用过渡句承上启下。\n· 观点要有论据（数据 / 案例 / 引用），避免空话。\n· 节奏上长短句交错，适当用金句、设问、列表增强可读性。\n· 在合适位置标注【配图建议】：说明此处应放什么图/图示及作用。"
        ),
        newStep(
          "评审与润色",
          "以主编视角终审并给出修改：\n1) 标题与开头的打开率是否够强？\n2) 逻辑是否顺、有无断层或重复？\n3) 论点是否有足够支撑、有无硬伤？\n4) 阅读节奏与排版（小标题、分段、重点）是否友好？\n5) 结尾是否有记忆点/行动号召？\n逐条给「问题→改法」，并产出润色后的终稿与一句话摘要（用于推送摘要栏）。"
        ),
      ],
    },
    {
      id: uid(),
      name: "联网搜索 · 技术选型全流程",
      description:
        "澄清需求—检索候选—多维对比—交叉验证—给出推荐与决策记录",
      color: PACK_COLORS[3],
      createdAt: now,
      updatedAt: now,
      steps: [
        newStep(
          "角色",
          "你是一名资深技术架构师，做技术选型时实事求是、用证据说话，既懂工程权衡也懂业务约束。善用联网检索获取一手、最新的资料并交叉验证。"
        ),
        newStep(
          "需求与约束澄清",
          "选型目标：「__________」（要解决的具体问题/场景）。\n先把约束讲清：团队技术栈与人力、性能/规模要求、预算与成本、上线时间、合规/许可证要求、长期维护诉求。\n列出本次选型最看重的 3–5 个决策维度并排定优先级。"
        ),
        newStep(
          "候选方案检索（联网）",
          "联网检索该领域当前主流的 3–5 个候选方案/技术/库。\n对每个候选记录：官方文档/仓库链接、最新稳定版本与发布时间、社区活跃度（star/issue/更新频率）、采用它的代表性项目或公司。务必标注信息来源与时间。"
        ),
        newStep(
          "多维对比",
          "按上一步定的决策维度，做一张对比表（候选 × 维度），常见维度：功能完备度、性能、生态与集成、学习曲线、文档质量、社区/商业支持、许可证、运维成本、可扩展性、长期风险。\n每格给简短结论，关键处附来源。"
        ),
        newStep(
          "交叉验证与风险",
          "对关键结论做交叉验证：是否有反例、踩坑帖、性能基准或迁移成本的实证？区分「事实」与「网络上的主观评价」。\n列出每个候选的主要风险与不确定项，以及缓解办法。"
        ),
        newStep(
          "结论与决策记录",
          "给出明确推荐（首选 + 备选），并说明为什么——对照决策维度逐条解释取舍。\n输出一份精简的「技术选型决策记录(ADR)」：背景 → 候选 → 决策 → 理由 → 影响与后续验证计划。最后附上引用来源清单。"
        ),
      ],
    },
  ];
}

/**
 * v3 增补：两套源自 ConardLi 教程的「全流程」工作流包。
 * ① 网页演示视频成片：文稿 → 16:9 网页演示 → 点击翻页 →（可选 MiniMax 配音）→ 录屏成片，
 *    这就是「视频效果到底怎么做的」那一半 —— 用 web-video-presentation 思路把稿子做成可录屏的网页。
 * ② Claude Code Harness 工程实践：把 Claude Code 当成生产力 harness 来调教 ——
 *    技能化沉淀、CC Switch / 供应商坞按任务切模型控成本、MiniMax CLI 接入、子代理编排与复盘。
 * 相关资源：garden-skills(https://github.com/ConardLi/garden-skills)、
 *           Claude Code 文档、MiniMax CLI、CC Switch。
 */
function seedPacksV3(): WorkflowPack[] {
  const now = Date.now();
  return [
    {
      id: uid(),
      name: "网页演示视频 · 文稿到成片",
      description:
        "把文稿/文章做成 16:9 可点击翻页的网页演示，逐章打磨视觉，再录屏成「视频感」成片（可选配音）",
      color: PACK_COLORS[4],
      createdAt: now,
      updatedAt: now,
      steps: [
        newStep(
          "角色",
          "你既是视频脚本作者，也是讲究审美的前端工程师。目标：把一段文稿/文章变成一套 16:9（1920×1080 固定舞台）、点击逐页推进、像视频一样的网页演示，最终用屏幕录制导出成片。参考 ConardLi 的 web-video-presentation 思路：内容先行、逐章打磨、反 AI 廉价感。"
        ),
        newStep(
          "输入素材",
          "本片要讲的内容：「__________」（贴上你的文稿/文章，或一句话主题）。\n先确认形态：是产品演示 / 技术分享 / 课程讲解 / 观点输出？目标时长几分钟？给谁看？\n如素材只是主题，请先补一版口播脚本再继续。"
        ),
        newStep(
          "脚本与大纲（一次产出 + 自检）",
          "一次性产出两份：\n· script.md —— 逐章口播稿（每章一个核心信息，口语、能直接念，控制单页信息量）。\n· outline.md —— 开发大纲（章节切分、每章视觉意图、动画/图示点、需要的素材清单）。\n产出后对照自检：每页是否只讲一件事？开场 5 秒有没有钩子？有没有大段文字堆在一屏？不达标先改稿，别急着写代码。"
        ),
        newStep(
          "5 项对齐（硬检查点，别跳过）",
          "在动手写代码前，和我逐项确认这 5 件事并定稿：\n1) 脚本 script.md 是否 OK；\n2) 大纲 outline.md 是否 OK；\n3) 主题/视觉风格（配色、字体、参考的设计调性）；\n4) 素材（要用到的图片/logo/数据/截图，从哪来）；\n5) 开发模式：A 逐章给我验收 / B 一口气做完统一审 / C 多子代理并行开发。\n这一步没对齐就不要往下做。"
        ),
        newStep(
          "搭骨架 + 先做完第 1 章",
          "用 Vite + React + TypeScript 搭一个固定 1920×1080 舞台、用一个 stepper（带持久化的翻页状态）控制点击推进的演示工程。\n先只把第 1 章做「完整、好看、定调」：它就是后面所有章节的视觉锚点和风格基准。做完让我先验收第 1 章的视觉、节奏、信息密度，通过了再继续。"
        ),
        newStep(
          "逐章开发（按选定模式）",
          "按对齐时选的模式开发第 2~N 章。每章遵守：一页一个核心信息、留白充足、动画为信息服务（不是炫技）、文字少而大（投影/录屏都看得清）、配色与第 1 章一致。\n每做完一章做一次反「AI 廉价感」自检：是否像随手生成的模板？是否有真实层次与细节？不像成片就重做该章。"
        ),
        newStep(
          "配音（可选，MiniMax CLI）",
          "若要带旁白：从各章抽出口播文本，逐段用 TTS 合成音频（首选 MiniMax CLI，其次 OpenAI/edge-tts）。\n配音模式下页面可支持自动播放（按音频时长自动翻页）；不配音就走手动点击、后期再配音轨。\n注意：MiniMax 需要 API key，没有就先跳过配音、只做手动翻页版。"
        ),
        newStep(
          "录屏成片 + 自检",
          "用屏幕录制软件按 16:9 全屏录制：自动模式让它自己播完，手动模式跟着脚本节奏点击翻页。\n成片自检：分辨率/比例对不对？有没有卡顿或错帧？节奏跟口播搭不搭？字幕/旁白齐不齐？\n最后说明产出了哪些文件（演示工程目录、脚本、录屏），以及还需我做什么（如手动录屏、配 key）。"
        ),
      ],
    },
    {
      id: uid(),
      name: "Claude Code Harness · 工程实践",
      description:
        "把 Claude Code 当生产力 harness 调教：技能化沉淀 + 供应商按任务切换(CC Switch) + MiniMax CLI 接入 + 子代理编排与复盘",
      color: PACK_COLORS[5],
      createdAt: now,
      updatedAt: now,
      steps: [
        newStep(
          "角色",
          "你是「Agent Harness 工程师」。harness = 把模型包成可靠生产力的那层工程：提示词/技能、工具与权限、供应商与模型选择、编排与上下文管理。目标是帮我把 Claude Code 从「能聊」调教成「能稳定交付」。实事求是，给可执行的具体动作，不堆方法论。"
        ),
        newStep(
          "现状盘点",
          "先盘清我现在的用法：\n· 常做哪些重复任务（写稿/查资料/做演示/写代码…）？\n· 现在靠手敲提示词还是已有沉淀？\n· 用哪个供应商/模型、有没有成本压力？\n· 有没有用到技能(skill)、工作流、子代理？\n据此判断 harness 的最大瓶颈在哪（提示词没沉淀 / 模型选得不对 / 缺工具 / 没编排）。"
        ),
        newStep(
          "技能化沉淀（skill）",
          "把高频、可复用的提示词沉淀成 skill（一段可被注入的系统提示词 + 配套参考），而不是每次重敲。\n参考 ConardLi 的 garden-skills（https://github.com/ConardLi/garden-skills）：web-video-presentation（做演示视频）、web-design-engineer（前端审美）、gpt-image-2（生图）、kb-retriever（本地知识库检索）。\n给我列出：哪些任务值得做成 skill、每个 skill 的边界与触发场景、先做哪一个。"
        ),
        newStep(
          "供应商按任务切换（CC Switch / 供应商坞）",
          "harness 的关键一招是「按任务选模型」：贵任务用强模型、量大/省钱任务切便宜模型或国产供应商。\n工具：CC Switch（https://github.com/farion1231/cc-switch）一键切 Claude Code 的供应商配置；本 App 的「供应商坞」同理（切换写 ~/.claude/settings.json 的 env）。\n给我一套切换策略：哪类任务用哪个供应商/模型、怎么快速切、怎么避免切错环境。"
        ),
        newStep(
          "MiniMax CLI 接入",
          "MiniMax CLI（https://github.com/MiniMax-AI/cli）：可作为低成本推理通道，也常用于语音合成（给演示视频配音）。\n给我接入步骤：装什么、配哪个 key（认 api.minimaxi.com）、怎么在配音/批量任务里调用，以及和主供应商怎么分工。"
        ),
        newStep(
          "编排与子代理",
          "对复杂任务用编排而非一段长提示词：拆成子任务、用子代理并行扇出、每条流水线「实现→对抗式校验→修复→汇总」。\n参考 Claude Code 文档（https://code.claude.com/docs/zh-CN/overview）的 subagent / 工具用法。\n针对我盘点出的一个真实复杂任务，给一份具体的编排方案（拆几步、谁验谁、怎么收口）。"
        ),
        newStep(
          "沉淀与复盘",
          "把这次定下的东西落地成可复用资产：哪些进 skill、哪些进工作流包、哪些进自动化流、供应商切换策略记到哪。\n给我一份「harness 升级清单」：已做 / 待做 / 优先级，以及一个一句话复盘——这次最大的提效点是什么。"
        ),
      ],
    },
  ];
}

export const useWorkflowsStore = defineStore("workflows", () => {
  const packs = ref<WorkflowPack[]>([]);

  // 编辑器（新建 / 修改共用一个模态）
  const editorOpen = ref(false);
  const editorTarget = ref<WorkflowPack | null>(null); // null = 新建

  // 「使用」→ 把拼装文本送进对话输入框；带 nonce 以便重复使用同一包也能触发
  const insertRequest = ref<{ text: string; n: number } | null>(null);
  let insertSeq = 0;

  function load() {
    try {
      const raw = localStorage.getItem(STORAGE_KEY);
      if (raw) packs.value = JSON.parse(raw) as WorkflowPack[];
    } catch {
      /* ignore，落到种入分支 */
    }
    // 首启：种入示例包一次（用户删光后不再回种）
    if (packs.value.length === 0 && !localStorage.getItem(SEED_KEY)) {
      packs.value = seedPacks();
      localStorage.setItem(SEED_KEY, "1");
      persist();
    }
    // v2 增补「写作/技术选型三件套」：对老用户也补一次，删除后不回种
    if (!localStorage.getItem(SEED_V2_KEY)) {
      packs.value = [...seedPacksV2(), ...packs.value];
      localStorage.setItem(SEED_V2_KEY, "1");
      persist();
    }
    // v3 增补「网页演示视频 / Harness 工程实践」两套：对老用户也补一次，删除后不回种
    if (!localStorage.getItem(SEED_V3_KEY)) {
      packs.value = [...seedPacksV3(), ...packs.value];
      localStorage.setItem(SEED_V3_KEY, "1");
      persist();
    }
  }

  function persist() {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(packs.value));
  }

  function openCreate() {
    editorTarget.value = null;
    editorOpen.value = true;
  }
  function openEdit(p: WorkflowPack) {
    editorTarget.value = p;
    editorOpen.value = true;
  }
  function closeEditor() {
    editorOpen.value = false;
    editorTarget.value = null;
  }

  /** 新建或更新一个包 */
  function savePack(draft: WorkflowDraft) {
    const now = Date.now();
    const steps = draft.steps.filter(
      (s) => s.label.trim() || s.content.trim()
    );
    if (draft.id) {
      const i = packs.value.findIndex((p) => p.id === draft.id);
      if (i >= 0) {
        packs.value[i] = {
          ...packs.value[i],
          name: draft.name.trim(),
          description: draft.description.trim(),
          color: draft.color,
          steps,
          updatedAt: now,
        };
        packs.value = [...packs.value];
      }
    } else {
      packs.value = [
        {
          id: uid(),
          name: draft.name.trim(),
          description: draft.description.trim(),
          color: draft.color,
          steps,
          createdAt: now,
          updatedAt: now,
        },
        ...packs.value,
      ];
    }
    persist();
  }

  function removePack(id: string) {
    packs.value = packs.value.filter((p) => p.id !== id);
    persist();
  }

  /** 点「使用」：拼装并请求填入对话框 */
  function usePack(p: WorkflowPack) {
    insertRequest.value = { text: assemblePack(p), n: ++insertSeq };
  }

  function clearInsert() {
    insertRequest.value = null;
  }

  load();

  return {
    packs,
    editorOpen,
    editorTarget,
    insertRequest,
    openCreate,
    openEdit,
    closeEditor,
    savePack,
    removePack,
    usePack,
    clearInsert,
  };
});
