/**
 * Typed wrappers around Tauri commands.
 *
 * Designed so the renderer can still mount in a plain browser (npm run dev) by
 * detecting absence of __TAURI_INTERNALS__ and returning empty / stub data.
 */
import { invoke as rawInvoke } from "@tauri-apps/api/core";
import { listen as rawListen, type UnlistenFn } from "@tauri-apps/api/event";

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
}
export interface KbEdge {
  source: string;
  target: string;
}
export interface KbGraph {
  nodes: KbNode[];
  edges: KbEdge[];
}

export const kb = {
  scan: () => invoke<number>("kb_scan"),
  search: (q: string, topK = 8) =>
    invoke<KbHit[]>("kb_search", { query: q, topK }),
  list: (subdir: string | null = null) =>
    invoke<string[]>("kb_list", { subdir }),
  read: (relPath: string) => invoke<string>("kb_read", { relPath }),
  ingest: (sourcePath: string) =>
    invoke<string>("kb_ingest", { sourcePath }),
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
  skillId?: string;
  conversationId?: string;
}

export interface ChatStreamEvent {
  reqId: string;
  kind: "delta" | "tool" | "error" | "done";
  text?: string;
  tool?: string;
  conversationId?: string;
}

export const chat = {
  send: (args: ChatSendArgs) =>
    invoke<string>("chat_send", { args: args as unknown as Record<string, unknown> }),
  cancel: (reqId: string) => invoke<void>("chat_cancel", { reqId }),
};

// ──────────────────────────────────────────────────────────────
// Skills module
// ──────────────────────────────────────────────────────────────
export interface Skill {
  id: string;
  name: string;
  description: string;
  source: string;
}

export const skills = {
  list: () => invoke<Skill[]>("list_skills"),
  get: (id: string) => invoke<Skill>("get_skill", { id }),
  create: (id: string, name: string, description: string, systemPrompt: string) =>
    invoke<void>("create_skill", { id, name, description, systemPrompt }),
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
};

// ──────────────────────────────────────────────────────────────
// Browser stubs (when running in plain `npm run dev` without Tauri)
// ──────────────────────────────────────────────────────────────
function browserStub(cmd: string, _args?: Record<string, unknown>): unknown {
  switch (cmd) {
    case "kb_scan":
      return 0;
    case "kb_search":
      return [];
    case "kb_list":
      return [];
    case "kb_read":
      return "_(browser stub)_  本文件需要 Tauri 后端读取。";
    case "kb_ingest":
      return "browser-stub";
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
    case "chat_send":
      return "stub-req-id";
    case "list_skills":
      return [
        { id: "deep-research", name: "深度搜索", description: "使用 LLM 大规模联网搜索相关内容，自动检索、汇总、交叉验证多来源信息", source: "third-party" },
        { id: "skill-creator", name: "Skill 创建向导", description: "引导用户创建自定义 Skill，自动生成模板和配置文件", source: "official" },
      ];
    case "get_skill":
      return { id: "deep-research", name: "深度搜索", description: "使用 LLM 大规模联网搜索相关内容", source: "third-party" };
    case "create_skill":
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
    default:
      return null;
  }
}
