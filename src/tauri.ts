/**
 * Typed wrappers around Tauri commands.
 *
 * Designed so the renderer can still mount in a plain browser (npm run dev) by
 * detecting absence of __TAURI_INTERNALS__ and returning empty / stub data.
 */
import { invoke as rawInvoke } from "@tauri-apps/api/core";
import {
  listen as rawListen,
  emit as rawEmit,
  type UnlistenFn,
} from "@tauri-apps/api/event";

export const isTauri =
  typeof window !== "undefined" &&
  // @ts-ignore tauri injects this
  typeof (window as any).__TAURI_INTERNALS__ !== "undefined";

export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri) {
    // Browser-only stubs so the UI is still navigable during pure-web dev.
    return browserStub(cmd, args) as T;
  }
  return rawInvoke<T>(cmd, args);
}

export async function listen<T>(
  event: string,
  cb: (payload: T) => void
): Promise<UnlistenFn> {
  if (!isTauri) return () => {};
  return rawListen<T>(event, (e) => cb(e.payload));
}

export async function emit(event: string, payload?: unknown): Promise<void> {
  if (!isTauri) return;
  await rawEmit(event, payload);
}

// ──────────────────────────────────────────────────────────────
// 飞书网关 module (板块⑭ 阶段 A)
// ──────────────────────────────────────────────────────────────
export interface FeishuConfig {
  enabled: boolean;
  appId: string;
  appSecret: string;
  /** "feishu"(国内) | "lark"(国际) */
  domain: string;
  /** App 启动时自动开启网关 */
  autoStart?: boolean;
  /** "open" | "allowlist" | "disabled" */
  dmPolicy: string;
  groupRequireMention: boolean;
  allowFrom: string[];
}
export interface FeishuTestResult {
  ok: boolean;
  botName: string;
  botOpenId: string;
  message: string;
}

export interface FeishuQrResult {
  /** 二维码 SVG（本地生成，可直接内联渲染） */
  svg: string;
  /** 二维码指向的飞书开放平台建应用 URL */
  url: string;
}

export interface WecomBotInfo {
  botId: string;
  secret: string;
}

export const feishu = {
  getConfig: () => invoke<FeishuConfig>("feishu_get_config"),
  setConfig: (config: FeishuConfig) =>
    invoke<void>("feishu_set_config", { config }),
  test: () => invoke<FeishuTestResult>("feishu_test_connection"),
  /** 「扫码创建机器人」：生成飞书建应用入口二维码 */
  createQr: () => invoke<FeishuQrResult>("feishu_create_qr"),
  /** 在系统浏览器打开飞书开放平台建应用页（扫码桌面兜底） */
  openConsole: () => invoke<void>("feishu_open_console"),
  /** 企业微信扫码自动配置（OAuth 回环：开系统浏览器扫码 → 自动回传 botId/secret） */
  wecomScanCreate: (source: string) =>
    invoke<WecomBotInfo>("wecom_scan_create", { source }),
  /** 飞书对话引擎：启动长连接网关（Node 桥 → headless claude → 回发） */
  gatewayStart: () => invoke<void>("feishu_gateway_start"),
  /** 停止网关 */
  gatewayStop: () => invoke<void>("feishu_gateway_stop"),
  /** 查询网关运行状态 */
  gatewayStatus: () => invoke<{ running: boolean }>("feishu_gateway_status"),
  /** 订阅网关日志（feishu://log） */
  onGatewayLog: (cb: (text: string) => void) => listen<string>("feishu://log", cb),
  /** 订阅网关状态（feishu://status: starting|installing|connected|stopped） */
  onGatewayStatus: (cb: (state: string) => void) => listen<string>("feishu://status", cb),
};

// ──────────────────────────────────────────────────────────────
// 自媒体「账号管理」
// ──────────────────────────────────────────────────────────────
export interface MediaAccountStatus {
  platform: "wechat" | "xhs";
  label: string;
  bound: boolean;
  profileDir: string;
  /** profile 最近活动时间（unix 秒）；未绑定为 null */
  lastActive: number | null;
  detail: string;
}
export const mediaAccounts = {
  /** 探测各平台登录态（读固定 profile 目录） */
  status: () => invoke<MediaAccountStatus[]>("media_accounts_status"),
  /** 解绑某平台：清除登录态 profile，强制下次重新扫码 */
  forget: (platform: "wechat" | "xhs") =>
    invoke<string>("media_account_forget", { platform }),
};

// ──────────────────────────────────────────────────────────────
// KB module
// ──────────────────────────────────────────────────────────────
export interface KbHit {
  path: string;
  title: string;
  snippet: string;
  score: number;
}
export interface KbNode {
  id: string;
  title: string;
  category: string;
  /** "doc" 文档 | "folder" 目录中枢 | "root" 知识库根 */
  kind: "doc" | "folder" | "root";
}
export interface KbEdge {
  source: string;
  target: string;
}
export interface KbGraph {
  nodes: KbNode[];
  edges: KbEdge[];
}
/** 「构建知识网」编译进度事件 (kb:compile) */
export interface KbCompileEvent {
  runId: string;
  /** phase | tool | page | delta | done | error */
  kind: string;
  text?: string;
  /** 仅 done: 编译后重扫的文档总数 */
  docCount?: number;
}

