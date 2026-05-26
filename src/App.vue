<script setup lang="ts">
import { computed, ref } from "vue";
import Sidebar from "./components/Sidebar.vue";
import RightDrawer from "./components/RightDrawer.vue";
import ChatPanel from "./components/ChatPanel.vue";
import WikiBrowse from "./components/WikiBrowse.vue";
import KnowledgeGraph from "./components/KnowledgeGraph.vue";
import SandboxStatus from "./features/sandbox/components/SandboxStatus.vue";
import ClaudeMdPanel from "./components/ClaudeMdPanel.vue";
import Settings from "./components/Settings.vue";
import SkillCenter from "./components/SkillCenter.vue";
import AddProviderModal from "./components/AddProviderModal.vue";
import UsageBoard from "./components/UsageBoard.vue";
import SplashScreen from "./components/SplashScreen.vue";
import Onboarding from "./components/Onboarding.vue";
import { useAppStore } from "./stores/app";
import { useArtifactsStore } from "./stores/artifacts";
import { useProvidersStore } from "./stores/providers";

const app = useAppStore();
const artifacts = useArtifactsStore();
const providers = useProvidersStore();

// 启动流程：splash(每次) → onboarding(仅首次) → ready
const ONBOARDED_KEY = "polaris.onboarded.v1";
const phase = ref<"splash" | "onboarding" | "ready">("splash");

function onSplashDone() {
  const done = localStorage.getItem(ONBOARDED_KEY);
  phase.value = done ? "ready" : "onboarding";
}
function onOnboardingDone() {
  phase.value = "ready";
}

// 预览成品文件时把右侧抽屉拓宽；展开模式更宽，让观看更好看
const drawerTrack = computed(() => {
  if (artifacts.current) {
    return artifacts.expanded ? "min(1040px, 72vw)" : "clamp(400px, 36vw, 560px)";
  }
  return `${app.drawerWidth}px`;
});

const layoutCols = computed(
  () => `${app.sidebarWidth}px 1fr ${drawerTrack.value}`
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
      <SkillCenter v-else-if="app.view === 'skill_center'" />
      <Settings v-else-if="app.view === 'settings'" />
      <div v-else class="placeholder">—</div>
    </main>
    <RightDrawer />

    <AddProviderModal v-if="providers.showAddModal" />
    <UsageBoard v-if="providers.showUsageBoard" />

    <!-- 启动流程覆盖层：splash → onboarding -->
    <Transition name="splash-fade">
      <SplashScreen v-if="phase === 'splash'" @done="onSplashDone" />
    </Transition>
    <Transition name="onboard-fade">
      <Onboarding v-if="phase === 'onboarding'" @done="onOnboardingDone" />
    </Transition>
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

<!-- 非 scoped：Transition 类名需作用在子组件根元素上 -->
<style>
.splash-fade-leave-active {
  transition: opacity 0.8s ease;
}
.splash-fade-leave-to {
  opacity: 0;
}
.onboard-fade-enter-active {
  transition: opacity 0.4s ease;
}
.onboard-fade-leave-active {
  transition: opacity 0.45s ease;
}
.onboard-fade-enter-from,
.onboard-fade-leave-to {
  opacity: 0;
}
</style>
