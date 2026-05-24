<script setup lang="ts">
import { ref } from "vue";
import { useAppStore } from "../stores/app";

const app = useAppStore();
const activeTab = ref<"artifacts" | "ref" | "audit">("artifacts");
</script>

<template>
  <aside class="dr" :class="{ collapsed: app.drawerCollapsed }">
    <div v-if="!app.drawerCollapsed" class="dh">
      <span class="title">文件抽屉</span>
      <button
        class="dh-btn"
        title="收起抽屉 (Ctrl+])"
        @click="app.toggleDrawer()"
      >
        ⇥
      </button>
    </div>
    <button
      v-else
      class="dh-btn rail"
      title="展开抽屉 (Ctrl+])"
      @click="app.toggleDrawer()"
    >
      ⇤
    </button>

    <template v-if="!app.drawerCollapsed">
      <div class="tabs">
        <button
          v-for="t in [
            { k: 'artifacts', l: '输出产物' },
            { k: 'ref', l: '参考资料' },
            { k: 'audit', l: '沙箱日志' },
          ]"
          :key="t.k"
          class="tab"
          :class="{ active: activeTab === t.k }"
          @click="activeTab = t.k as any"
        >
          {{ t.l }}
        </button>
      </div>
      <div class="body">
        <div class="empty">
          <div class="empty-glyph">▤</div>
          <div class="empty-text">
            <template v-if="activeTab === 'artifacts'">
              当前对话暂无输出产物
            </template>
            <template v-else-if="activeTab === 'ref'">
              本轮无 KB 召回引用
            </template>
            <template v-else> 沙箱待启动 / 暂无审计事件 </template>
          </div>
        </div>
      </div>
    </template>

    <template v-else>
      <div class="rail-tabs">
        <button class="rail-tab active" title="输出产物">▤</button>
        <button class="rail-tab" title="参考资料">▦</button>
        <button class="rail-tab" title="沙箱日志">⛨</button>
      </div>
    </template>
  </aside>
</template>

<style scoped>
.dr {
  background: var(--panel);
  border-left: 1px solid var(--border-soft);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.dr.collapsed {
  padding: 8px 4px;
  align-items: center;
  gap: 8px;
}

.dh {
  display: flex;
  align-items: center;
  padding: 11px 12px;
  border-bottom: 1px solid var(--border-soft);
  gap: 6px;
}
.title {
  flex: 1;
  font-family: var(--serif);
  font-weight: 600;
  font-size: 13px;
}
.dh-btn {
  width: 24px;
  height: 24px;
  border: none;
  border-radius: 3px;
  background: transparent;
  color: var(--muted);
  font-size: 12px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}
.dh-btn:hover {
  background: var(--selection-bg);
  color: var(--text);
}
.dh-btn.rail {
  margin-top: 4px;
}

.tabs {
  display: flex;
  border-bottom: 1px solid var(--border-soft);
  padding: 0 12px;
  gap: 14px;
  font-size: 12.5px;
}
.tab {
  background: transparent;
  border: none;
  padding: 10px 0;
  color: var(--muted);
}
.tab.active {
  color: var(--text);
  font-weight: 600;
  border-bottom: 2px solid var(--ink);
  margin-bottom: -1px;
}

.body {
  flex: 1;
  overflow-y: auto;
}
.empty {
  height: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  color: var(--dim);
  font-size: 12.5px;
  text-align: center;
  padding: 40px 20px;
  font-family: var(--serif);
  letter-spacing: 1px;
}
.empty-glyph {
  font-size: 28px;
  color: var(--border-strong);
  margin-bottom: 12px;
}

.rail-tabs {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 8px;
}
.rail-tab {
  width: 32px;
  height: 32px;
  border: none;
  border-radius: 4px;
  background: transparent;
  color: var(--muted);
  font-family: var(--serif);
  font-size: 13px;
}
.rail-tab:hover {
  background: var(--selection-bg);
  color: var(--text);
}
.rail-tab.active {
  background: var(--selection-bg);
  color: var(--ink);
}
</style>