/** 知识库拖拽上传的逐文件结果 */
export interface KbUploadResult {
  name: string;
  relPath: string;
  ok: boolean;
  message: string;
}

/** wiki 质量检查 (kb_lint) 单条问题 */
export interface KbLintIssue {
  /** dead-link | missing-type | orphan | unsafe-path */
  kind: string;
  path: string;
  detail: string;
}
/** wiki 质量检查报告 */
export interface KbLintReport {
  totalPages: number;
  deadLinks: number;
  missingType: number;
  orphans: number;
  unsafePaths: number;
  issues: KbLintIssue[];
}

/** 「维护知识网」(enrich / dedup) 进度事件 (kb:enrich / kb:dedup) */
export interface KbMaintainEvent {
  runId: string;
  /** phase | tool | delta | done | error */
  kind: string;
  text?: string;
  /** 仅 done: enrich=applied 补链数 / dedup=merged 合并数 */
  applied?: number;
  merged?: number;
}

export const kb = {
  scan: () => invoke<number>("kb_scan"),
  /** 构建知识网：跑一个有写权限的 wiki 维护者 agent，摄入即编译。返回 runId，进度走 kb:compile 事件 */
  compile: () => invoke<string>("kb_compile"),
  /** wiki 质量检查：死双链/缺 type/孤儿页/不安全路径，纯规则即时返回 */
  lint: () => invoke<KbLintReport>("kb_lint"),
  /** 自动补双链：只读 claude 出 {term,target} 建议，Rust 执行替换。返回 runId，进度走 kb:enrich */
  enrichLinks: () => invoke<string>("kb_enrich_links"),
  /** 智能去重：规则粗筛 + AI 细判 + 代码合并。返回 runId，进度走 kb:dedup */
  dedup: () => invoke<string>("kb_dedup"),
  search: (q: string, topK = 8) =>
    invoke<KbHit[]>("kb_search", { query: q, topK }),
  list: (subdir: string | null = null) =>
    invoke<string[]>("kb_list", { subdir }),
  read: (relPath: string) => invoke<string>("kb_read", { relPath }),
  /** 删除一份资料(浏览页 ×)，返回剩余文件数 */
  delete: (relPath: string) => invoke<number>("kb_delete", { relPath }),
  /** 清空资料库(管理页)，返回剩余文件数 */
  clear: () => invoke<number>("kb_clear"),
  ingest: (sourcePath: string) =>
    invoke<string>("kb_ingest", { sourcePath }),
  /** 拖拽上传：任意格式 → 转 markdown 入 raw/，返回逐文件结果 */
  uploadFiles: (paths: string[]) =>
    invoke<KbUploadResult[]>("kb_upload_files", { paths }),
  graph: () => invoke<KbGraph>("kb_graph"),
  root: () => invoke<string>("kb_root"),
  defaultRoot: () => invoke<string>("kb_default_root"),
  setRoot: (newPath: string) =>
    invoke<number>("kb_set_root", { newPath }),
};

// ──────────────────────────────────────────────────────────────
// Sandbox module → 已迁出至 features/sandbox/api.ts (架构重构 Phase 1)
// 浏览器降级 stub 仍保留在本文件下方的 browserStub() 中。
// ──────────────────────────────────────────────────────────────

// ──────────────────────────────────────────────────────────────
// Chat module
// ──────────────────────────────────────────────────────────────
export type PermissionMode =
  | "manual"
  | "auto_current"
  | "auto_all"
  | "deny";

export interface ChatSendArgs {
  prompt: string;
  permissionMode: PermissionMode;
  useSandbox?: boolean;
  skillIds?: string[];
  conversationId?: string;
  /** 目标模式：完成条件。设置后 Claude 会持续推进直到达成，不中途收尾。 */
  goal?: string;
  /** 「请教毛主席」：注入毛选式客观分析指令，调用毛主席资料库，生成标注来源的 HTML。 */
  consultMao?: boolean;
  /** 「动态编排」：多智能体编排——编排器拆 N 个独立子任务，Task 子代理并行扇出，每条流水线 实现→对抗式校验→修复，最后汇总。 */
  dynamicWorkflow?: boolean;
  /** 「知识库严格搜索」：打开时才把 KB 结构化 wiki + 双链地图注入上下文。默认 false。 */
  useKb?: boolean;
  /** 「分批长任务」：把超长生成拆成多轮有界批次（注入 polaris.build.json 清单协议）。 */
  batchBuild?: boolean;
  /** 每批最多构建几个单元（页/章/文件）。 */
  batchSize?: number;
}

export interface ChatStreamEvent {
  reqId: string;
  kind: "delta" | "tool" | "error" | "done" | "artifact" | "meta";
  text?: string;
  tool?: string;
  conversationId?: string;
}

