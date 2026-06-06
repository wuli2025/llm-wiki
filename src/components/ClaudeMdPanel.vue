<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import {
  claudeMd,
  convApi,
  persona as personaApi,
  type ClaudeMdArea,
  type KbClaudeMd,
  type PersonaPreset,
  type ProjectClaudeMd,
} from "../tauri";

type Selected =
  | { kind: "kb" }
  | { kind: "project"; projectId: string; projectName: string };

const projects = ref<ProjectClaudeMd[]>([]);
const kbInfo = ref<KbClaudeMd | null>(null);

// 板块⑫: 预设人格库 + 每个项目的人格/知识库 scope 元数据
const presets = ref<PersonaPreset[]>([]);
const projMeta = ref<Record<string, { personaId: string | null; kbScope: string | null }>>({});
const kbScope = ref("");
const applying = ref(false);
const showGallery = ref(false);
// 非阻塞的覆盖确认（替代原生 confirm，避免模态卡死界面）
const pendingOverwrite = ref<PersonaPreset | null>(null);

// 给后端调用加超时，避免任一命令异常时面板无限转圈（=「卡住」）
function withTimeout<T>(p: Promise<T>, ms: number, label: string): Promise<T> {
  return Promise.race([
    p,
    new Promise<T>((_, rej) =>
      setTimeout(() => rej(new Error(`${label} 超时(${ms}ms)`)), ms)
    ),
  ]);
}

const selected = ref<Selected | null>(null);
const content = ref("");
const originalContent = ref("");
const loading = ref(false);
const saving = ref(false);
const message = ref<{ kind: "ok" | "err"; text: string } | null>(null);

const dirty = computed(() => content.value !== originalContent.value);

const selectedMeta = computed(() => {
  const s = selected.value;
  if (!s) return null;
  if (s.kind === "kb") {
    return {
      label: "知识库",
      sub: kbInfo.value?.absPath ?? "",
      exists: kbInfo.value?.exists ?? false,
      active: kbInfo.value?.active ?? false,
    };
  }
  const p = projects.value.find((x) => x.projectId === s.projectId);
  return {
    label: s.projectName,
    sub: p?.absPath ?? "",
    exists: p?.exists ?? false,
    active: p?.active ?? false,
  };
});

async function refresh() {
  loading.value = true;
  // 用 allSettled + 超时：任一命令失败/卡住都不再让整个面板空转，且能在顶部看到原因。
  const [psR, kbR, prR, metaR] = await Promise.allSettled([
    withTimeout(claudeMd.listProjects(), 8000, "加载项目"),
    withTimeout(claudeMd.kbInfo(), 8000, "加载知识库信息"),
    presets.value.length
      ? Promise.resolve(presets.value)
      : withTimeout(personaApi.list(), 8000, "加载预设人格"),
    withTimeout(convApi.listProjects(), 8000, "加载项目元数据"),
  ]);
  if (psR.status === "fulfilled") projects.value = psR.value;
  if (kbR.status === "fulfilled") kbInfo.value = kbR.value;
  if (prR.status === "fulfilled") presets.value = prR.value;
  if (metaR.status === "fulfilled") {
    const map: Record<string, { personaId: string | null; kbScope: string | null }> = {};
    for (const p of metaR.value) {
      map[p.id] = { personaId: p.personaId ?? null, kbScope: p.kbScope ?? null };
    }
    projMeta.value = map;
  }
  const failed = [psR, kbR, prR, metaR].find((r) => r.status === "rejected");
  if (failed && failed.status === "rejected") {
    message.value = { kind: "err", text: `加载异常：${failed.reason}` };
  }
  loading.value = false;
}

/** 点预设卡片：已有内容 → 弹内联覆盖确认（非阻塞）；否则直接套用 */
function applyPreset(preset: PersonaPreset) {
  if (!selected.value || selected.value.kind !== "project") return;
  const hasContent = !!content.value.trim() && !/polaris:placeholder/.test(content.value);
  if (hasContent) {
    pendingOverwrite.value = preset; // 显示内联确认条，不调原生 confirm
    return;
  }
  void doApply(preset, false);
}

