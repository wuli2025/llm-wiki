<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useAppStore } from "../stores/app";
import type { Conversation } from "../tauri";

const app = useAppStore();

const navItems: { key: typeof app.view; label: string; glyph: string }[] = [
  { key: "chat", label: "对话", glyph: "✎" },
  { key: "wiki", label: "知识库", glyph: "▥" },
  { key: "graph", label: "图谱", glyph: "◈" },
  { key: "sandbox", label: "沙箱", glyph: "⛨" },
  { key: "claude_md", label: "目录说明", glyph: "❡" },
  { key: "settings", label: "设置", glyph: "⚙" },
];

const newProjectName = ref("");
const showNewProject = ref(false);

onMounted(() => {
  app.refreshProjects();
});

async function submitNewProject() {
  const n = newProjectName.value.trim();
  if (!n) {
    showNewProject.value = false;
    return;
  }
  await app.createProject(n);
  newProjectName.value = "";
  showNewProject.value = false;
}

async function newConv(pid: string) {
  await app.createConversation(pid);
}

async function confirmDelete(c: Conversation) {
  if (confirm(`删除对话「${c.title}」?(消息也会被清空)`)) {
    await app.deleteConversation(c);
  }
}
</script>

<template>
  <aside class="sb" :class="{ collapsed: app.sidebarCollapsed }">
    <!-- Head -->
    <div class="sb-head">
      <template v-if="!app.sidebarCollapsed">
        <div class="brand"><span class="b-dot"></span>北极星</div>
        <button
          class="collapse-btn"
          title="收起侧栏"
          @click="app.toggleSidebar()"
        >
          ⇤
        </button>
      </template>
      <template v-else>
        <button
          class="collapse-btn rail"
          title="展开侧栏"
          @click="app.toggleSidebar()"
        >
          ⇥
        </button>
      </template>
    </div>

    <!-- Nav -->
    <nav class="nav">
      <button
        v-for="it in navItems"
        :key="it.key"
        class="nav-item"
        :class="{ active: app.view === it.key }"
        :title="it.label"
        @click="app.setView(it.key)"
      >
        <span class="glyph">{{ it.glyph }}</span>
        <span v-if="!app.sidebarCollapsed" class="label">{{ it.label }}</span>
      </button>
    </nav>

    <!-- Projects + Conversations -->
    <div v-if="!app.sidebarCollapsed" class="proj-section">
      <div class="proj-head">
        <span class="proj-title">项目</span>
        <button
          class="ic-btn plus"
          title="新建项目"
          @click="showNewProject = !showNewProject"
        >
          +
        </button>
      </div>

      <div v-if="showNewProject" class="new-proj-row">
        <input
          v-model="newProjectName"
          placeholder="项目名"
          @keydown.enter="submitNewProject"
          @keydown.esc="(showNewProject = false), (newProjectName = '')"
          autofocus
        />
        <button class="primary-mini" @click="submitNewProject">建</button>
      </div>

      <div v-for="proj in app.projects" :key="proj.id" class="proj-block">
        <div
          class="proj"
          :class="{ active: app.currentProjectId === proj.id }"
          @click="app.toggleProject(proj.id)"
        >
          <span class="arrow">{{
            app.expandedProjects.has(proj.id) ? "▾" : "▸"
          }}</span>
          <span class="name">{{ proj.name }}</span>
          <button
            class="ic-btn plus mini"
            title="新建对话"
            @click.stop="newConv(proj.id)"
          >
            +
          </button>
        </div>

        <template v-if="app.expandedProjects.has(proj.id)">
          <div class="day-label">对话</div>
          <div
            v-for="c in app.conversationsByProject[proj.id] || []"
            :key="c.id"
            class="conv"
            :class="{ active: app.currentConvId === c.id }"
            @click="app.selectConversation(c)"
          >
            <span class="cv-name" :title="c.title">{{ c.title }}</span>
            <button
              class="ca delete"
              title="删除对话"
              @click.stop="confirmDelete(c)"
            >
              ×
            </button>
          </div>
          <div
            v-if="(app.conversationsByProject[proj.id] || []).length === 0"
            class="empty-hint"
          >
            点项目右侧 + 新建对话
          </div>
        </template>
      </div>
    </div>

    <div class="footer">
      <div v-if="!app.sidebarCollapsed" class="footer-text">
        v0.2 · 墨蓝水墨
      </div>
    </div>
  </aside>
</template>

<style scoped>
.sb {
  background: var(--bg-soft);
  border-right: 1px solid var(--border-soft);
  display: flex;
  flex-direction: column;
  padding: 8px 8px 6px;
  overflow: hidden;
}
.sb.collapsed {
  padding: 8px 4px;
}