/** 分批构建清单 polaris.build.json 的一个单元 */
export interface BuildUnit {
  id: string;
  title: string;
  status: "pending" | "done" | string;
  artifact?: string;
}

/** 分批构建清单（断点续传凭据） */
export interface BuildManifest {
  goal?: string;
  kind?: string;
  batch_size?: number;
  output?: string;
  units: BuildUnit[];
}

/** 对话拖拽上传的附件（复制进会话 uploads 目录） */
export interface AttachedFile {
  name: string;
  /** uploads 目录里的绝对路径（正斜杠） */
  path: string;
  /** text | image | pdf | office | binary */
  kind: "text" | "image" | "pdf" | "office" | "binary";
  size: number;
  ok: boolean;
  error?: string;
}

export const chat = {
  send: (args: ChatSendArgs) =>
    invoke<string>("chat_send", { args: args as unknown as Record<string, unknown> }),
  cancel: (reqId: string) => invoke<void>("chat_cancel", { reqId }),
  /** 读取分批构建清单 polaris.build.json（分批长任务的断点/进度凭据）。不存在返回 null。 */
  buildManifest: (conversationId: string | undefined) =>
    invoke<BuildManifest | null>("chat_build_manifest", {
      conversationId: conversationId ?? null,
    }),
  /** 拖拽上传：把文件复制进当前会话，返回附件清单 */
  attachFiles: (conversationId: string | undefined, paths: string[]) =>
    invoke<AttachedFile[]>("chat_attach_files", {
      conversationId: conversationId ?? null,
      paths,
    }),
};

// ──────────────────────────────────────────────────────────────
// Artifacts module — 对话生成的成品文件，右侧抽屉预览
// ──────────────────────────────────────────────────────────────
export type ArtifactKind =
  | "html"
  | "svg"
  | "image"
  | "markdown"
  | "text"
  | "binary";

export interface ArtifactPayload {
  path: string;
  name: string;
  ext: string;
  kind: ArtifactKind;
  /** 文本类(html/svg/markdown/text)内容 */
  text?: string;
  /** 图片类的 data URL */
  dataUrl?: string;
  size: number;
}

/** 「参考资料」文件夹视图的一条文件记录 */
export interface ArtifactEntry {
  path: string;
  name: string;
  ext: string;
  kind: ArtifactKind;
  size: number;
  /** 修改时间 Unix 秒 */
  modified: number;
}

export const artifacts = {
  read: (path: string) => invoke<ArtifactPayload>("artifact_read", { path }),
  openExternal: (path: string) =>
    invoke<void>("artifact_open_external", { path }),
  /** 在系统文件管理器中定位并选中该文件（资源管理器 / 访达） */
  reveal: (path: string) => invoke<void>("artifact_reveal", { path }),
  /** 列出某会话产物文件，按修改时间倒序 */
  list: (conversationId?: string) =>
    invoke<ArtifactEntry[]>("artifact_list", {
      conversationId: conversationId ?? null,
    }),
  /** 跨所有对话检索历史产物文件（文件名 + 正文） */
  search: (query: string) =>
    invoke<ArtifactSearchHit[]>("artifact_search", { query }),
};

/** 跨对话产物搜索命中 */
export interface ArtifactSearchHit {
  path: string;
  name: string;
  kind: ArtifactKind;
  conversationId: string;
  snippet: string;
  modified: number;
  score: number;
}

// ──────────────────────────────────────────────────────────────
// Project module — 可运行项目（一键启动前后端 + 内嵌预览）
// ──────────────────────────────────────────────────────────────
export interface ProjectInfo {
  /** 项目根绝对路径（正斜杠）——唯一标识 */
  root: string;
  name: string;
  /** 预览 URL（前端起来后内嵌 iframe 加载） */
  open?: string | null;
  /** 是否正在运行 */
  running: boolean;
  /** 服务名列表（展示用） */
  services: string[];
}

export const project = {
  /** 列出某会话产物里的可运行项目（带 polaris.project.json 的文件夹） */
  list: (conversationId?: string) =>
    invoke<ProjectInfo[]>("project_list", {
      conversationId: conversationId ?? null,
    }),
  /** 该项目是否正在运行 */
  status: (root: string) => invoke<boolean>("project_status", { root }),
  /** 一键运行：装依赖 + 起前后端，进度走 project:log / project:ready / project:exit 事件 */
  run: (root: string) => invoke<void>("project_run", { root }),
  /** 停止：kill 整个进程树 */
  stop: (root: string) => invoke<void>("project_stop", { root }),
};

// ──────────────────────────────────────────────────────────────
// Skills module
// ──────────────────────────────────────────────────────────────
export interface Skill {
  id: string;
  name: string;
  description: string;
  source: string;
  /** 是否已拥有可用（预装 / 已安装 / 用户自建） */
  installed?: boolean;
  /** 是否可删除（物理存在于用户目录，可卸载） */
  removable?: boolean;
}