/** 真正执行套用（写 CLAUDE.md + 绑定 scope） */
async function doApply(preset: PersonaPreset, overwrite: boolean) {
  if (!selected.value || selected.value.kind !== "project") return;
  const pid = selected.value.projectId;
  pendingOverwrite.value = null;
  applying.value = true;
  message.value = null;
  try {
    await withTimeout(personaApi.apply(pid, preset.id, overwrite), 8000, "套用人格");
    showGallery.value = false;
    await loadContent("project", pid);
    await refresh();
    kbScope.value = projMeta.value[pid]?.kbScope ?? preset.kbScope ?? "";
    message.value = { kind: "ok", text: `已套用人格「${preset.name}」` };
  } catch (err: any) {
    message.value = { kind: "err", text: `套用失败: ${err}` };
  } finally {
    applying.value = false;
  }
}

/** 保存当前项目的知识库 scope 绑定 */
async function saveScope() {
  if (!selected.value || selected.value.kind !== "project") return;
  const pid = selected.value.projectId;
  try {
    await convApi.setKbScope(pid, kbScope.value.trim() || null);
    await refresh();
    message.value = { kind: "ok", text: "已更新知识库范围" };
  } catch (err: any) {
    message.value = { kind: "err", text: `保存失败: ${err}` };
  }
}

async function selectKb() {
  if (dirty.value && !confirm("当前文件有未保存的修改, 切换会丢失。继续?")) return;
  selected.value = { kind: "kb" };
  await loadContent("kb");
}

async function selectProject(p: ProjectClaudeMd) {
  if (dirty.value && !confirm("当前文件有未保存的修改, 切换会丢失。继续?")) return;
  selected.value = {
    kind: "project",
    projectId: p.projectId,
    projectName: p.projectName,
  };
  kbScope.value = projMeta.value[p.projectId]?.kbScope ?? "";
  showGallery.value = false;
  await loadContent("project", p.projectId);
}

/** 当前选中项目套用的人格预设（用于显示图标/名称） */
const currentPersona = computed(() => {
  const s = selected.value;
  if (!s || s.kind !== "project") return null;
  const pid = projMeta.value[s.projectId]?.personaId;
  return presets.value.find((p) => p.id === pid) ?? null;
});

async function loadContent(area: ClaudeMdArea, projectId?: string) {
  message.value = null;
  try {
    const text = await claudeMd.read(area, projectId);
    content.value = text;
    originalContent.value = text;
  } catch (err: any) {
    message.value = { kind: "err", text: `读取失败: ${err}` };
    content.value = "";
    originalContent.value = "";
  }
}

async function save() {
  if (!selected.value || !dirty.value) return;
  saving.value = true;
  message.value = null;
  try {
    if (selected.value.kind === "kb") {
      await claudeMd.write("kb", undefined, content.value);
    } else {
      await claudeMd.write("project", selected.value.projectId, content.value);
    }
    originalContent.value = content.value;
    message.value = { kind: "ok", text: "已保存" };
    await refresh();
  } catch (err: any) {
    message.value = { kind: "err", text: `保存失败: ${err}` };
  } finally {
    saving.value = false;
  }
}

function revert() {
  content.value = originalContent.value;
  message.value = null;
}

function stripMarker() {
  const lines = content.value.split(/\r?\n/);
  while (lines.length && /polaris:placeholder/.test(lines[0])) lines.shift();
  while (lines.length && lines[0].trim() === "") lines.shift();
  content.value = lines.join("\n");
}

function statusBadge(active: boolean, exists: boolean): string {
  if (!exists) return "未创建";
  return active ? "已启用" : "占位";
}

onMounted(refresh);
</script>

