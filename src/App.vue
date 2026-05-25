<script setup lang="ts">
import { computed } from "vue";
import Sidebar from "./components/Sidebar.vue";
import RightDrawer from "./components/RightDrawer.vue";
import ChatPanel from "./components/ChatPanel.vue";
import WikiBrowse from "./components/WikiBrowse.vue";
import KnowledgeGraph from "./components/KnowledgeGraph.vue";
import SandboxStatus from "./features/sandbox/components/SandboxStatus.vue";
import ClaudeMdPanel from "./components/ClaudeMdPanel.vue";
import Settings from "./components/Settings.vue";
import { useAppStore } from "./stores/app";

const app = useAppStore();

const layoutCols = computed(
  () => `${app.sidebarWidth}px 1fr ${app.drawerWidth}px`
);
</script>

<template>
  <div class="shell" :style="{ gridTemplateColumns: layoutCols }">
    <Sidebar />
    <main class="main">
      <ChatPanel v-if="app.view === 'chat'" />
      <WikiBrowse v-else-if="app.view === 'wiki'" />
      <KnowledgeGraph v-else-if="app.view === 'graph'" />
      <SandboxStatus v-else-if="app.view === 'sandbox'" />
      <ClaudeMdPanel v-else-if="app.view === 'claude_md'" />
      <Settings v-else-if="app.view === 'settings'" />
      <div v-else class="placeholder">—</div>
    </main>
    <RightDrawer />
  </div>
</template>

<style scoped>
.shell {
  height: 100vh;
  display: grid;
  transition: grid-template-columns 180ms ease;
}
.main {
  height: 100vh;
  overflow: hidden;
  background: var(--bg);
  display: flex;
  flex-direction: column;
}
.placeholder {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--muted);
  font-family: var(--serif);
  font-size: 14px;
  letter-spacing: 2px;
}
</style>
