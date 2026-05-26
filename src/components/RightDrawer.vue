<script setup lang="ts">
import { ref, computed } from "vue";
import { marked } from "marked";
import {
  X,
  RefreshCw,
  ExternalLink,
  Globe,
  Maximize2,
  Minimize2,
  FileCode,
  FileText,
  File as FileIcon,
  Image as ImageIcon,
  Loader,
} from "@lucide/vue";
import { useAppStore } from "../stores/app";
import { useArtifactsStore } from "../stores/artifacts";

const app = useAppStore();
const artifacts = useArtifactsStore();
const activeTab = ref<"artifacts" | "ref" | "audit">("artifacts");

const headIcon = computed(() => {
  const k = artifacts.payload?.kind;
  if (k === "html" || k === "svg") return FileCode;
  if (k === "image") return ImageIcon;
  if (k === "markdown" || k === "text") return FileText;
  return FileIcon;
});

const renderedMd = computed(() => {
  const p = artifacts.payload;
  if (p?.kind === "markdown" && p.text) {
    return marked.parse(p.text) as string;
  }
  return "";
});

function fmtSize(n: number): string {
  if (!n) return "";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}
</script>

<template>
  <aside
    class="dr"
    :class="{
      collapsed: app.drawerCollapsed && !artifacts.current,
      preview: !!artifacts.current,
    }"
  >
    <!-- ───────── 成品预览模式 ───────── -->
    <template v-if="artifacts.current">
      <div class="pv-head">
        <component :is="headIcon" :size="15" :stroke-width="1.7" class="pv-ficon" />
        <span class="pv-name" :title="artifacts.current.path">
          {{ artifacts.current.name }}
        </span>
        <span v-if="artifacts.payload" class="pv-size">
          {{ fmtSize(artifacts.payload.size) }}
        </span>
        <div class="pv-actions">
          <button class="pv-btn" title="刷新" @click="artifacts.refresh()">
            <RefreshCw :size="14" :stroke-width="1.8" />
          </button>
          <button
            class="pv-btn"
            :title="artifacts.expanded ? '收起' : '放大'"
            @click="artifacts.toggleExpand()"
          >
            <component
              :is="artifacts.expanded ? Minimize2 : Maximize2"
              :size="14"
              :stroke-width="1.8"
            />
          </button>
          <button
            class="pv-btn"
            title="用默认浏览器打开"
            @click="artifacts.openExternal()"
          >
            <Globe :size="15" :stroke-width="1.8" />
          </button>
          <button class="pv-btn" title="关闭预览" @click="artifacts.close()">
            <X :size="15" :stroke-width="2" />
          </button>
        </div>
      </div>

      <div class="pv-body">
        <div v-if="artifacts.loading" class="pv-state">
          <Loader :size="22" :stroke-width="1.6" class="spin" />
          <span>正在加载…</span>
        </div>
        <div v-else-if="artifacts.error" class="pv-state err">
          <span>{{ artifacts.error }}</span>
          <button class="pv-open-ext" @click="artifacts.openExternal()">
            <ExternalLink :size="14" :stroke-width="1.8" />
            <span>用系统程序打开</span>
          </button>
        </div>

        <template v-else-if="artifacts.payload">
          <!-- HTML / SVG → iframe 完整渲染 -->
          <iframe
            v-if="
              artifacts.payload.kind === 'html' ||
              artifacts.payload.kind === 'svg'
            "
            :key="artifacts.payload.path"
            class="pv-frame"
            :srcdoc="artifacts.payload.text"
            sandbox="allow-scripts allow-popups allow-forms allow-modals allow-pointer-lock allow-downloads"
            referrerpolicy="no-referrer"
          />
          <!-- 图片 -->
          <div
            v-else-if="artifacts.payload.kind === 'image'"
            class="pv-img-wrap"
          >
            <img :src="artifacts.payload.dataUrl" :alt="artifacts.payload.name" />
          </div>
          <!-- Markdown → 渲染 -->
          <div
            v-else-if="artifacts.payload.kind === 'markdown'"
            class="pv-md markdown"
            v-html="renderedMd"
          />
          <!-- 纯文本 / 代码 -->
          <pre
            v-else-if="artifacts.payload.kind === 'text'"
            class="pv-code"
          ><code>{{ artifacts.payload.text }}</code></pre>
          <!-- 其它二进制 -->
          <div v-else class="pv-state">
            <FileIcon :size="26" :stroke-width="1.4" />
            <span>该文件类型暂不支持内嵌预览</span>
            <button class="pv-open-ext" @click="artifacts.openExternal()">
              <ExternalLink :size="14" :stroke-width="1.8" />
              <span>用系统程序打开</span>
            </button>
          </div>
        </template>
      </div>
    </template>

    <!-- ───────── 默认抽屉模式 ───────── -->
    <template v-else>
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
                生成 HTML / 报告 / 图片等成品后,会在对话里出现可点击的文件,点开即在此预览
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