<template>
  <div class="cmd-root">
    <div class="cmd-head">
      <div>
        <div class="title">人格 · 项目的灵魂与专属知识库</div>
        <div class="sub">
          每个项目就是一个人格(它的 <code>CLAUDE.md</code>)。可一键套用预设人格,
          并绑定该人格的专属知识库范围。发消息前自动注入(身份+人格+时间+对应知识库)。
        </div>
      </div>
      <button class="btn ghost" @click="refresh" :disabled="loading">
        {{ loading ? "刷新中…" : "重新扫描" }}
      </button>
    </div>

    <div class="cmd-body">
      <!-- Left: list -->
      <aside class="list">
        <div class="grp-head">知识库 · 全局共享</div>
        <button
          class="item"
          :class="{
            active: selected?.kind === 'kb',
            on: kbInfo?.active,
          }"
          @click="selectKb"
          :title="kbInfo?.absPath"
        >
          <span class="dot" :class="{ on: kbInfo?.active }" />
          <span class="rel">PolarisKB</span>
          <span
            class="badge"
            :class="{ on: kbInfo?.active, miss: !kbInfo?.exists }"
          >
            {{ statusBadge(kbInfo?.active ?? false, kbInfo?.exists ?? false) }}
          </span>
        </button>

        <div class="grp-head">项目 · {{ projects.length }}</div>
        <button
          v-for="p in projects"
          :key="p.projectId"
          class="item"
          :class="{
            active:
              selected?.kind === 'project' &&
              selected.projectId === p.projectId,
            on: p.active,
          }"
          @click="selectProject(p)"
          :title="p.absPath"
        >
          <span class="dot" :class="{ on: p.active }" />
          <span class="rel">{{ p.projectName }}</span>
          <span
            class="badge"
            :class="{ on: p.active, miss: !p.exists }"
          >
            {{ statusBadge(p.active, p.exists) }}
          </span>
        </button>

        <div v-if="projects.length === 0 && !loading" class="empty">
          没有项目。请先到左边栏新建项目。
        </div>
      </aside>

      <!-- Right: editor -->
      <section class="editor">
        <div v-if="!selected || !selectedMeta" class="placeholder">
          ← 从左边挑一个
        </div>
        <template v-else>
          <div class="ed-head">
            <div class="ed-path">
              <span class="ed-area">
                {{ selected.kind === "kb" ? "知识库" : "项目" }}
              </span>
              <span class="ed-rel">{{ selectedMeta.label }}</span>
              <span
                v-if="!selectedMeta.exists"
                class="badge miss"
                style="margin-left: 8px"
              >未创建(保存即新建)</span>
            </div>
            <div class="ed-actions">
              <button
                class="btn ghost"
                @click="stripMarker"
                :disabled="!/polaris:placeholder/.test(content)"
                title="一键删掉顶部 polaris:placeholder 行 → 启用"
              >
                启用 (删占位行)
              </button>
              <button class="btn ghost" @click="revert" :disabled="!dirty">
                还原
              </button>
              <button
                class="btn primary"
                @click="save"
                :disabled="!dirty || saving"
              >
                {{ saving ? "保存中…" : dirty ? "保存" : "已保存" }}
              </button>
            </div>
          </div>
          <div class="ed-fullpath" :title="selectedMeta.sub">
            {{ selectedMeta.sub }}
          </div>

          <!-- 板块⑫: 人格条 —— 仅项目可套用预设人格 + 绑定知识库 scope -->
          <div v-if="selected.kind === 'project'" class="persona-bar">
            <div class="pb-left">
              <span class="pb-icon">{{ currentPersona?.icon ?? "🧩" }}</span>
              <span class="pb-name">{{ currentPersona?.name ?? "自定义人格" }}</span>
            </div>
            <div class="pb-scope">
              <label>知识库范围</label>
              <input
                v-model="kbScope"
                placeholder="raw/子目录 (空=全库)"
                @keydown.enter="saveScope"
              />
              <button class="btn ghost mini" @click="saveScope">绑定</button>
            </div>
            <button class="btn primary mini" @click="showGallery = !showGallery">
              {{ showGallery ? "收起" : "选择人格" }}
            </button>
          </div>

          <!-- 内联覆盖确认（替代原生 confirm，避免模态卡死） -->
          <div v-if="pendingOverwrite" class="ow-bar">
            <span>「{{ pendingOverwrite.name }}」会覆盖当前项目的人格内容，确认？</span>
            <div class="ow-actions">
              <button class="btn primary mini" @click="doApply(pendingOverwrite, true)">确认覆盖</button>
              <button class="btn ghost mini" @click="pendingOverwrite = null">取消</button>
            </div>
          </div>

          <!-- 预设人格画廊（仿 WeSight 右侧选人格） -->
          <div v-if="selected.kind === 'project' && showGallery" class="gallery">
            <button
              v-for="ps in presets"
              :key="ps.id"
              class="p-card"
              :class="{ on: currentPersona?.id === ps.id }"
              :disabled="applying"
              @click="applyPreset(ps)"
              :title="ps.description"
            >
              <span class="pc-icon">{{ ps.icon }}</span>
              <span class="pc-name">{{ ps.name }}</span>
              <span class="pc-desc">{{ ps.description }}</span>
              <span v-if="ps.kbScope" class="pc-scope">📚 {{ ps.kbScope }}</span>
            </button>
          </div>

          <div v-if="message" class="msg" :class="message.kind">
            {{ message.text }}
          </div>
          <textarea
            v-model="content"
            class="ed-area-input"
            spellcheck="false"
            placeholder="编辑 CLAUDE.md…"
          ></textarea>
        </template>
      </section>
    </div>
  </div>
