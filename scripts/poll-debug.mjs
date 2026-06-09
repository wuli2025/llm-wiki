// 隔离测试：不连 WS，纯轮询检测完成，用与 ws-debug 完全相同的提示词与流程。
// 若 msg2 卡 → 确认「无 WS 订阅」是触发条件；若全过 → 之前是提示词差异。
const BASE = "http://localhost:8080";
const inv = async (cmd, args) => {
  const r = await fetch(`${BASE}/api/invoke`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ cmd, args }) });
  const t = await r.text(); return t ? JSON.parse(t) : null;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

(async () => {
  const proj = await inv("conv_create_project", { name: "poll-debug" });
  const conv = await inv("conv_create_conversation", { projectId: proj.id });
  const cid = conv.id;
  for (let n = 1; n <= 4; n++) {
    const before = (await inv("conv_get_messages", { conversationId: cid })).filter((m) => m.role === "assistant").length;
    const t0 = Date.now();
    await inv("chat_send", { args: { prompt: `请只回复:第${n}条OK`, permissionMode: "auto_all", conversationId: cid } });
    let ok = false;
    for (let i = 0; i < 40; i++) {
      await sleep(3000);
      const after = (await inv("conv_get_messages", { conversationId: cid })).filter((m) => m.role === "assistant").length;
      if (after > before) { ok = true; break; }
    }
    const dt = Math.round((Date.now() - t0) / 1000);
    console.log(`msg${n}: ${ok ? dt + "s ✅" : "TIMEOUT " + dt + "s ❌"}`);
    if (!ok) { console.log("→ 无 WS + 轮询 复现卡死"); process.exit(2); }
  }
  console.log("✅ 无 WS 也全部完成（说明之前是别的因素）");
})();