/* ───────── 预览头 ───────── */
.pv-head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 12px;
  border-bottom: 1px solid var(--border-soft);
  background: var(--bg);
}
.pv-ficon {
  color: var(--primary);
  flex-shrink: 0;
}
.pv-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.pv-size {
  font-size: 11px;
  color: var(--muted);
  flex-shrink: 0;
}
.pv-actions {
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: 2px;
  flex-shrink: 0;
}
.pv-btn {
  width: 28px;
  height: 28px;
  border: none;
  background: transparent;
  color: var(--muted);
  border-radius: 6px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}
.pv-btn:hover {
  background: var(--bg-soft);
  color: var(--primary);
}

/* ───────── 预览体 ───────── */
.pv-body {
  flex: 1;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  background: #fff;
}
.pv-frame {
  flex: 1;
  width: 100%;
  height: 100%;
  border: none;
  background: #fff;
}
.pv-img-wrap {
  flex: 1;
  overflow: auto;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
  background:
    repeating-conic-gradient(#f4f4f0 0% 25%, #ffffff 0% 50%) 50% / 20px 20px;
}
.pv-img-wrap img {
  max-width: 100%;
  height: auto;
  box-shadow: var(--shadow-sm);
}
.pv-md {
  flex: 1;
  overflow: auto;
  padding: 24px 28px;
  font-size: 14px;
  line-height: 1.7;
  color: var(--text);
}
.pv-code {
  flex: 1;
  overflow: auto;
  margin: 0;
  padding: 16px 18px;
  font-family: var(--mono);
  font-size: 12.5px;
  line-height: 1.6;
  color: var(--text);
  background: var(--bg-soft);
  white-space: pre;
  tab-size: 2;
}
.pv-state {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  color: var(--muted);
  font-size: 13px;
  padding: 40px 24px;
  text-align: center;
}
.pv-state.err {
  color: var(--vermilion);
}
.pv-open-ext {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 6px 12px;
  border: 1px solid var(--border);
  background: var(--panel);
  border-radius: 6px;
  color: var(--text-2);
  font-size: 12.5px;
  cursor: pointer;
}
.pv-open-ext:hover {
  border-color: var(--primary);
  color: var(--primary);
}
.spin {
  animation: pv-spin 0.9s linear infinite;
}
@keyframes pv-spin {
  to {
    transform: rotate(360deg);
  }
}

/* markdown 渲染基本排版 */
.markdown :deep(h1),
.markdown :deep(h2),
.markdown :deep(h3) {
  font-family: var(--serif);
  margin: 1.2em 0 0.5em;
  line-height: 1.3;
}
.markdown :deep(p) {
  margin: 0.6em 0;
}
.markdown :deep(pre) {
  background: var(--bg-soft);
  padding: 12px 14px;
  border-radius: 6px;
  overflow: auto;
  font-family: var(--mono);
  font-size: 12.5px;
}
.markdown :deep(code) {
  font-family: var(--mono);
  font-size: 0.9em;
}
.markdown :deep(:not(pre) > code) {
  background: var(--bg-soft);
  padding: 1px 5px;
  border-radius: 3px;
}
.markdown :deep(table) {
  border-collapse: collapse;
  margin: 0.8em 0;
}
.markdown :deep(th),
.markdown :deep(td) {
  border: 1px solid var(--border);
  padding: 6px 10px;
}
.markdown :deep(img) {
  max-width: 100%;
}
.markdown :deep(a) {
  color: var(--primary);
}
.markdown :deep(blockquote) {
  border-left: 3px solid var(--border-strong);
  margin: 0.8em 0;
  padding-left: 14px;
  color: var(--muted);
}

/* ───────── 默认抽屉样式（原样保留） ───────── */
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
