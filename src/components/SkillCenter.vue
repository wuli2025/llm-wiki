<script setup lang="ts">
import { ref, onMounted, computed } from "vue";
import {
  Puzzle,
  Plus,
  Search,
  Sparkles,
  Globe,
  Wrench,
  Trash2,
  X,
} from "@lucide/vue";
import { skills as skillsApi, type Skill } from "../tauri";

const activeTab = ref<"market" | "mine">("market");
const skillList = ref<Skill[]>([]);
const searchQuery = ref("");
const loading = ref(false);

// 创建弹窗
const showCreateModal = ref(false);
const createForm = ref({
  id: "",
  name: "",
  description: "",
  systemPrompt: "",
});
const createError = ref("");

onMounted(loadSkills);

async function loadSkills() {
  loading.value = true;
  try {
    skillList.value = await skillsApi.list();
  } catch {
    skillList.value = [
      { id: "deep-research", name: "深度搜索", description: "...", source: "third-party" },
      { id: "skill-creator", name: "Skill 创建向导", description: "...", source: "official" },
    ];
  } finally {
    loading.value = false;
  }
}

const marketSkills = computed(() =>
  skillList.value.filter((s) => s.source !== "user")
);

const mySkills = computed(() =>
  skillList.value.filter((s) => s.source === "user")
);

const currentSkills = computed(() => {
  const list = activeTab.value === "market" ? marketSkills.value : mySkills.value;
  if (!searchQuery.value.trim()) return list;
  const q = searchQuery.value.toLowerCase();
  return list.filter(
    (s) =>
      s.name.toLowerCase().includes(q) ||
      s.description.toLowerCase().includes(q)
  );
});

function iconForSkill(skill: Skill) {
  if (skill.id === "deep-research") return Globe;
  if (skill.id === "skill-creator") return Wrench;
  return Sparkles;
}

function sourceLabel(source: string) {
  if (source === "official") return "官方";
  if (source === "third-party") return "第三方";
  return "我的";
}

async function onDelete(skill: Skill) {
  if (!confirm(`确定删除技能「${skill.name}」?`)) return;
  try {
    await skillsApi.delete(skill.id);
    await loadSkills();
  } catch (e: any) {
    alert(`删除失败: ${e?.message ?? e}`);
  }
}

function openCreateModal() {
  createForm.value = { id: "", name: "", description: "", systemPrompt: "" };
  createError.value = "";
  showCreateModal.value = true;
}

function closeCreateModal() {
  showCreateModal.value = false;
}

function sanitizeId(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^\w\s-]/g, "")
    .replace(/\s+/g, "-")
    .replace(/-+/g, "-")
    .replace(/^-|-$/g, "");
}

function onNameInput() {
  if (!createForm.value.id) {
    createForm.value.id = sanitizeId(createForm.value.name);
  }
}