export const skills = {
  list: () => invoke<Skill[]>("list_skills"),
  get: (id: string) => invoke<Skill>("get_skill", { id }),
  create: (id: string, name: string, description: string, systemPrompt: string) =>
    invoke<void>("create_skill", { id, name, description, systemPrompt }),
  install: (id: string) => invoke<void>("install_skill", { id }),
  /** 从外部来源导入：本地 .md/.zip/目录 · 远程 .md/.zip · git 仓库 URL（返回导入的 id 列表） */
  import: (source: string) => invoke<string[]>("import_skill", { source }),
  delete: (id: string) => invoke<void>("delete_skill", { id }),
};

// ──────────────────────────────────────────────────────────────
// CLAUDE.md 主上下文 module
// 每个 conv 项目一份 + KB 共享一份
// ──────────────────────────────────────────────────────────────
export interface ProjectClaudeMd {
  projectId: string;
  projectName: string;
  absPath: string;
  exists: boolean;
  active: boolean;
  size: number;
}

export interface KbClaudeMd {
  absPath: string;
  exists: boolean;
  active: boolean;
  size: number;
}

export type ClaudeMdArea = "kb" | "project";

export const claudeMd = {
  listProjects: () => invoke<ProjectClaudeMd[]>("claude_md_list_projects"),
  kbInfo: () => invoke<KbClaudeMd>("claude_md_kb_info"),
  read: (area: ClaudeMdArea, projectId?: string) =>
    invoke<string>("claude_md_read", { area, projectId: projectId ?? null }),
  write: (area: ClaudeMdArea, projectId: string | undefined, content: string) =>
    invoke<void>("claude_md_write", {
      area,
      projectId: projectId ?? null,
      content,
    }),
};

// ──────────────────────────────────────────────────────────────
// Conv module (项目 + 对话历史)
// ──────────────────────────────────────────────────────────────
export interface Project {
  id: string;
  name: string;
  createdAt: number;
  archived: boolean;
  /** 板块⑫ 套用的预设人格 id（自定义为 null） */
  personaId?: string | null;
  /** 该人格绑定的专属知识库 scope（KB 根下相对子目录，null/空=全局） */
  kbScope?: string | null;
}

export interface Conversation {
  id: string;
  projectId: string;
  title: string;
  createdAt: number;
  updatedAt: number;
}

export interface Message {
  id: string;
  conversationId: string;
  role: "user" | "assistant" | "tool";
  content: string;
  createdAt: number;
}

// Rust 端用 snake_case, serde 默认行为, 这里手动映射回 camelCase
type RawProject = {
  id: string;
  name: string;
  created_at: number;
  archived: boolean;
  persona_id?: string | null;
  kb_scope?: string | null;
};
type RawConv = {
  id: string;
  project_id: string;
  title: string;
  created_at: number;
  updated_at: number;
};
type RawMsg = {
  id: string;
  conversation_id: string;
  role: string;
  content: string;
  created_at: number;
};

const p = (r: RawProject): Project => ({
  id: r.id,
  name: r.name,
  createdAt: r.created_at,
  archived: r.archived,
  personaId: r.persona_id ?? null,
  kbScope: r.kb_scope ?? null,
});
const c = (r: RawConv): Conversation => ({
  id: r.id,
  projectId: r.project_id,
  title: r.title,
  createdAt: r.created_at,
  updatedAt: r.updated_at,
});
const m = (r: RawMsg): Message => ({
  id: r.id,
  conversationId: r.conversation_id,
  role: r.role as Message["role"],
  content: r.content,
  createdAt: r.created_at,
});

export const convApi = {
  listProjects: async () => (await invoke<RawProject[]>("conv_list_projects")).map(p),
  createProject: async (name: string) =>
    p(await invoke<RawProject>("conv_create_project", { name })),
  archiveProject: (projectId: string) =>
    invoke<void>("conv_archive_project", { projectId }),
  openProjectDir: (projectId: string) =>
    invoke<void>("conv_open_project_dir", { projectId }),
  listConversations: async (projectId: string) =>
    (await invoke<RawConv[]>("conv_list_conversations", { projectId })).map(c),
  createConversation: async (projectId: string) =>
    c(await invoke<RawConv>("conv_create_conversation", { projectId })),
  deleteConversation: (conversationId: string) =>
    invoke<void>("conv_delete_conversation", { conversationId }),
  renameConversation: (conversationId: string, title: string) =>
    invoke<void>("conv_rename_conversation", { conversationId, title }),
  getMessages: async (conversationId: string) =>
    (await invoke<RawMsg[]>("conv_get_messages", { conversationId })).map(m),
  /** 板块⑫: 设置项目的知识库 scope（人格工坊下拉） */
  setKbScope: (projectId: string, kbScope: string | null) =>
    invoke<void>("conv_set_project_kb_scope", { projectId, kbScope }),
};

