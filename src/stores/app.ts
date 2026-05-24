import { defineStore } from "pinia";
import { ref, computed } from "vue";
import {
  convApi,
  type Conversation,
  type Project,
} from "../tauri";

export type ViewKey =
  | "chat"
  | "wiki"
  | "graph"
  | "sandbox"
  | "claude_md"
  | "settings";

export const useAppStore = defineStore("app", () => {
  const view = ref<ViewKey>("chat");
  const sidebarCollapsed = ref(false);
  const drawerCollapsed = ref(false);

  // 项目 + 对话
  const projects = ref<Project[]>([]);
  const expandedProjects = ref<Set<string>>(new Set());
  const conversationsByProject = ref<Record<string, Conversation[]>>({});
  const currentConvId = ref<string | null>(null);
  const currentProjectId = ref<string | null>(null);

  function setView(v: ViewKey) {
    view.value = v;
  }
  function toggleSidebar() {
    sidebarCollapsed.value = !sidebarCollapsed.value;
  }
  function toggleDrawer() {
    drawerCollapsed.value = !drawerCollapsed.value;
  }

  const sidebarWidth = computed(() => (sidebarCollapsed.value ? 48 : 260));
  const drawerWidth = computed(() => (drawerCollapsed.value ? 48 : 300));

  async function refreshProjects() {
    projects.value = await convApi.listProjects();
    if (!currentProjectId.value && projects.value.length) {
      currentProjectId.value = projects.value[0].id;
      expandedProjects.value.add(currentProjectId.value);
      await refreshConversations(currentProjectId.value);
    }
  }

  async function refreshConversations(projectId: string) {
    conversationsByProject.value[projectId] =
      await convApi.listConversations(projectId);
    // Vue 3 reactive: 替换 ref 触发更新
    conversationsByProject.value = { ...conversationsByProject.value };
  }

  async function toggleProject(projectId: string) {
    if (expandedProjects.value.has(projectId)) {
      expandedProjects.value.delete(projectId);
    } else {
      expandedProjects.value.add(projectId);
      if (!conversationsByProject.value[projectId]) {
        await refreshConversations(projectId);
      }
    }
    expandedProjects.value = new Set(expandedProjects.value);
  }

  async function createProject(name: string) {
    const p = await convApi.createProject(name);
    projects.value = [...projects.value, p];
    expandedProjects.value = new Set([...expandedProjects.value, p.id]);
    currentProjectId.value = p.id;
    conversationsByProject.value = { ...conversationsByProject.value, [p.id]: [] };
    return p;
  }

  async function createConversation(projectId: string) {
    const c = await convApi.createConversation(projectId);
    const cur = conversationsByProject.value[projectId] ?? [];
    conversationsByProject.value = {
      ...conversationsByProject.value,
      [projectId]: [c, ...cur],
    };
    expandedProjects.value = new Set([...expandedProjects.value, projectId]);
    currentConvId.value = c.id;
    currentProjectId.value = projectId;
    setView("chat");
    return c;
  }

  async function deleteConversation(conv: Conversation) {
    await convApi.deleteConversation(conv.id);
    const cur = conversationsByProject.value[conv.projectId] ?? [];
    conversationsByProject.value = {
      ...conversationsByProject.value,
      [conv.projectId]: cur.filter((c) => c.id !== conv.id),
    };
    if (currentConvId.value === conv.id) {
      currentConvId.value = null;
    }
  }

  function selectConversation(conv: Conversation) {
    currentConvId.value = conv.id;
    currentProjectId.value = conv.projectId;
    setView("chat");
  }

  return {
    // ui
    view,
    sidebarCollapsed,
    drawerCollapsed,
    sidebarWidth,
    drawerWidth,
    setView,
    toggleSidebar,
    toggleDrawer,
    // conv
    projects,
    expandedProjects,
    conversationsByProject,
    currentConvId,
    currentProjectId,
    refreshProjects,
    refreshConversations,
    toggleProject,
    createProject,
    createConversation,
    deleteConversation,
    selectConversation,
  };
});
