<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import cytoscape, { type Core } from "cytoscape";
import { kb } from "../tauri";

const container = ref<HTMLDivElement | null>(null);
const stats = ref({ nodes: 0, edges: 0 });
const empty = ref(false);
let cy: Core | null = null;

const categoryColor: Record<string, string> = {
  default: "#2c4661",
  概念: "#2c4661",
  课程: "#a78c4f",
  人物: "#7a5fb0",
  事件: "#c0392b",
  方法: "#3a6d63",
};

onMounted(async () => {
  const g = await kb.graph();
  stats.value = { nodes: g.nodes.length, edges: g.edges.length };
  empty.value = g.nodes.length === 0;
  if (empty.value || !container.value) return;

  cy = cytoscape({
    container: container.value,
    elements: [
      ...g.nodes.map((n) => ({
        data: { id: n.id, label: n.title, cat: n.category || "default" },
      })),
      ...g.edges.map((e, i) => ({
        data: { id: `e${i}`, source: e.source, target: e.target },
      })),
    ],
    style: [
      {
        selector: "node",
        style: {
          "background-color": (ele: any) =>
            categoryColor[ele.data("cat")] || categoryColor.default,
          label: "data(label)",
          color: "#1a1a1c",
          "font-family": "Source Han Serif SC, serif",
          "font-size": 11,
          "text-valign": "bottom",
          "text-margin-y": 4,
          width: 18,
          height: 18,
          "border-width": 1,
          "border-color": "#ffffff",
        },
      },
      {
        selector: "edge",
        style: {
          width: 1,
          "line-color": "#d0cec5",
          "curve-style": "bezier",
          "target-arrow-shape": "triangle",
          "target-arrow-color": "#d0cec5",
          "arrow-scale": 0.7,
        },
      },
      {
        selector: "node:selected",
        style: {
          "border-width": 2,
          "border-color": "#c0392b",
          "background-color": "#c0392b",
        },
      },
    ],
    layout: { name: "cose", animate: false, padding: 30, idealEdgeLength: 90 } as any,
    wheelSensitivity: 0.2,
  });
});

onUnmounted(() => {
  if (cy) {
    cy.destroy();
    cy = null;
  }
});
</script>

<template>
  <div class="graph">
    <div class="head">
      <div class="title">知识图谱</div>
      <div class="stats">
        节点 <strong>{{ stats.nodes }}</strong> · 边 <strong>{{ stats.edges }}</strong>
      </div>
    </div>
    <div v-if="empty" class="empty">
      <div class="empty-glyph">◈</div>
      <div>当前 KB 没有可视化节点</div>
      <div class="empty-hint">
        在 KB 文件 frontmatter 写 <code>category: 概念</code> 和
        <code>[[wiki-link]]</code> 双链,刷新本页查看图谱
      </div>
    </div>
    <div v-else ref="container" class="cy"></div>
  </div>
</template>

<style scoped>
.graph {
  display: flex;
  flex-direction: column;
  height: 100vh;
  background: var(--bg);
}
.head {
  padding: 18px 28px;
  border-bottom: 1px solid var(--hairline);
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.title {
  font-family: var(--serif);
  font-size: 18px;
  letter-spacing: 2px;
  color: var(--ink);
}
.stats {
  font-size: 12px;
  color: var(--muted);
}
.stats strong {
  color: var(--ink);
  font-family: var(--mono);
}
.cy {
  flex: 1;
  background: var(--bg-soft);
}
.empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  color: var(--muted);
  font-family: var(--serif);
  letter-spacing: 1px;
  gap: 6px;
}
.empty-glyph {
  font-size: 56px;
  color: var(--border-strong);
  margin-bottom: 8px;
}
.empty-hint {
  font-family: var(--sans);
  font-size: 12px;
  color: var(--dim);
  max-width: 460px;
  text-align: center;
  letter-spacing: 0;
  margin-top: 8px;
  line-height: 1.7;
}
.empty-hint code {
  background: var(--code-bg);
  padding: 1px 6px;
  border-radius: 2px;
  font-family: var(--mono);
  font-size: 11px;
}
</style>