// ──────────────────────────────────────────────────────────────
// 人格模块 module (板块⑫) — 预设人格库 + 应用到项目
// ──────────────────────────────────────────────────────────────
export interface PersonaPreset {
  id: string;
  name: string;
  icon: string;
  description: string;
  /** 建议绑定的知识库 scope（KB 根下相对子目录，空=全局） */
  kbScope: string;
  /** 人格正文（写入项目 CLAUDE.md 的内容） */
  body: string;
}

export const persona = {
  list: () => invoke<PersonaPreset[]>("persona_list"),
  /** 把预设人格应用到项目（写 CLAUDE.md + 绑定 scope）；已有内容需 overwrite=true */
  apply: (projectId: string, personaId: string, overwrite = false) =>
    invoke<void>("persona_apply", { projectId, personaId, overwrite }),
};

// ──────────────────────────────────────────────────────────────
// API 供应商坞 + 用量看板 module
// ──────────────────────────────────────────────────────────────
export interface ProviderView {
  id: string;
  name: string;
  note: string;
  baseUrl: string;
  tokenField: string;
  category: string; // official | cn_official | aggregator | third_party | cloud_provider | custom
  websiteUrl: string;
  color: string;
  kind: string; // official | key | codex | copilot | custom
  isPreset: boolean;
  hasKey: boolean;
  authToken: string;
  /** 完整 settings_config（env + includeCoAuthoredBy/attribution 等） */
  settingsConfig: any;
}
export interface ProviderListResult {
  providers: ProviderView[];
  currentId: string;
}
export interface ProviderSaveInput {
  id?: string;
  name: string;
  note?: string;
  websiteUrl?: string;
  tokenField?: string;
  /** 完整 settings_config（env 含 base_url + token + 开关） */
  settingsConfig: any;
}
export interface TokenBucket {
  input: number;
  output: number;
  cacheRead: number;
  cacheCreation: number;
  total: number;
  requests: number;
  cost: number;
}
export interface DailyUsage {
  date: string;
  label: string;
  total: number;
  cost: number;
}
export interface UsageSummary {
  available: boolean;
  today: TokenBucket;
  week: TokenBucket;
  month: TokenBucket;
  year: TokenBucket;
  daily: DailyUsage[];
}
export interface CodexStatus {
  installed: boolean;
  loggedIn: boolean;
  authPath: string;
}
export interface CodexDeviceLogin {
  deviceCode: string;
  userCode: string;
  verificationUri: string;
  interval: number;
  expiresIn: number;
}
export interface CodexPollResult {
  status: "pending" | "ok";
}
export interface CodexProxyInfo {
  running: boolean;
  port: number;
  lastError: string;
}

export const provider = {
  list: () => invoke<ProviderListResult>("provider_list"),
  switch: (id: string) => invoke<string>("provider_switch", { id }),
  save: (input: ProviderSaveInput) =>
    invoke<string>("provider_save", { input }),
  delete: (id: string) => invoke<void>("provider_delete", { id }),
  usage: () => invoke<UsageSummary>("usage_summary"),
  codexStatus: () => invoke<CodexStatus>("codex_status"),
  codexStartLogin: () => invoke<CodexDeviceLogin>("codex_start_login"),
  codexPollLogin: (deviceCode: string, userCode: string) =>
    invoke<CodexPollResult>("codex_poll_login", { deviceCode, userCode }),
  codexProxyInfo: () => invoke<CodexProxyInfo>("codex_proxy_info"),
};

// ──────────────────────────────────────────────────────────────
// 环境医生 module — 新用户「环境监测 + 配置安装」(claude / pwsh / PATH)
// ──────────────────────────────────────────────────────────────
export interface ToolStatus {
  key: "claude" | "pwsh" | "node" | "npm";
  name: string;
  found: boolean;
  version: string | null;
  path: string | null;
  onPath: boolean;
  required: boolean;
  hint: string;
}
export interface EnvReport {
  os: string;
  claude: ToolStatus;
  pwsh: ToolStatus;
  node: ToolStatus;
  npm: ToolStatus;
  claudeDir: string | null;
  claudeDirOnUserPath: boolean;
  /** 是否有 claude 可用的 shell (真身 PowerShell 7 / Git Bash)；false ⇒ 对话会报缺 shell */
  shellReady: boolean;
  ready: boolean;
}
export interface PathFixResult {
  ok: boolean;
  dir: string | null;
  status: string;
  message: string;
}
export interface EnvStreamEvent {
  reqId: string;
  kind: "log" | "error" | "done";
  line?: string;
  ok?: boolean;
  message?: string;
}
/** Claude Code 更新检测结果 */
export interface ClaudeUpdateInfo {
  installed: boolean;
  current: string | null;
  latest: string | null;
  updateAvailable: boolean;
  checked: boolean;
  message: string;
}