</template>

<style scoped>
.cmd-root {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
  background: var(--bg);
}

.cmd-head {
  padding: 14px 18px 10px;
  border-bottom: 1px solid var(--border-soft);
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
.cmd-head .title {
  font-family: var(--serif);
  font-size: 16px;
  letter-spacing: 2px;
  color: var(--ink);
}
.cmd-head .sub {
  font-size: 12px;
  color: var(--muted);
  margin-top: 4px;
}
.cmd-head .sub code {
  font-size: 11.5px;
  background: var(--selection-bg);
  padding: 1px 5px;
  border-radius: 3px;
}

.cmd-body {
  flex: 1;
  display: grid;
  grid-template-columns: 280px 1fr;
  min-height: 0;
}

.list {
  border-right: 1px solid var(--border-soft);
  overflow-y: auto;
  padding: 6px 4px;
}
.grp-head {
  font-family: var(--serif);
  font-size: 11px;
  letter-spacing: 1.5px;
  color: var(--dim);
  padding: 12px 10px 4px;
}
.item {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 7px 10px;
  border: none;
  border-radius: 3px;
  background: transparent;
  color: var(--text-2);
  font-size: 13px;
  text-align: left;
}
.item:hover {
  background: var(--selection-bg);
}
.item.active {
  background: var(--selection-bg);
  color: var(--text);
  font-weight: 500;
  border-left: 2px solid var(--ink);
  padding-left: 8px;
}
.dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--border);
  flex-shrink: 0;
}
.dot.on {
  background: var(--primary);
}
.rel {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.badge {
  font-size: 10px;
  padding: 1px 6px;
  border-radius: 2px;
  background: var(--border);
  color: var(--muted);
  font-family: var(--serif);
  letter-spacing: 1px;
}
.badge.on {
  background: var(--ink);
  color: #fff;
}
.badge.miss {
  background: transparent;
  border: 1px dashed var(--border);
  color: var(--dim);
}

.empty {
  font-size: 12px;
  color: var(--dim);
  padding: 12px;
  font-style: italic;
}

.editor {
  display: flex;
  flex-direction: column;
  min-height: 0;
}
.placeholder {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--muted);
  font-family: var(--serif);
  letter-spacing: 2px;
}
.ed-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 18px 6px;
}
.ed-path {
  display: flex;
  gap: 10px;
  align-items: baseline;
  flex: 1;
  min-width: 0;
}
.ed-area {
  font-family: var(--serif);
  font-size: 11px;
  letter-spacing: 1.5px;
  color: var(--dim);
}
.ed-rel {
  font-size: 14px;
  color: var(--text);
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ed-fullpath {
  padding: 0 18px 10px;
  font-size: 11px;
  color: var(--dim);
  font-family: ui-monospace, Consolas, monospace;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  border-bottom: 1px solid var(--border-soft);
}
.ed-actions {
  display: flex;
  gap: 6px;
}

.btn {
  padding: 5px 12px;
  border-radius: 3px;
  font-size: 12px;
  border: 1px solid var(--border);
  background: var(--panel);
  color: var(--text);
  cursor: pointer;
}
.btn:hover {
  background: var(--selection-bg);
}
.btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}
.btn.ghost {
  background: transparent;
}
.btn.primary {
  background: var(--ink);
  border-color: var(--ink);
  color: #fff;
}
.btn.primary:hover {
  background: var(--primary);
  border-color: var(--primary);
}

