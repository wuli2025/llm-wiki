import { defineStore } from "pinia";
import { ref, computed } from "vue";
import { project as api, listen, type ProjectInfo } from "../tauri";

/**
 * 板块⑮「可运行项目」前端状态。
 *
 * 模型把要跑起来的应用打包成带 `polaris.project.json` 的项目文件夹后，这里负责：
 * - 列出本对话产出的可运行项目（refresh）
 * - 一键运行（run）：后端装依赖 + 起前后端，进度走 project:log / project:ready / project:exit 事件
 * - 端口起来后内嵌 iframe 预览（previewUrl + ready）
 * - 停止（stop）：kill 整个进程树
 *
 * RightDrawer 据 activeRoot 切到「运行预览」全屏态：iframe 看应用 + 日志台 + 停止/重载。
 */
interface LogLine {
  /** info(应用旁白) | stdout | stderr */
  stream: string;
  line: string;
}

export const useProjectsStore = defineStore("projects", () => {
  const list = ref<ProjectInfo[]>([]);
  const loading = ref(false);

  // 当前正在「运行预览」的项目 root（null = 没在预览，抽屉回到列表态）
  const activeRoot = ref<string | null>(null);
  const previewUrl = ref<string | null>(null);
  const ready = ref(false); // 端口起来了没 —— 决定 iframe 显不显示
  const starting = ref(false); // 正在装依赖 / 起服务
  const logs = ref<LogLine[]>([]);
  const logsOpen = ref(true);
  const frameNonce = ref(0); // iframe 刷新 key

  const active = computed(
    () => list.value.find((p) => p.root === activeRoot.value) ?? null
  );
  const hasRunning = computed(() => list.value.some((p) => p.running));

  function markRunning(root: string, v: boolean) {
    const p = list.value.find((x) => x.root === root);
    if (p) p.running = v;
  }
  function pushLog(stream: string, line: string) {
    logs.value.push({ stream, line });
    if (logs.value.length > 1200) logs.value.splice(0, logs.value.length - 1200);
  }

  let bound = false;
  const unlisteners: Array<() => void> = [];
  async function bind() {
    if (bound) return;
    bound = true;
    unlisteners.push(
      await listen<{ root: string; stream: string; line: string }>(
        "project:log",
        (e) => {
          if (e.root !== activeRoot.value) return;
          pushLog(e.stream, e.line);
        }
      )
    );
    unlisteners.push(
      await listen<{ root: string; open?: string | null }>(
        "project:ready",
        (e) => {
          markRunning(e.root, true);
          if (e.root !== activeRoot.value) return;
          starting.value = false;
          ready.value = true;
          if (e.open) previewUrl.value = e.open;
          frameNonce.value++;
        }
      )
    );
    unlisteners.push(
      await listen<{ root: string; ok: boolean; message?: string }>(
        "project:exit",
        (e) => {
          markRunning(e.root, false);
          if (e.root !== activeRoot.value) return;
          if (e.message) pushLog("info", e.message);
          ready.value = false;
          starting.value = false;
        }
      )
    );
  }

  async function refresh(convId?: string) {
    loading.value = true;
    try {
      list.value = await api.list(convId);
    } catch {
      list.value = [];
    } finally {
      loading.value = false;
    }
  }

  /** 一键运行（或为已在跑的项目重新打开预览：后端 run 命中已运行会直接回 ready）。 */
  async function run(p: ProjectInfo) {
    await bind();
    activeRoot.value = p.root;
    previewUrl.value = p.open ?? null;
    ready.value = false;
    starting.value = true;
    logsOpen.value = true;
    logs.value = [{ stream: "info", line: `▶ 启动项目「${p.name}」` }];
    markRunning(p.root, true);
    try {
      await api.run(p.root);
    } catch (e: any) {
      starting.value = false;
      markRunning(p.root, false);
      pushLog("stderr", e?.message ?? String(e));
    }
  }

  async function stop(root: string) {
    try {
      await api.stop(root);
    } catch {
      /* 忽略：停止失败不致命 */
    }
    markRunning(root, false);
    if (root === activeRoot.value) {
      ready.value = false;
      starting.value = false;
    }
  }

  /** 关闭运行预览，回到项目列表（不停止进程，可后续再打开）。 */
  function closePreview() {
    activeRoot.value = null;
    previewUrl.value = null;
    ready.value = false;
    starting.value = false;
    logs.value = [];
  }

  function reloadFrame() {
    frameNonce.value++;
  }
  function toggleLogs() {
    logsOpen.value = !logsOpen.value;
  }

  return {
    list,
    loading,
    activeRoot,
    previewUrl,
    ready,
    starting,
    logs,
    logsOpen,
    frameNonce,
    active,
    hasRunning,
    refresh,
    run,
    stop,
    closePreview,
    reloadFrame,
    toggleLogs,
  };
});
