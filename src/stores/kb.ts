import { defineStore } from "pinia";
import { ref } from "vue";
import { kb, listen, type KbCompileEvent } from "../tauri";

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

  return {
    compiling,
    compileLog,
    compileMsg,
    compileRunId,
    lastDocCount,
    doneTick,
    ensureListener,
    startCompile,
  };
});
