<script setup lang="ts">
import { ref, onMounted, computed } from "vue";
import { marked } from "marked";
import { kb, type KbHit } from "../tauri";

type Tab = "overview" | "browse" | "manage";
const tab = ref<Tab>("browse");
const files = ref<string[]>([]);
const selected = ref<string | null>(null);
const markdown = ref("");
const rendered = computed(() => (markdown.value ? marked.parse(markdown.value) : ""));
const query = ref("");
const hits = ref<KbHit[]>([]);
const rootPath = ref("");
const scanned = ref<number | null>(null);
const ingestPath = ref("");
const ingestMsg = ref("");

onMounted(async () => {
  rootPath.value = await kb.root();
  await refreshList();
});

async function refreshList() {
  try {
    files.value = await kb.list(null);
  } catch (e: any) {
    files.value = [];
  }
}

async function openFile(p: string) {
  selected.value = p;
  try {
    markdown.value = await kb.read(p);
  } catch (e: any) {
    markdown.value = `_(读取失败:${e?.message ?? e})_`;
  }
}

async function doScan() {
  scanned.value = await kb.scan();
  await refreshList();
}

async function doSearch() {
  if (!query.value.trim()) {
    hits.value = [];
    return;
  }
  hits.value = await kb.search(query.value.trim());
}

async function doIngest() {
  if (!ingestPath.value.trim()) return;
  try {
    const r = await kb.ingest(ingestPath.value.trim());
    ingestMsg.value = `已 ingest → ${r}`;
    await refreshList();
  } catch (e: any) {
    ingestMsg.value = `失败:${e?.message ?? e}`;
  }
}
</script>

<template>
  <div class="wiki">
    <div class="head">
      <div class="title">维基知识库</div>
      <div class="tabs">
        <button
          v-for="t in [
            { k: 'overview', l: '概览' },
            { k: 'browse', l: '浏览' },
            { k: 'manage', l: '管理' },
          ]"
          :key="t.k"
          class="tab"
          :class="{ active: tab === t.k }"
          @click="tab = t.k as Tab"
        >
          {{ t.l }}
        </button>
      </div>
      <div class="root">
        <span class="root-label">KB 根:</span>
        <code>{{ rootPath }}</code>
      </div>
    </div>

    <div v-if="tab === 'overview'" class="body overview">
      <div class="cards">
        <div class="card">
          <div class="card-title">三层目录铁律</div>
          <div class="card-body">
            <code>raw/</code> 只读原始 · <code>output/</code> 撰文 + Lint ·
            <code>wiki/</code> 知识层
          </div>
        </div>
        <div class="card">
          <div class="card-title">KB-first 召回</div>
          <div class="card-body">
            每次发消息前自动 <code>kb_search</code>,关键词加权评分,Top-N
            注入 system prompt
          </div>
        </div>
        <div class="card">
          <div class="card-title">6 模式</div>
          <div class="card-body">
            查询(严/普)· 拆解课件 · Ingest · 撰文 · Lint;
            v0.1 仅启用「普通查询 + ingest」
          </div>
        </div>
      </div>
      <button class="primary-btn" @click="doScan()">扫描索引</button>
      <span v-if="scanned !== null" class="muted">扫描完成,共 {{ scanned }} 个文件</span>
    </div>

    <div v-if="tab === 'browse'" class="body browse">
      <div class="left">
        <div class="search-row">
          <input
            v-model="query"
            placeholder="搜索 KB(标题/正文)"
            @keydown.enter="doSearch"
          />
          <button class="btn" @click="doSearch">搜</button>
        </div>
        <div v-if="hits.length" class="hit-list">
          <div class="section-title">搜索结果</div>
          <div
            v-for="h in hits"
            :key="h.path"
            class="hit"
            @click="openFile(h.path)"
          >
            <div class="hit-title">{{ h.title }}</div>
            <div class="hit-snip">{{ h.snippet }}</div>
            <div class="hit-meta">score {{ h.score.toFixed(1) }} · {{ h.path }}</div>
          </div>
        </div>
        <div class="section-title">所有文件</div>
        <div
          v-for="f in files"
          :key="f"
          class="file"
          :class="{ active: selected === f }"
          @click="openFile(f)"
        >
          {{ f }}
        </div>
        <div v-if="files.length === 0" class="muted empty">
          KB 为空,可在「管理」tab ingest 文件,或访问根目录手动放 .md
        </div>
      </div>
      <div class="right">
        <div v-if="!selected" class="placeholder">
          <div class="ph-glyph">▥</div>
          <div>选择左侧文件浏览</div>
        </div>
        <div v-else class="md" v-html="rendered"></div>
      </div>
    </div>

    <div v-if="tab === 'manage'" class="body manage">
      <div class="card">
        <div class="card-title">Ingest 文件 → KB</div>
        <div class="card-body">
          填入本机文件绝对路径,自动复制到 <code>raw/</code> 并建立索引(MVP:仅支持 .md / .txt)
        </div>
        <div class="ingest-row">
          <input v-model="ingestPath" placeholder="例:D:\polaris\案例文件夹\01_xxx.md" />
          <button class="primary-btn" @click="doIngest">Ingest</button>
        </div>
        <div v-if="ingestMsg" class="ingest-msg">{{ ingestMsg }}</div>
      </div>
      <div class="card">
        <div class="card-title">索引重建</div>
        <div class="card-body">
          扫描 KB 根下所有 .md 文件,构建内存索引(MVP 不持久化,启动后自动扫描)
        </div>
        <button class="primary-btn" @click="doScan">立即扫描</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.wiki {
  display: flex;
  flex-direction: column;
  height: 100vh;
}
.head {
  padding: 18px 28px 0;
  border-bottom: 1px solid var(--hairline);
}
.title {
  font-family: var(--serif);
  font-size: 18px;
  letter-spacing: 2px;
  color: var(--ink);
}
.tabs {
  margin-top: 14px;
  display: flex;
  gap: 18px;
}
.tab {
  background: transparent;
  border: none;
  padding: 8px 0;
  color: var(--muted);
  font-size: 13px;
}
.tab.active {
  color: var(--text);
  font-weight: 600;
  border-bottom: 2px solid var(--ink);
}
.root {
  margin-top: 8px;
  font-size: 11px;
  color: var(--muted);
  padding-bottom: 8px;
}
.root-label {
  margin-right: 6px;
}
.root code {
  background: var(--code-bg);
  padding: 1px 6px;
  border-radius: 2px;
  font-family: var(--mono);
}

