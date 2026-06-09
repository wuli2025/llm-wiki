// Polaris 飞书对话引擎 · Node 桥
// 用飞书官方 SDK 的 WSClient 起「长连接」，收 im.message.receive_v1 事件 → 打到 stdout(JSON 行)；
// 从 stdin 读 {type:'reply'} 指令 → 调 im.message.create 把 Claude 的回复发回飞书。
// Rust 端(feishu.rs 网关)负责把消息路由给 headless claude 再把回复写回本进程 stdin。
import * as Lark from "@larksuiteoapi/node-sdk";

const appId = process.env.FEISHU_APP_ID || "";
const appSecret = process.env.FEISHU_APP_SECRET || "";
const isLark = (process.env.FEISHU_DOMAIN || "feishu") === "lark";

function send(obj) {
  try {
    process.stdout.write(JSON.stringify(obj) + "\n");
  } catch {
    /* ignore */
  }
}

if (!appId || !appSecret) {
  send({ type: "fatal", text: "缺少 App ID / App Secret" });
  process.exit(1);
}

const baseCfg = { appId, appSecret };
if (isLark) baseCfg.domain = Lark.Domain.Lark;

const client = new Lark.Client(baseCfg);
// autoReconnect + 回调：WS 断线官方 SDK 自动重连，并把状态回传给 Rust 端（防断 + 自检）。
const wsClient = new Lark.WSClient({
  ...baseCfg,
  autoReconnect: true,
  onReady: () => send({ type: "status", state: "connected" }),
  onError: (e) => send({ type: "log", text: "连接错误: " + ((e && e.message) || e) }),
  onReconnecting: () => send({ type: "status", state: "reconnecting" }),
  onReconnected: () => send({ type: "status", state: "connected" }),
  onClose: () => send({ type: "log", text: "WS 连接关闭" }),
});

// stdin: 逐行读回复指令 {type:'reply', chatId, text}
let buf = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", async (chunk) => {
  buf += chunk;
  let idx;
  while ((idx = buf.indexOf("\n")) >= 0) {
    const line = buf.slice(0, idx).trim();
    buf = buf.slice(idx + 1);
    if (!line) continue;
    try {
      const msg = JSON.parse(line);
      if (msg.type === "reply" && msg.chatId && msg.text) {
        await client.im.v1.message.create({
          params: { receive_id_type: "chat_id" },
          data: {
            receive_id: msg.chatId,
            msg_type: "text",
            content: JSON.stringify({ text: msg.text }),
          },
        });
        send({ type: "log", text: "已回复 " + msg.chatId });
      }
    } catch (e) {
      send({ type: "log", text: "回复失败: " + ((e && e.message) || e) });
    }
  }
});

wsClient.start({
  eventDispatcher: new Lark.EventDispatcher({}).register({
    "im.message.receive_v1": async (data) => {
      try {
        const m = (data && data.message) || {};
        let text = "";
        try {
          text = JSON.parse(m.content || "{}").text || "";
        } catch {
          /* 非文本消息 */
        }
        // 去掉 @机器人 占位符（飞书把 @ 渲染成 @_user_1 之类）
        text = text.replace(/@_user_\d+/g, "").trim();
        send({
          type: "message",
          chatId: m.chat_id || "",
          messageId: m.message_id || "",
          chatType: m.chat_type || "p2p",
          mentioned: Array.isArray(m.mentions) && m.mentions.length > 0,
          senderOpenId:
            (data.sender && data.sender.sender_id && data.sender.sender_id.open_id) || "",
          text,
        });
      } catch (e) {
        send({ type: "log", text: "收消息解析失败: " + ((e && e.message) || e) });
      }
    },
  }),
});

send({ type: "log", text: "长连接启动中…" });
process.on("uncaughtException", (e) =>
  send({ type: "log", text: "未捕获异常: " + ((e && e.message) || e) })
);
