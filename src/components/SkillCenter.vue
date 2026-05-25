<script setup lang="ts">
import { ref, onMounted } from "vue";
import { Puzzle, Plus, Search, Sparkles, Globe, Wrench } from "@lucide/vue";
import { skills as skillsApi, type Skill } from "../tauri";

const activeTab = ref<"market" | "mine">("market");
const skillList = ref<Skill[]>([]);
const searchQuery = ref("");

onMounted(async () => {
  try {
    skillList.value = await skillsApi.list();
  } catch {
    // browser stub fallback
    skillList.value = [
      { id: "deep-research", name: "深度搜索", description: "使用 LLM 大规模联网搜索相关内容，自动检索、汇总、交叉验证多来源信息", source: "third-party" },
      { id: "skill-creator", name: "Skill 创建向导", description: "引导用户创建自定义 Skill，自动生成模板和配置文件", source: "official" },
    ];
  }
});

const filteredSkills = () => {
  if (!searchQuery.value.trim()) return skillList.value;
  const q = searchQuery.value.toLowerCase();
  return skillList.value.filter(
    (s) =>
      s.name.toLowerCase().includes(q) ||
      s.description.toLowerCase().includes(q)
  );
};

function iconForSkill(skill: Skill) {
  if (skill.id === "deep-research") return Globe;
  if (skill.id === "skill-creator") return Wrench;
  return Sparkles;
}

function sourceLabel(source: string) {
  return source === "official" ? "官方" : "第三方";
}
</script>

<template>
  <div class="skill-center">
    <!-- Header -->
    <div class="sc-header">
      <div class="sc-title">
        <Puzzle :size="20" :stroke-width="1.8" class="sc-title-icon" />
        <span>技能中心</span>
      </div>
      <button class="sc-new-btn">
        <Plus :size="14" :stroke-width="2" />
        <span>新技能</span>
      </button>
    </div>

    <!-- Search + Tabs -->
    <div class="sc-toolbar">
      <div class="sc-tabs">
        <button
          class="sc-tab"
          :class="{ active: activeTab === 'market' }"
          @click="activeTab = 'market'"
        >
          市场精选
        </button>
        <button
          class="sc-tab"
          :class="{ active: activeTab === 'mine' }"
          @click="activeTab = 'mine'"
        >
          我的技能
        </button>
      </div>
      <div class="sc-search">
        <Search :size="14" :stroke-width="1.8" class="sc-search-icon" />
        <input
          v-model="searchQuery"
          placeholder="搜索技能..."
          type="text"
        />
      </div>
    </div>

    <!-- Skill Grid -->
    <div class="sc-grid">
      <div
        v-for="skill in filteredSkills()"
        :key="skill.id"
        class="sc-card"
      >
        <div class="sc-card-head">
          <div class="sc-card-icon">
            <component
              :is="iconForSkill(skill)"
              :size="22"
              :stroke-width="1.6"
            />
          </div>
          <div class="sc-card-meta">
            <div class="sc-card-name">{{ skill.name }}</div>
            <div class="sc-card-source">{{ sourceLabel(skill.source) }}</div>
          </div>
        </div>
        <div class="sc-card-desc">{{ skill.description }}</div>
        <div class="sc-card-foot">
          <button class="sc-card-use">使用</button>
        </div>
      </div>
    </div>

    <!-- Empty state -->
    <div v-if="filteredSkills().length === 0" class="sc-empty">
      暂无技能
    </div>
  </div>
</template>

<style scoped>
.skill-center {
  height: 100vh;
  overflow-y: auto;
  padding: 24px 32px;
  background: var(--bg);
}
.sc-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 20px;
}
.sc-title {
  display: flex;
  align-items: center;
  gap: 10px;
  font-family: var(--serif);
  font-size: 18px;
  font-weight: 600;
  color: var(--ink);
  letter-spacing: 1px;
}
.sc-title-icon {
  color: var(--primary);
}
.sc-new-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 6px 14px;
  background: var(--ink);
  color: #fafaf7;
  border: none;
  border-radius: 6px;
  font-size: 12.5px;
  cursor: pointer;
}
.sc-new-btn:hover {
  background: var(--primary);
}

.sc-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  margin-bottom: 20px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--border-soft);
}
.sc-tabs {
  display: flex;
  gap: 4px;
}
.sc-tab {
  padding: 6px 14px;
  border: none;
  background: transparent;
  color: var(--muted);
  font-size: 13px;
  border-radius: 4px;
  cursor: pointer;
}
.sc-tab:hover {
  color: var(--text);
  background: var(--bg-soft);
}
.sc-tab.active {
  color: var(--ink);
  background: var(--panel);
  font-weight: 600;
  box-shadow: var(--shadow-sm);
}
.sc-search {
  display: flex;
  align-items: center;
  gap: 8px;
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 5px 10px;
  width: 240px;
}
.sc-search-icon {
  color: var(--muted);
  flex-shrink: 0;
}
.sc-search input {
  border: none;
  outline: none;
  background: transparent;
  font-size: 12.5px;
  color: var(--text);
  width: 100%;
}
.sc-search input::placeholder {
  color: var(--dim);
}

.sc-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
  gap: 16px;
}
.sc-card {
  background: var(--panel);
  border: 1px solid var(--border-soft);
  border-radius: 10px;
  padding: 16px;
  box-shadow: var(--shadow-sm);
  transition: box-shadow 0.15s, border-color 0.15s;
}
.sc-card:hover {
  box-shadow: var(--shadow);
  border-color: var(--border);
}
.sc-card-head {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 10px;
}
.sc-card-icon {
  width: 36px;
  height: 36px;
  border-radius: 8px;
  background: var(--primary-soft);
  color: var(--primary);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}
.sc-card-meta {
  flex: 1;
  min-width: 0;
}
.sc-card-name {
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
}
.sc-card-source {
  font-size: 11px;
  color: var(--muted);
  margin-top: 2px;
}
.sc-card-desc {
  font-size: 12px;
  color: var(--text-2);
  line-height: 1.6;
  margin-bottom: 12px;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.sc-card-foot {
  display: flex;
  justify-content: flex-end;
}
.sc-card-use {
  padding: 5px 14px;
  background: var(--ink);
  color: #fafaf7;
  border: none;
  border-radius: 5px;
  font-size: 12px;
  cursor: pointer;
}
.sc-card-use:hover {
  background: var(--primary);
}

.sc-empty {
  text-align: center;
  padding: 60px 0;
  color: var(--muted);
  font-size: 13px;
}
</style>
