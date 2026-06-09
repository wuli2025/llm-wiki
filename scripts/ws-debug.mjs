// 调试：一个会话连发两条消息，打印第 2 条消息的所有 WS 帧（找出卡在哪）。
const BASE = "http://localhost:8080";
const inv = async (cmd, args) => {
  const r = await fetch(`${BASE}/api/invoke`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ cmd, args }) });
  const t = await r.text(); return t ? JSON.parse(t) : null;
};
const ws = new WebSocket("ws://localhost:8080/ws");
let phase = 0, myReq = null, cid = null;
const frames = [];

function watch(reqId, label, done) {
  myReq = reqId; console.log(`\n=== ${label} reqId=${reqId} ===`);
}

// 并发轮询 conv_get_messages（模拟 PowerShell/monitor 的轮询），验证是否致卡。
setInterval(() => {
  if (cid) inv("conv_get_messages", { conversationId: cid }).catch(() => {});
}, 2000);

ws.onopen = async () => {
  const proj = await inv("conv_create_project", { name: "ws-debug" });
  const conv = await inv("conv_create_conversation", { projectId: proj.id });
  cid = conv.id;
  console.log("发 msg1...");
  const r1 = await inv("chat_send", { args: { prompt: "请只回复:第一条OK", permissionMode: "auto_all", conversationId: cid } });
  watch(r1, "MSG1");
};

ws.onmessage = async (e) => {
  const { topic, payload } = JSON.parse(e.data);
  if (topic !== "chat:stream" || payload.reqId !== myReq) return;
  const snip = (payload.text || payload.tool || "").toString().slice(0, 80);
  console.log(`  [${payload.kind}] ${snip}`);
  if (payload.kind === "done") {
    phase++;
    if (phase <= 3) {
      console.log(`\nmsg${phase} 完成，发 msg${phase + 1}（带历史）...`);
      const r = await inv("chat_send", { args: { prompt: `请只回复:第${phase + 1}条OK`, permissionMode: "auto_all", conversationId: cid } });
      watch(r, `MSG${phase + 1}`);
    } else {
      console.log("\n✅ 全部 4 条消息完成"); process.exit(0);
    }
  }
};
ws.onerror = (e) => { console.error("WS err", e.message); process.exit(1); };
setTimeout(() => { console.log("\n⏱ 90s 到，msg2 未完成（卡住）"); process.exit(2); }, 90000);