async function submitCreate() {
  createError.value = "";
  const { id, name, description, systemPrompt } = createForm.value;
  if (!id.trim() || !name.trim() || !systemPrompt.trim()) {
    createError.value = "ID、名称和 System Prompt 为必填项";
    return;
  }
  try {
    await skillsApi.create(id.trim(), name.trim(), description.trim(), systemPrompt.trim());
    await loadSkills();
    closeCreateModal();
    activeTab.value = "mine";
  } catch (e: any) {
    createError.value = e?.message ?? String(e);
  }
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
      <button class="sc-new-btn" @click="openCreateModal">
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
          <span v-if="mySkills.length > 0" class="sc-tab-badge">{{ mySkills.length }}</span>
        </button>
      </div>
      <div class="sc-search">
        <Search :size="14" :stroke-width="1.8" class="sc-search-icon" />
        <input v-model="searchQuery" placeholder="搜索技能..." type="text" />
      </div>
    </div>

    <!-- Skill Grid -->
    <div v-if="!loading" class="sc-grid">
      <div v-for="skill in currentSkills" :key="skill.id" class="sc-card">
        <div class="sc-card-head">
          <div class="sc-card-icon">
            <component :is="iconForSkill(skill)" :size="22" :stroke-width="1.6" />
          </div>
          <div class="sc-card-meta">
            <div class="sc-card-name">{{ skill.name }}</div>
            <div class="sc-card-source">{{ sourceLabel(skill.source) }}</div>
          </div>
        </div>
        <div class="sc-card-desc">{{ skill.description }}</div>
        <div class="sc-card-foot">
          <button v-if="skill.source === 'user'" class="sc-card-delete" @click="onDelete(skill)"
            title="删除"
          >
            <Trash2 :size="13" :stroke-width="1.8" />
          </button>
          <button class="sc-card-use">使用</button>
        </div>
      </div>
    </div>

    <!-- Empty state -->
    <div v-if="currentSkills.length === 0 && !loading" class="sc-empty">
      <template v-if="activeTab === 'mine'">
        <div>还没有创建技能</div>
        <button class="sc-empty-btn" @click="openCreateModal">+ 创建第一个技能</button>
      </template>
      <template v-else>
        暂无技能
      </template>
    </div>

    <!-- 创建弹窗 -->
    <div v-if="showCreateModal" class="modal-overlay" @click.self="closeCreateModal">
      <div class="modal">
        <div class="modal-head">
          <span class="modal-title">创建新技能</span>
          <button class="modal-close" @click="closeCreateModal">
            <X :size="16" :stroke-width="2" />
          </button>
        </div>
        <div class="modal-body">
          <div class="form-row">
            <label>名称</label>
            <input v-model="createForm.name" placeholder="例如: 高老师风格写作" @input="onNameInput" />
          </div>
          <div class="form-row">
            <label>ID（唯一标识，只能用小写字母、数字、-）</label>
            <input v-model="createForm.id" placeholder="gao-style-writer" />
          </div>
          <div class="form-row">
            <label>描述</label>
            <input v-model="createForm.description" placeholder="一句话描述这个技能的作用..." />
          </div>
          <div class="form-row">
            <label>System Prompt（核心指令）</label>
            <textarea
              v-model="createForm.systemPrompt"
              placeholder="# 角色定义&#10;&#10;你是...&#10;&#10;## 工作方式&#10;1. ..."
              rows="8"
            ></textarea>
          </div>
          <div v-if="createError" class="form-error">{{ createError }}</div>
        </div>
        <div class="modal-foot">
          <button class="modal-btn secondary" @click="closeCreateModal">取消</button>
          <button class="modal-btn primary" @click="submitCreate">创建</button>
        </div>
      </div>
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
  display: inline-flex;
  align-items: center;
  gap: 6px;
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
.sc-tab-badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 18px;
  height: 18px;
  padding: 0 5px;
  background: var(--primary-soft);
  color: var(--primary);
  border-radius: 9px;
  font-size: 10.5px;
  font-weight: 600;
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
  align-items: center;
  gap: 8px;
  justify-content: flex-end;
}
.sc-card-delete {
  padding: 5px;
  background: transparent;
  border: none;
  color: var(--muted);
  border-radius: 4px;
  cursor: pointer;
}
.sc-card-delete:hover {
  color: var(--vermilion);
  background: rgba(192, 57, 43, 0.06);
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
.sc-empty-btn {
  margin-top: 12px;
  padding: 6px 16px;
  background: var(--ink);
  color: #fafaf7;
  border: none;
  border-radius: 6px;
  font-size: 12.5px;
  cursor: pointer;
}
.sc-empty-btn:hover {
  background: var(--primary);
}

/* ─────────── 创建弹窗 ─────────── */
.modal-overlay {
  position: fixed;
  inset: 0;
  background: rgba(20, 20, 25, 0.35);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}
.modal {
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 12px;
  width: 520px;
  max-width: 90vw;
  max-height: 85vh;
  display: flex;
  flex-direction: column;
  box-shadow: var(--shadow-lg);
}
.modal-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 16px 20px;
  border-bottom: 1px solid var(--border-soft);
}
.modal-title {
  font-size: 15px;
  font-weight: 600;
  color: var(--text);
}
.modal-close {
  width: 28px;
  height: 28px;
  border: none;
  background: transparent;
  color: var(--muted);
  border-radius: 4px;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}
.modal-close:hover {
  background: var(--bg-soft);
  color: var(--text);
}
.modal-body {
  padding: 16px 20px;
  overflow-y: auto;
}
.form-row {
  margin-bottom: 14px;
}
.form-row label {
  display: block;
  font-size: 12px;
  color: var(--text-2);
  margin-bottom: 5px;
}
.form-row input,
.form-row textarea {
  width: 100%;
  padding: 8px 10px;
  border: 1px solid var(--border);
  border-radius: 6px;
  font-size: 13px;
  background: var(--bg);
  color: var(--text);
  outline: none;
  resize: vertical;
}
.form-row input:focus,
.form-row textarea:focus {
  border-color: var(--primary);
}
.form-error {
  color: var(--vermilion);
  font-size: 12px;
  padding: 4px 0;
}
.modal-foot {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  padding: 12px 20px 16px;
  border-top: 1px solid var(--border-soft);
}
.modal-btn {
  padding: 6px 16px;
  border-radius: 6px;
  font-size: 13px;
  border: none;
  cursor: pointer;
}
.modal-btn.secondary {
  background: var(--bg-soft);
  color: var(--text-2);
}
.modal-btn.secondary:hover {
  background: var(--border);
}
.modal-btn.primary {
  background: var(--ink);
  color: #fafaf7;
}
.modal-btn.primary:hover {
  background: var(--primary);
}
</style>