.msg {
  padding: 6px 18px;
  font-size: 12px;
  border-bottom: 1px solid var(--border-soft);
}
.msg.ok {
  color: var(--primary);
  background: var(--selection-bg);
}
.msg.err {
  color: var(--vermilion);
  background: var(--selection-bg);
}

.ed-area-input {
  flex: 1;
  border: none;
  outline: none;
  resize: none;
  padding: 14px 18px;
  font-family: ui-monospace, "JetBrains Mono", Consolas, monospace;
  font-size: 13px;
  line-height: 1.65;
  background: var(--panel);
  color: var(--text);
  tab-size: 2;
}

/* 板块⑫ 人格条 + 预设画廊 */
.btn.mini {
  padding: 3px 10px;
  font-size: 11.5px;
}
.ow-bar {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 9px 18px;
  background: rgba(248, 180, 80, 0.14);
  border-bottom: 1px solid var(--border-soft);
  font-size: 12.5px;
  color: var(--text);
}
.ow-bar span {
  flex: 1;
}
.ow-actions {
  display: flex;
  gap: 6px;
}
.persona-bar {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 8px 18px;
  border-bottom: 1px solid var(--border-soft);
  background: var(--bg-soft);
}
.pb-left {
  display: flex;
  align-items: center;
  gap: 7px;
}
.pb-icon {
  font-size: 18px;
}
.pb-name {
  font-size: 13px;
  font-weight: 500;
  color: var(--text);
}
.pb-scope {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-left: auto;
}
.pb-scope label {
  font-size: 11px;
  color: var(--dim);
  font-family: var(--serif);
  letter-spacing: 1px;
}
.pb-scope input {
  width: 160px;
  padding: 4px 8px;
  border: 1px solid var(--border);
  border-radius: 4px;
  font-size: 12px;
  background: var(--panel);
  color: var(--text);
}
.pb-scope input:focus {
  outline: none;
  border-color: var(--primary);
}
.gallery {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(170px, 1fr));
  gap: 10px;
  padding: 14px 18px;
  border-bottom: 1px solid var(--border-soft);
  overflow-y: auto;
  max-height: 280px;
  background: var(--bg-soft);
}
.p-card {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 12px;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--panel);
  text-align: left;
  cursor: pointer;
  transition: border-color 0.13s, transform 0.13s, box-shadow 0.13s;
}
.p-card:hover {
  border-color: var(--primary);
  transform: translateY(-2px);
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.1);
}
.p-card:disabled {
  opacity: 0.5;
  cursor: wait;
}
.p-card.on {
  border-color: var(--ink);
  box-shadow: 0 0 0 1px var(--ink) inset;
}
.pc-icon {
  font-size: 22px;
}
.pc-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
}
.pc-desc {
  font-size: 11px;
  color: var(--muted);
  line-height: 1.45;
}
.pc-scope {
  font-size: 10.5px;
  color: var(--dim);
  margin-top: 2px;
}
</style>