.body {
  flex: 1;
  overflow: hidden;
  padding: 18px 28px;
}
.body.overview {
  display: flex;
  flex-direction: column;
  gap: 18px;
}
.body.browse {
  display: grid;
  grid-template-columns: 320px 1fr;
  gap: 16px;
  height: calc(100vh - 130px);
}
.body.manage {
  display: flex;
  flex-direction: column;
  gap: 18px;
}

.cards {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 14px;
}
.card {
  background: var(--panel);
  border: 1px solid var(--hairline);
  border-radius: 4px;
  padding: 16px 18px;
}
.card-title {
  font-family: var(--serif);
  font-weight: 600;
  font-size: 13.5px;
  color: var(--text);
  margin-bottom: 6px;
}
.card-body {
  font-size: 12.5px;
  color: var(--text-2);
  line-height: 1.7;
}

.primary-btn {
  align-self: flex-start;
  padding: 7px 16px;
  background: var(--ink);
  color: #fafaf7;
  border: none;
  border-radius: 4px;
  font-size: 12.5px;
}
.primary-btn:hover {
  background: var(--primary);
}
.muted {
  color: var(--muted);
  font-size: 12px;
}

.left {
  border: 1px solid var(--hairline);
  border-radius: 4px;
  padding: 10px;
  overflow-y: auto;
  background: var(--panel);
}
.right {
  border: 1px solid var(--hairline);
  border-radius: 4px;
  padding: 22px 28px;
  overflow-y: auto;
  background: var(--panel);
}
.search-row {
  display: flex;
  gap: 6px;
  margin-bottom: 10px;
}
.search-row input {
  flex: 1;
  padding: 6px 8px;
  border: 1px solid var(--border);
  border-radius: 3px;
  font-size: 12.5px;
  background: var(--bg);
}
.search-row input:focus {
  outline: none;
  border-color: var(--primary);
}
.btn {
  padding: 6px 12px;
  border: 1px solid var(--border);
  background: var(--panel);
  border-radius: 3px;
  font-size: 12.5px;
}
.btn:hover {
  border-color: var(--primary);
}

.section-title {
  font-family: var(--serif);
  font-size: 11px;
  letter-spacing: 1.5px;
  color: var(--dim);
  padding: 8px 4px 4px;
}
.hit-list {
  margin-bottom: 10px;
}
.hit {
  padding: 8px 10px;
  border-radius: 3px;
  cursor: pointer;
  margin-bottom: 2px;
}
.hit:hover {
  background: var(--selection-bg);
}
.hit-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
}
.hit-snip {
  font-size: 11.5px;
  color: var(--muted);
  margin-top: 2px;
  line-height: 1.5;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.hit-meta {
  font-size: 10.5px;
  color: var(--dim);
  margin-top: 2px;
  font-family: var(--mono);
}

.file {
  padding: 5px 10px;
  font-size: 12.5px;
  color: var(--text-2);
  border-radius: 3px;
  cursor: pointer;
  font-family: var(--mono);
}
.file:hover {
  background: var(--selection-bg);
  color: var(--text);
}
.file.active {
  background: var(--selection-bg);
  color: var(--ink);
  font-weight: 500;
}

.placeholder {
  height: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  color: var(--dim);
  font-family: var(--serif);
  letter-spacing: 1px;
}
.ph-glyph {
  font-size: 40px;
  margin-bottom: 12px;
  color: var(--border-strong);
}

.md {
  font-size: 13.5px;
  line-height: 1.85;
  color: var(--text);
}
.md :deep(h1),
.md :deep(h2),
.md :deep(h3) {
  font-family: var(--serif);
  letter-spacing: 1px;
}
.md :deep(h1) {
  font-size: 22px;
  margin-top: 0;
}
.md :deep(h2) {
  font-size: 17px;
  border-bottom: 1px solid var(--hairline);
  padding-bottom: 6px;
}
.md :deep(code) {
  background: var(--code-bg);
  padding: 1.5px 6px;
  border-radius: 2px;
  font-family: var(--mono);
  font-size: 12px;
}
.md :deep(pre) {
  background: var(--bg-soft);
  border: 1px solid var(--hairline);
  padding: 14px 16px;
  border-radius: 3px;
  overflow-x: auto;
}
.md :deep(blockquote) {
  border-left: 2px solid var(--ink);
  padding-left: 14px;
  color: var(--text-2);
  margin-left: 0;
}
.md :deep(a) {
  color: var(--primary);
}

.ingest-row {
  display: flex;
  gap: 6px;
  margin-top: 12px;
}
.ingest-row input {
  flex: 1;
  padding: 7px 10px;
  border: 1px solid var(--border);
  border-radius: 3px;
  font-size: 12.5px;
  background: var(--bg);
  font-family: var(--mono);
}
.ingest-msg {
  margin-top: 8px;
  font-size: 12px;
  color: var(--muted);
}
.empty {
  padding: 20px 8px;
  font-style: italic;
}
</style>
