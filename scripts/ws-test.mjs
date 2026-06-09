// 验证 WebSocket 流式：连 /ws，触发一轮 chat，统计收到的 delta/tool/done 帧。
const BASE = "http://localhost:8080";
const WS = "ws://localhost:8080/ws";

const ws = new WebSocket(WS);
let deltas = 0, tools = 0, done = false, firstText = "";
let myReq = null;

ws.onopen = async () => {
  console.log("WS 已连接，创建会话并发消息…");
  const inv = async (cmd, args) => {
    const r = await fetch(`${BASE}/api/invoke`, {
      method: "POST", headers: { "content-type": "application/json" },
      body: JSON.stringify({ cmd, args }),
    });
    if (!r.ok) throw new Error(`${cmd} -> HTTP ${r.status}`);
    const t = await r.text();
    return t ? JSON.parse(t) : null;
  };
  const proj = await inv("conv_create_project", { name: "ws-test" });
  const conv = await inv("conv_create_conversation", { projectId: proj.id });
  myReq = await inv("chat_send", {
    args: { prompt: "请用三句话介绍杭州。", permissionMode: "auto_all", conversationId: conv.id },
  });
  console.log("reqId =", myReq);
};

ws.onmessage = (e) => {
  const { topic, payload } = JSON.parse(e.data);
  if (topic !== "chat:stream") return;
  if (payload.reqId !== myReq) return;
  if (payload.kind === "delta") { deltas++; if (firstText.length < 60 && payload.text) firstText += payload.text; }
  else if (payload.kind === "tool") tools++;
  else if (payload.kind === "done") {
    done = true;
    console.log(`\n✅ WS 流式 OK: delta=${deltas} tool=${tools} done=true`);
    console.log("首段文本:", firstText.slice(0, 60));
    ws.close(); process.exit(0);
  } else if (payload.kind === "error") {
    console.log("error 帧:", (payload.text || "").slice(0, 120));
  }
};
ws.onerror = (e) => { console.error("WS 错误", e.message || e); process.exit(1); };

setTimeout(() => {
  if (!done) { console.error(`❌ 超时：delta=${deltas} tool=${tools}`); process.exit(1); }
}, 180000);