export const envDoctor = {
  check: () => invoke<EnvReport>("env_check"),
  fixPath: () => invoke<PathFixResult>("env_fix_path"),
  /** 安装 Claude Code。method: "npm"(经国内镜像, 默认) | "native"(官方原生脚本, 兜底) */
  installClaude: (method: "npm" | "native" = "npm") =>
    invoke<string>("env_install_claude", { method }),
  /** 安装 Node.js LTS (winget) —— npm 安装方式的前置依赖 */
  installNode: () => invoke<string>("env_install_node"),
  installPwsh: () => invoke<string>("env_install_pwsh"),
  /** 检测 Claude Code 是否有新版本 (当前版本 vs npmmirror latest) */
  checkClaudeUpdate: () => invoke<ClaudeUpdateInfo>("env_claude_update_check"),
  /** 更新 Claude Code 到最新版 (走国内 npmmirror)，流式日志同安装 */
  updateClaude: () => invoke<string>("env_update_claude"),
  cancel: (reqId: string) => invoke<void>("env_cancel", { reqId }),
};

// ──────────────────────────────────────────────────────────────
// Browser stubs (when running in plain `npm run dev` without Tauri)
// ──────────────────────────────────────────────────────────────
function browserStub(cmd: string, _args?: Record<string, unknown>): unknown {
  switch (cmd) {
    case "kb_scan":
      return 0;
    case "kb_compile":
      return "kbc-stub";
    case "kb_search":
      return [];
    case "kb_list":
      return [];
    case "kb_read":
      return "_(browser stub)_  本文件需要 Tauri 后端读取。";
    case "kb_delete":
      return 0;
    case "kb_clear":
      return 0;
    case "kb_ingest":
      return "browser-stub";
    case "kb_upload_files": {
      const paths = (_args?.paths as string[]) ?? [];
      return paths.map((p) => ({
        name: p.split(/[\\/]/).pop() || p,
        relPath: `raw/${p.split(/[\\/]/).pop() || p}`,
        ok: true,
        message: "(browser stub)",
      }));
    }
    case "chat_attach_files": {
      const paths = (_args?.paths as string[]) ?? [];
      return paths.map((p) => ({
        name: p.split(/[\\/]/).pop() || p,
        path: p,
        kind: "binary",
        size: 0,
        ok: true,
      }));
    }
    case "kb_graph":
      return { nodes: [], edges: [] };
    case "kb_root":
      return "(browser-only, no fs access)";
    case "kb_default_root":
      return "(browser-only)";
    case "kb_set_root":
      return 0;
    case "sandbox_status":
      return {
        docker_installed: false,
        docker_running: false,
        image_built: false,
        image_name: "polaris-sandbox:alpine",
        container_running: false,
        container_name: "polaris-sandbox",
        notes: ["浏览器模式 - 仅 UI 预览,无 Docker 能力"],
      };
    case "sandbox_build_image":
    case "sandbox_start":
    case "sandbox_stop":
    case "sandbox_exec":
      return "(browser stub)";
    case "cube_config_get":
      return { backend: "docker", endpoint: "", apiKey: "" };
    case "cube_config_set":
      return (_args?.config as unknown) ?? { backend: "docker", endpoint: "", apiKey: "" };
    case "cube_status":
      return {
        backend: "docker",
        endpoint: "",
        configured: false,
        reachable: false,
        note: "浏览器模式 - 无后端探测",
      };
    case "chat_send":
      return "stub-req-id";
    case "artifact_read": {
      const path = String(_args?.path ?? "demo.html");
      return {
        path,
        name: path.split("/").pop() || path,
        ext: "html",
        kind: "html",
        text:
          "<!doctype html><html><body style='font-family:sans-serif;padding:40px;text-align:center'><h1>预览占位</h1><p>浏览器模式无后端，无法读取真实文件。</p></body></html>",
        size: 0,
      };
    }
    case "artifact_open_external":
      return undefined;
    case "artifact_list":
      return [];
    case "artifact_search":
      return [];
    case "project_list":
      return [];
    case "project_status":
      return false;
    case "project_run":
    case "project_stop":
      return undefined;
    case "list_skills":
      return [
        { id: "deep-research", name: "深度搜索", description: "使用 LLM 大规模联网搜索相关内容，自动检索、汇总、交叉验证多来源信息", source: "third-party", installed: true, removable: false },
        { id: "skill-creator", name: "Skill 创建向导", description: "引导用户创建自定义 Skill，自动生成模板和配置文件", source: "official", installed: true, removable: false },
        { id: "pdf", name: "PDF 文档处理", description: "提取 / 生成 / 编辑 PDF：抽取文本表格、合并拆分、Markdown 转 PDF、表单与 OCR", source: "official", installed: false, removable: false },
        { id: "xlsx", name: "Excel 表格", description: "读取分析与生成 Excel：透视统计、公式、图表、多 sheet 报表", source: "official", installed: false, removable: false },
        { id: "pptx", name: "PPT 演示文稿", description: "把 PDF / 文档 / 数据转成有高级感的 PPT：母版配色、版式层级、图表，python-pptx 生成", source: "official", installed: false, removable: false },
        { id: "edge-tts", name: "语音合成 Edge-TTS", description: "把文本转成自然语音音频，多语言多音色，免费无需 key", source: "third-party", installed: false, removable: false },
        { id: "hyperframes", name: "视频动画 Hyperframes", description: "用逐帧 / 分镜方式生成短视频与动画，ffmpeg 合成，可配 Edge-TTS 旁白", source: "third-party", installed: false, removable: false },
        { id: "web-search", name: "联网搜索", description: "实时联网检索，基于 Tavily / Brave 等真实来源回答并交叉验证", source: "third-party", installed: false, removable: false },
        { id: "image-gen", name: "AI 生图 gpt-image-2", description: "用 OpenAI gpt-image-2 模型按描述生成图片，自动扩写提示词，支持多候选与改图", source: "third-party", installed: false, removable: false },
        { id: "cloak-browser", name: "CloakBrowser 浏览器", description: "Agent 默认浏览器：源码级隐身 Chromium，drop-in 替换 Playwright，过 Cloudflare / 反爬。可随时关闭移除", source: "third-party", installed: true, removable: false },
      ];
    case "get_skill":
      return { id: "deep-research", name: "深度搜索", description: "使用 LLM 大规模联网搜索相关内容", source: "third-party", installed: true, removable: false };
    case "import_skill":
      return ["browser-stub-skill"];
    case "create_skill":
    case "install_skill":
    case "delete_skill":
      return undefined;
    case "conv_list_projects":
      return [
        {
          id: "p-stub",
          name: "(浏览器) 示例项目",
          created_at: 0,
          archived: false,
        },
      ];
    case "conv_create_project":
      return {
        id: "p-stub-new",
        name: (_args?.name as string) || "新项目",
        created_at: 0,
        archived: false,
      };
    case "conv_list_conversations":
      return [];
    case "conv_create_conversation":
      return {
        id: "c-stub-new",
        project_id: _args?.projectId as string,
        title: "新对话",
        created_at: 0,
        updated_at: 0,
      };
    case "conv_get_messages":
      return [];
    case "conv_archive_project":
    case "conv_open_project_dir":
    case "conv_delete_conversation":
    case "conv_rename_conversation":
      return undefined;
    case "claude_md_list_projects":
      return [];
    case "claude_md_kb_info":
      return {
        absPath: "(browser-only)",
        exists: false,
        active: false,
        size: 0,
      };
    case "claude_md_read":
      return "_(browser stub)_  本文件需要 Tauri 后端读取。";
    case "claude_md_write":
      return undefined;
    case "conv_set_project_kb_scope":
    case "persona_apply":
      return undefined;
    case "feishu_get_config":
      return {
        enabled: false,
        appId: "",
        appSecret: "",
        autoStart: false,
        domain: "feishu",
        dmPolicy: "open",
        groupRequireMention: true,
        allowFrom: [],
      };
    case "feishu_set_config":
      return undefined;
    case "feishu_test_connection":
      return {
        ok: false,
        botName: "",
        botOpenId: "",
        message: "浏览器模式无法连接飞书，请在桌面应用中测试。",
      };
    case "feishu_create_qr":
      return {
        svg: "<svg xmlns='http://www.w3.org/2000/svg' width='240' height='240'><rect width='240' height='240' fill='#fff'/><text x='120' y='124' font-size='12' fill='#999' text-anchor='middle'>浏览器模式无二维码</text></svg>",
        url: "https://open.feishu.cn/app",
      };
    case "feishu_open_console":
      return undefined;
    case "wecom_scan_create":
      throw new Error("浏览器模式无法扫码创建，请在桌面应用中操作。");
    case "feishu_gateway_status":
      return { running: false };
    case "feishu_gateway_start":
      throw new Error("浏览器模式无法启动网关，请在桌面应用中操作。");
    case "feishu_gateway_stop":
      return undefined;
    case "persona_list":
      return [
        { id: "stock-expert", name: "股票助手", icon: "📈", description: "A 股深度分析 / 公告监控 / 行情查询。", kbScope: "raw/股票", body: "(browser stub)" },
        { id: "content-writer", name: "内容创作", icon: "✍️", description: "公众号/自媒体写手：选题、撰写、5 种风格。", kbScope: "raw/创作", body: "(browser stub)" },
        { id: "lesson-planner", name: "备课出卷", icon: "📚", description: "K12 教案/试卷/答案解析。", kbScope: "raw/教学", body: "(browser stub)" },
        { id: "content-summarizer", name: "内容总结", icon: "📋", description: "网页/文档/会议纪要结构化摘要。", kbScope: "", body: "(browser stub)" },
        { id: "health-interpreter", name: "医疗健康解读", icon: "🏥", description: "体检报告/化验单通俗解读。", kbScope: "raw/健康", body: "(browser stub)" },
        { id: "pet-care", name: "萌宠管家", icon: "🐾", description: "猫狗行为/健康/营养。", kbScope: "raw/萌宠", body: "(browser stub)" },
        { id: "mao", name: "毛主席", icon: "☭", description: "毛选式客观分析。", kbScope: "raw/毛主席", body: "(browser stub)" },
      ];
    case "provider_list": {
      const mk = (id: string, name: string, baseUrl: string, category: string, color: string, kind: string, hasKey: boolean, authToken = "") => ({
        id, name, note: "", baseUrl, tokenField: "ANTHROPIC_AUTH_TOKEN", category, websiteUrl: baseUrl, color, kind, isPreset: true, hasKey, authToken,
        settingsConfig: { env: baseUrl ? { ANTHROPIC_BASE_URL: baseUrl, ...(authToken ? { ANTHROPIC_AUTH_TOKEN: authToken } : {}) } : {} },
      });
      return {
        providers: [
          mk("claude-official", "Claude 官方", "", "official", "#D97757", "official", true),
          mk("zhipu-glm", "智谱 GLM", "https://open.bigmodel.cn/api/anthropic", "cn_official", "#2c6fff", "key", false),
          mk("kimi", "Kimi 月之暗面", "https://api.moonshot.cn/anthropic", "cn_official", "#2c6fff", "key", true, "sk-demo"),
          mk("deepseek", "DeepSeek 深度求索", "https://api.deepseek.com/anthropic", "cn_official", "#2c6fff", "key", false),
          mk("openrouter", "OpenRouter", "https://openrouter.ai/api", "aggregator", "#7c5cff", "key", false),
          mk("aihubmix", "AiHubMix", "https://aihubmix.com", "aggregator", "#7c5cff", "key", false),
          mk("packycode", "PackyCode", "https://www.packyapi.com", "third_party", "#e8833a", "key", false),
          mk("github-copilot", "GitHub Copilot", "https://api.githubcopilot.com", "third_party", "#e8833a", "copilot", false),
          mk("codex", "Codex (ChatGPT)", "https://chatgpt.com/backend-api/codex", "third_party", "#e8833a", "codex", false),
        ],
        currentId: "kimi",
      };
    }
    case "provider_switch":
      return String(_args?.id ?? "claude-official");
    case "provider_save":
      return "custom-stub";
    case "provider_delete":
      return undefined;
    case "codex_status":
      return { installed: false, loggedIn: false, authPath: "(browser-only)" };
    case "codex_start_login":
      return {
        deviceCode: "stub-device",
        userCode: "WXYZ-1234",
        verificationUri: "https://auth.openai.com/codex/device",
        interval: 5,
        expiresIn: 900,
      };
    case "codex_poll_login":
      return { status: "ok" };
    case "codex_proxy_info":
      return { running: false, port: 0, lastError: "" };
    case "env_check": {
      const tool = (key: string, name: string, found: boolean, required = false): ToolStatus => ({
        key: key as ToolStatus["key"],
        name,
        found,
        version: found ? "(browser stub) v0.0.0" : null,
        path: found ? `/usr/local/bin/${key}` : null,
        onPath: found,
        required,
        hint: found ? "(browser stub) 已安装" : "未安装 —— 浏览器预览无法真实检测",
      });
      return {
        os: "browser",
        claude: tool("claude", "Claude Code", false, true),
        pwsh: tool("pwsh", "PowerShell 7", false),
        node: tool("node", "Node.js", true),
        npm: tool("npm", "npm", true),
        claudeDir: null,
        claudeDirOnUserPath: true,
        shellReady: false,
        ready: false,
      };
    }
    case "env_fix_path":
      return {
        ok: false,
        dir: null,
        status: "skipped",
        message: "浏览器预览模式无法修改环境变量。",
      };
    case "env_install_claude":
    case "env_install_node":
    case "env_install_pwsh":
    case "env_update_claude":
      return "env-stub-req";
    case "env_claude_update_check":
      return {
        installed: true,
        current: "1.0.0",
        latest: "1.0.1",
        updateAvailable: true,
        checked: true,
        message: "(browser stub) 发现新版本 1.0.1 (当前 1.0.0)。",
      };
    case "env_cancel":
      return undefined;
    case "usage_summary": {
      const daily = Array.from({ length: 14 }, (_, i) => {
        const d = new Date(Date.now() - (13 - i) * 86400000);
        const label = `${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
        return { date: label, label, total: Math.round(300000 + Math.random() * 1600000), cost: +(Math.random() * 6).toFixed(4) };
      });
      return {
        available: true,
        today: { input: 75600, output: 644800, cacheRead: 45506800, cacheCreation: 1637200, total: 720483 + 47144001, requests: 411, cost: 49.107 },
        week: { input: 280000, output: 64000, cacheRead: 6100000, cacheCreation: 410000, total: 6854000, requests: 248, cost: 112.4 },
        month: { input: 980000, output: 240000, cacheRead: 22000000, cacheCreation: 1400000, total: 24620000, requests: 940, cost: 421.8 },
        year: { input: 1900000, output: 520000, cacheRead: 44000000, cacheCreation: 2800000, total: 49220000, requests: 1894, cost: 980.5 },
        daily,
      };
    }
    default:
      return null;
  }
}
