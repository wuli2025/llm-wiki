<script setup lang="ts">
import { onMounted, ref } from "vue";
import { feishu, type FeishuConfig, type FeishuTestResult } from "../tauri";

const cfg = ref<FeishuConfig>({
  enabled: false,
  appId: "",
  appSecret: "",
  domain: "feishu",
  dmPolicy: "open",
  groupRequireMention: true,
  allowFrom: [],
});
const allowText = ref("");
const saving = ref(false);
const testing = ref(false);
const testResult = ref<FeishuTestResult | null>(null);
const savedMsg = ref("");

async function load() {
  cfg.value = await feishu.getConfig();
  allowText.value = (cfg.value.allowFrom || []).join("\n");
}

async function save() {
  saving.value = true;
  savedMsg.value = "";
  try {
    cfg.value.allowFrom = allowText.value
      .split(/\r?\n/)
      .map((s) => s.trim())
      .filter(Boolean);
    await feishu.setConfig(cfg.value);
    savedMsg.value = "已保存";
    await load();
  } catch (e: any) {
    savedMsg.value = `保存失败: ${e}`;
  } finally {
    saving.value = false;
  }
}

async function test() {
  testing.value = true;
  testResult.value = null;
  try {
    await save(); // 先存再测，确保用最新凭证
    testResult.value = await feishu.test();
  } catch (e: any) {
    testResult.value = { ok: false, botName: "", botOpenId: "", message: `${e}` };
  } finally {
    testing.value = false;
  }
}

onMounted(load);
</script>

<template>
  <div class="fs-root">
    <div class="head">
      <div class="title">飞书网关</div>
      <div class="sub">
        填入飞书开放平台的机器人 App ID / Secret，<b>无需公网服务器</b>即可让飞书里
        @机器人 的消息走 Polaris 对话管线（人格 + 知识库）后自动回复。
      </div>
      <div class="stage-note">
        当前为<b>阶段 A</b>：凭证配置 + 连接测试已可用；实时收发（WebSocket 长连接）为阶段 B，需联调后开启。
      </div>
    </div>

    <div class="form">
      <label class="fld">
        <span>App ID</span>
        <input v-model="cfg.appId" placeholder="cli_xxxxxxxx" spellcheck="false" />
      </label>
      <label class="fld">
        <span>App Secret</span>
        <input v-model="cfg.appSecret" type="password" placeholder="留 ******** 表示不修改" spellcheck="false" />
      </label>
      <label class="fld">
        <span>版本</span>
        <select v-model="cfg.domain">
          <option value="feishu">飞书（国内）</option>
          <option value="lark">Lark（国际）</option>
        </select>
      </label>
      <label class="fld">
        <span>私聊策略</span>
        <select v-model="cfg.dmPolicy">
          <option value="open">开放（任何人可私聊）</option>
          <option value="allowlist">白名单（仅下方 open_id）</option>
          <option value="disabled">关闭私聊</option>
        </select>
      </label>
      <label class="fld check">
        <input type="checkbox" v-model="cfg.groupRequireMention" />
        <span>群聊需 @机器人才响应（推荐）</span>
      </label>
      <label class="fld" v-if="cfg.dmPolicy === 'allowlist'">
        <span>白名单 open_id（每行一个）</span>
        <textarea v-model="allowText" rows="3" placeholder="ou_xxx" spellcheck="false"></textarea>
      </label>
    </div>

    <div class="actions">
      <button class="btn primary" :disabled="saving" @click="save">
        {{ saving ? "保存中…" : "保存" }}
      </button>
      <button class="btn" :disabled="testing" @click="test">
        {{ testing ? "测试中…" : "连接测试" }}
      </button>
      <span v-if="savedMsg" class="saved">{{ savedMsg }}</span>
    </div>

    <div v-if="testResult" class="result" :class="{ ok: testResult.ok, err: !testResult.ok }">
      {{ testResult.message }}
      <span v-if="testResult.ok && testResult.botOpenId" class="oid">
        bot open_id: {{ testResult.botOpenId }}
      </span>
    </div>

    <div class="howto">
      <div class="ht-title">怎么拿到 App ID / Secret（小白指引）</div>
      <ol>
        <li>打开飞书开放平台 open.feishu.cn → 「开发者后台」→ 创建「企业自建应用」。</li>
        <li>在「凭证与基础信息」页复制 App ID 与 App Secret，填到上方。</li>
        <li>「权限管理」开启：获取与发送单聊/群组消息、读取机器人信息。</li>
        <li>「事件订阅」选择<b>长连接</b>方式（无需填回调地址）。</li>
        <li>把机器人加进群，@它发消息即可（阶段 B 开启后实时回复）。</li>
      </ol>
    </div>
  </div>
</template>

<style scoped>
.fs-root {
  flex: 1;
  overflow-y: auto;
  padding: 22px 28px;
  background: var(--bg);
}
.head .title {
  font-family: var(--serif);
  font-size: 18px;
  letter-spacing: 2px;
  color: var(--ink);
}
.head .sub {
  font-size: 12.5px;
  color: var(--muted);
  margin-top: 6px;
  max-width: 680px;
  line-height: 1.6;
}
.stage-note {
  margin-top: 8px;
  font-size: 12px;
  color: var(--text-2);
  background: var(--selection-bg);
  border-left: 3px solid var(--primary);
  padding: 7px 12px;
  border-radius: 0 8px 8px 0;
  max-width: 680px;
}
.form {
  margin-top: 18px;
  display: flex;
  flex-direction: column;
  gap: 14px;
  max-width: 520px;
}
.fld {
  display: flex;
  flex-direction: column;
  gap: 5px;
}
.fld > span {
  font-size: 12px;
  color: var(--text-2);
}
.fld input,
.fld select,
.fld textarea {
  padding: 7px 10px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--panel);
  color: var(--text);
  font-size: 13px;
}
.fld input:focus,
.fld select:focus,
.fld textarea:focus {
  outline: none;
  border-color: var(--primary);
}
.fld.check {
  flex-direction: row;
  align-items: center;
  gap: 8px;
}
.fld.check span {
  font-size: 13px;
  color: var(--text-2);
}
.actions {
  margin-top: 18px;
  display: flex;
  align-items: center;
  gap: 10px;
}
.btn {
  padding: 7px 16px;
  border-radius: 6px;
  border: 1px solid var(--border);
  background: var(--panel);
  color: var(--text);
  font-size: 13px;
  cursor: pointer;
}
.btn:hover {
  background: var(--selection-bg);
}
.btn.primary {
  background: var(--ink);
  border-color: var(--ink);
  color: #fff;
}
.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.saved {
  font-size: 12.5px;
  color: var(--primary);
}
.result {
  margin-top: 14px;
  padding: 10px 14px;
  border-radius: 8px;
  font-size: 13px;
  max-width: 680px;
}
.result.ok {
  background: rgba(74, 222, 128, 0.12);
  color: #2f8f50;
}
.result.err {
  background: rgba(248, 113, 113, 0.12);
  color: var(--vermilion);
}
.result .oid {
  display: block;
  margin-top: 4px;
  font-family: ui-monospace, Consolas, monospace;
  font-size: 11.5px;
  opacity: 0.8;
}
.howto {
  margin-top: 26px;
  padding-top: 16px;
  border-top: 1px solid var(--border-soft);
  max-width: 680px;
}
.ht-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
  margin-bottom: 8px;
}
.howto ol {
  margin: 0;
  padding-left: 20px;
  color: var(--muted);
  font-size: 12.5px;
  line-height: 1.9;
}
</style>
