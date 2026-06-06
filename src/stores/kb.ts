import { defineStore } from "pinia";
import { ref } from "vue";
import {
  kb,
  listen,
  type KbCompileEvent,
  type KbMaintainEvent,
  type KbLintReport,
} from "../tauri";

// 「构建知识网」全局状态。
// 后端 kb_compile 本就是独立线程 + 全局事件,离开 wiki 视图进程不会停;
// 之前进度锁在 WikiBrowse 组件里,组件一卸载就退订+清零,看起来像停了。
// 把状态+监听抬到这里 → 监听只注册一次、脱离任何组件生命周期,
// 切走切回甚至关掉 wiki 视图,进度都在后台继续累积,回来即见。
export const useKbStore = defineStore("kb", () => {
  const compiling = ref(false);
  const compileLog = ref<string[]>([]);
  const compileMsg = ref("");
  const compileRunId = ref("");
  // 编译后重扫的文档总数(done 时回填),供 WikiBrowse 更新计数
  const lastDocCount = ref<number | null>(null);
  // 每次编译完成自增 → WikiBrowse watch 它来刷新文件列表
  const doneTick = ref(0);

  let unlisten: (() => void) | null = null;

  // 全局只注册一次 kb:compile 监听
  async function ensureListener() {
    if (unlisten) return;
    unlisten = await listen<KbCompileEvent>("kb:compile", (ev) => {
      if (ev.runId !== compileRunId.value) return;
      const t = ev.text ?? "";
      if (ev.kind === "done") {
        compiling.value = false;
        compileMsg.value = t || "完成";
        if (typeof ev.docCount === "number") lastDocCount.value = ev.docCount;
        doneTick.value++;
        return;
      }
      const icon =
        ev.kind === "error"
          ? "⚠ "
          : ev.kind === "page"
            ? "📄 "
            : ev.kind === "phase"
              ? "▸ "
              : "· ";
      compileLog.value.push(icon + t);
      if (compileLog.value.length > 200)
        compileLog.value.splice(0, compileLog.value.length - 200);
    });
  }

  // 启动一次构建知识网。进行中重复调用直接忽略(后端进程仍在跑)。
  async function startCompile() {
    if (compiling.value) return;
    compiling.value = true;
    compileMsg.value = "";
    compileLog.value = [];
    lastDocCount.value = null;
    await ensureListener();
    try {
      compileRunId.value = await kb.compile();
    } catch (e: any) {
      compiling.value = false;
      compileMsg.value = "启动失败:" + (e?.message ?? e);
    }
  }

  // ── 维护知识网: 自动补双链 (enrich) / 智能去重 (dedup) ──
  // 借鉴 llm_wiki「AI 出决策、代码执行」。复用上面的进度日志 UI (同时只跑一个维护操作)。
  let unlistenMaintain: (() => void)[] = [];
  async function ensureMaintainListener() {
    if (unlistenMaintain.length) return;
    const handle = (ev: KbMaintainEvent) => {
      if (ev.runId !== compileRunId.value) return;
      const t = ev.text ?? "";
      if (ev.kind === "done") {
        compiling.value = false;
        compileMsg.value = t || "完成";
        doneTick.value++;
        return;
      }
      const icon =
        ev.kind === "error" ? "⚠ " : ev.kind === "phase" ? "▸ " : "· ";
      compileLog.value.push(icon + t);
      if (compileLog.value.length > 200)
        compileLog.value.splice(0, compileLog.value.length - 200);
    };
    unlistenMaintain.push(await listen<KbMaintainEvent>("kb:enrich", handle));
    unlistenMaintain.push(await listen<KbMaintainEvent>("kb:dedup", handle));
  }

  async function startMaintain(kind: "enrich" | "dedup") {
    if (compiling.value) return;
    compiling.value = true;
    compileMsg.value = "";
    compileLog.value = [kind === "enrich" ? "▸ 自动补双链…" : "▸ 智能去重…"];
    lastDocCount.value = null;
    await ensureMaintainListener();
    try {
      compileRunId.value =
        kind === "enrich" ? await kb.enrichLinks() : await kb.dedup();
    } catch (e: any) {
      compiling.value = false;
      compileMsg.value = "启动失败:" + (e?.message ?? e);
    }
  }

  // ── wiki 质量检查 (lint): 同步返回报告 ──
  const lintReport = ref<KbLintReport | null>(null);
  const linting = ref(false);
  async function runLint() {
    linting.value = true;
    try {
      lintReport.value = await kb.lint();
    } finally {
      linting.value = false;
    }
  }

  return {
    compiling,
    compileLog,
    compileMsg,
    compileRunId,
    lastDocCount,
    doneTick,
    ensureListener,
    startCompile,
    startMaintain,
    lintReport,
    linting,
    runLint,
  };
});