.sb-head {
  display: flex;
  align-items: center;
  padding: 4px 4px 10px;
  border-bottom: 1px solid var(--border-soft);
  margin-bottom: 8px;
  gap: 6px;
}
.brand {
  flex: 1;
  font-family: var(--serif);
  font-weight: 600;
  font-size: 14px;
  color: var(--ink);
  letter-spacing: 2px;
  display: inline-flex;
  align-items: center;
  gap: 8px;
}
.b-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--primary);
}
.collapse-btn {
  width: 24px;
  height: 24px;
  border-radius: 3px;
  background: transparent;
  border: none;
  color: var(--muted);
  font-size: 13px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}
.collapse-btn:hover {
  background: var(--selection-bg);
  color: var(--text);
}
.collapse-btn.rail {
  margin: 0 auto;
}

.nav {
  display: flex;
  flex-direction: column;
  gap: 1px;
}
.nav-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 7px 10px;
  border: none;
  border-radius: 3px;
  background: transparent;
  color: var(--text-2);
  font-size: 13px;
  text-align: left;
}
.nav-item:hover {
  background: var(--selection-bg);
}
.nav-item.active {
  background: var(--selection-bg);
  color: var(--text);
  font-weight: 500;
  border-left: 2px solid var(--ink);
  padding-left: 8px;
}
.sb.collapsed .nav-item {
  justify-content: center;
  padding: 7px 0;
}
.sb.collapsed .nav-item.active {
  border-left: none;
  border-right: 2px solid var(--ink);
}
.glyph {
  display: inline-block;
  width: 16px;
  text-align: center;
  color: var(--muted);
  font-family: var(--serif);
}
.nav-item.active .glyph {
  color: var(--ink);
}
.label {
  flex: 1;
}

.proj-section {
  margin-top: 14px;
  padding-top: 10px;
  border-top: 1px solid var(--border-soft);
  overflow-y: auto;
  flex: 1;
}
.proj-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 10px 6px;
}
.proj-title {
  font-family: var(--serif);
  font-size: 11px;
  letter-spacing: 1.5px;
  color: var(--dim);
}
.ic-btn {
  width: 18px;
  height: 18px;
  border: none;
  border-radius: 3px;
  background: transparent;
  color: var(--muted);
  font-size: 14px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  line-height: 1;
}
.ic-btn:hover {
  background: var(--border);
  color: var(--text);
}
.ic-btn.plus {
  background: var(--ink);
  color: #fff;
  font-size: 11px;
}
.ic-btn.plus:hover {
  background: var(--primary);
}
.ic-btn.mini {
  opacity: 0;
}

.new-proj-row {
  display: flex;
  gap: 4px;
  padding: 4px 10px 6px;
}
.new-proj-row input {
  flex: 1;
  padding: 4px 6px;
  border: 1px solid var(--border);
  border-radius: 3px;
  font-size: 12px;
  background: var(--panel);
}
.new-proj-row input:focus {
  outline: none;
  border-color: var(--primary);
}
.primary-mini {
  padding: 2px 10px;
  background: var(--ink);
  color: #fff;
  border: none;
  border-radius: 3px;
  font-size: 11px;
}
.primary-mini:hover {
  background: var(--primary);
}

.proj-block {
  margin-bottom: 2px;
}
.proj {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 10px;
  font-size: 13px;
  border-radius: 3px;
  cursor: pointer;
}
.proj:hover {
  background: var(--selection-bg);
}
.proj:hover .ic-btn.mini {
  opacity: 1;
}
.proj.active {
  background: var(--selection-bg);
  font-weight: 600;
}
.proj .arrow {
  width: 10px;
  font-size: 10px;
  color: var(--muted);
}
.proj .name {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.day-label {
  font-size: 10.5px;
  color: var(--dim);
  padding: 6px 10px 2px 26px;
  font-family: var(--serif);
  letter-spacing: 1px;
}
.conv {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 4px 10px 4px 26px;
  font-size: 12.5px;
  color: var(--muted);
  border-radius: 3px;
  cursor: pointer;
}
.conv:hover {
  background: var(--selection-bg);
  color: var(--text);
}
.conv:hover .ca {
  opacity: 1;
}
.conv.active {
  background: var(--selection-bg);
  color: var(--text);
  font-weight: 500;
}
.cv-name {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ca {
  width: 16px;
  height: 16px;
  border: none;
  background: transparent;
  color: var(--muted);
  font-size: 13px;
  border-radius: 2px;
  opacity: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  line-height: 1;
}
.ca:hover {
  background: var(--border);
  color: var(--text);
}
.ca.delete:hover {
  color: var(--vermilion);
}

.empty-hint {
  font-size: 11px;
  color: var(--dim);
  padding: 4px 10px 4px 26px;
  font-style: italic;
}

.footer {
  margin-top: auto;
  padding-top: 6px;
  border-top: 1px solid var(--border-soft);
}
.footer-text {
  font-size: 10.5px;
  color: var(--dim);
  text-align: center;
  font-family: var(--serif);
  letter-spacing: 1.5px;
  padding: 4px 0;
}
</style>
