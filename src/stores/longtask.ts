import { defineStore } from "pinia";
import { ref } from "vue";
import { chat as chatApi, type BuildManifest, type PermissionMode } from "../tauri";
import { useChatStore } from "./chat";

/**
 * 分批长任务编排（Long Task / Batch Build）—— 板块⑨配套
 * ──────────────────────────────────────────────────────────────
 * 问题：一次性让 Claude 生成超长产出（典型 60 页 PPT）会在**单轮**里连续吐几万 token，
 * 流式连接跑太久被掐 → `socket closed` / 进程坏死（exit 1），且崩了就全丢。
 *
 * 解法：把活儿拆成「先规划成清单 → 每轮只建一小批 → 落盘 checkpoint」。后端 `batch_build`
 * 协议让模型把计划写进 `polaris.build.json`、每轮只建 ≤K 个 pending 单元并回写状态；本 store
 * 负责把多轮串起来：发一轮 → 等 done → 读清单 → 还有 pending 就再发一轮，直到清单清空。
 * 因为每轮都以「done」收尾（即便崩了 done 也会发），崩溃的那轮等于零进展，下一轮读清单从
 * 下一个 pending 接着干 —— 断线续跑是天然的，不丢已落盘的批次。
 */

export interface BatchOpts {
  permissionMode: PermissionMode;
  skillIds: string[];
  useKb?: boolean;
}

/** 长任务启发式判定：「N 页/张/章/…」且 N ≥ 阈值，单轮就该拆批。 */
export function detectLongTask(prompt: string): boolean {
  const m = prompt.match(/(\d{1,3})\s*(页|张|slides?|节|章|条目?|篇|个\s*(?:幻灯片|页面|板块))/i);
  if (m && parseInt(m[1], 10) >= 12) return true;
  return false;
}

/** 据本轮注入的 input token 自适应批量：context 越重，每批越小，让单轮输出更稳。 */
function batchSizeFor(tokens: number): number {
  if (tokens > 60_000) return 4;
  if (tokens > 30_000) return 6;
  return 8;
}

function pendingCount(m: BuildManifest | null): number {
  if (!m || !Array.isArray(m.units)) return -1; // -1 = 没清单/读不到
  return m.units.filter((u) => u.status !== "done").length;
}
function doneCount(m: BuildManifest | null): number {
  if (!m || !Array.isArray(m.units)) return 0;
  return m.units.filter((u) => u.status === "done").length;
}

const CONTINUE_PROMPT =
  "继续构建下一批。先 Read 工作目录里的 `polaris.build.json`，按清单取下一批 pending 单元构建、" +
  "回写状态、增量落盘；本批做完即停，末尾报进度。若全部 done 则做收尾并写 `BUILD COMPLETE`。";

// 单条长任务最多驱动多少轮，避免清单异常时无限循环（60 页 / 每批 4~8 ≈ 8~15 轮，给足余量）。
const MAX_TURNS = 40;

export const useLongTaskStore = defineStore("longTask", () => {
  // 正在跑分批的对话 id → 进度文案（缩小版进度条用）。
  const running = ref<Record<string, string>>({});

  function isRunning(convId: string | null): boolean {
    return !!(convId && running.value[convId] != null);
  }
  function progressText(convId: string | null): string {
    if (!convId) return "";
    return running.value[convId] ?? "";
  }
  function stop(convId: string) {
    delete running.value[convId];
  }
  function note(convId: string, text: string) {
    useChatStore().pushBubble(convId, { role: "assistant", text });
  }

  async function safeManifest(convId: string): Promise<BuildManifest | null> {
    try {
      return await chatApi.buildManifest(convId);
    } catch {
      return null;
    }
  }

  /**
   * 跑一个分批长任务。turn0 规划 + 第一批，之后循环续批直到清单清空 / 停滞 / 触顶。
   * 与普通 send 的区别：注入 batch_build 协议、自适应批量、逐轮读清单决定续不续。
   */
  async function runBatchBuild(
    convId: string,
    prompt: string,
    display: string,
    opts: BatchOpts
  ): Promise<void> {
    const chat = useChatStore();
    running.value[convId] = "规划中…";

    const baseOpts = {
      permissionMode: opts.permissionMode,
      skillIds: opts.skillIds,
      useKb: opts.useKb,
      batchBuild: true,
    };

    // ── 轮 0：规划 + 第一批 ──
    await chat.send(convId, prompt, display, undefined, {
      ...baseOpts,
      batchSize: batchSizeFor(chat.inputTokens(convId)),
    });
    await chat.waitForDone(convId);
    if (!isRunning(convId)) return; // 被用户中途停掉

    let manifest = await safeManifest(convId);
    let prevPending = pendingCount(manifest);
    let stalls = 0;

    // ── 续批循环 ──
    for (let turn = 0; turn < MAX_TURNS; turn++) {
      if (!isRunning(convId)) return;
      const pending = pendingCount(manifest);
      const total = manifest?.units?.length ?? 0;

      if (manifest && total > 0 && pending === 0) break; // 全部 done

      // 停滞检测：没清单(-1) 或本轮 pending 没减少 → 可能崩在很早 / 卡住。容忍重试，连续 3 次才放弃。
      if (pending < 0 || pending >= prevPending) {
        stalls++;
        if (stalls >= 3) {
          note(
            convId,
            pending < 0
              ? "未能产出构建清单 polaris.build.json，已停止分批。可重发或改用普通模式。"
              : `连续多轮无进展，已停止分批（剩 ${pending} 个待建）。可重发让它从断点续跑。`
          );
          stop(convId);
          return;
        }
      } else {
        stalls = 0;
      }
      prevPending = pending < 0 ? prevPending : pending;

      const done = doneCount(manifest);
      running.value[convId] = total > 0 ? `已建 ${done}/${total}，续下一批…` : "续跑中…";

      await chat.send(convId, CONTINUE_PROMPT, `（继续构建 · 第 ${turn + 2} 批）`, undefined, {
        ...baseOpts,
        batchSize: batchSizeFor(chat.inputTokens(convId)),
      });
      await chat.waitForDone(convId);
      manifest = await safeManifest(convId);
    }

    // ── 收尾 ──
    const total = manifest?.units?.length ?? 0;
    const done = doneCount(manifest);
    if (total > 0 && done >= total) {
      note(convId, `✅ 分批长任务完成：共 ${total} 个单元全部生成（产物在右侧「参考资料」/「项目」可预览）。`);
    } else if (total > 0) {
      note(convId, `分批已结束：${done}/${total} 个单元完成。重发可从断点继续。`);
    }
    stop(convId);
  }

  return { running, isRunning, progressText, stop, runBatchBuild };
});
