//! 板块 ⑥ API 供应商坞 — Claude Code 供应商切换 + token 用量/成本看板
//!
//! 剥离自 cc-switch 的 Claude 供应商能力, 与 Polaris 墨蓝水墨前端融为一体。
//! - 每个供应商携带一份完整 `settings_config`(env + includeCoAuthoredBy/attribution
//!   等顶层键)。切换 = 把它合并写进 `~/.claude/settings.json`(只接管我们管理的键,
//!   其余原样保留;首次改动前 .polaris.bak 备份)。Polaris 每次 spawn `claude` 重读
//!   settings, 故下一条消息即生效。
//! - 用量看板: 读 `~/.claude/projects/**/*.jsonl`(ccusage 思路), 聚合 token + 按内置
//!   定价表估算成本, 今日/周/月/年 + 14 天趋势。零额外网络、零额外依赖。
//! - Codex / Copilot: 说 OpenAI 协议, 让 `claude` 直连需翻译代理(cc-switch 的 proxy/,
//!   1.5MB+), 与轻量化冲突 → 不路由。Codex 授权委托官方 `codex` CLI。

use anyhow::Result;
use directories::UserDirs;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
#[cfg(feature = "desktop")]
use tauri::AppHandle;
#[cfg(not(feature = "desktop"))]
use crate::host::AppHandle;
use walkdir::WalkDir;

// 构建期注入的「粉丝福利」MiniMax key(XOR 滚动混淆字节, 见 build.rs)。
include!(concat!(env!("OUT_DIR"), "/gift_key.rs"));

const DEFAULT_TOKEN_FIELD: &str = "ANTHROPIC_AUTH_TOKEN";
const API_KEY_FIELD: &str = "ANTHROPIC_API_KEY";

/// 切换时先从 live env 清掉这些受管键, 再套用供应商配置 → 切换结果确定。
const MANAGED_ENV_KEYS: &[&str] = &[
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "ENABLE_TOOL_SEARCH",
    "DISABLE_AUTOUPDATER",
    "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS",
    "CLAUDE_CODE_EFFORT_LEVEL",
    // 模型钉选 —— 纳入受管, 切换时先清后套, 否则上一家的模型名会串到下一家
    // (例: 切回 Claude 官方却残留 MiniMax-M3 → 官方拿去请求 Anthropic 直接报错)。
    "ANTHROPIC_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL",
];

/// 模型钉选的四档键 —— 第三方单模型供应商把这四档全设成同一个 model id,
/// 后台小任务(走 HAIKU 档)就不会回落 Claude 默认名而被网关当未知模型处理。
const MODEL_ENV_KEYS: &[&str] = &[
    "ANTHROPIC_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
];
const MANAGED_TOP_KEYS: &[&str] = &["attribution", "includeCoAuthoredBy"];

// ───────────────────────── 预设供应商表 (全量 55) ─────────────────────────
// base_url / apiKeyField / category 取自 cc-switch claudeProviderPresets。
// kind: official(清空 env) | key(写 base+token) | codex / copilot(需授权代理)

struct Preset {
    id: &'static str,
    name: &'static str,
    base_url: &'static str,
    token_field: &'static str,
    category: &'static str,
    kind: &'static str,
}

const PRESETS: &[Preset] = &[
    Preset { id: "claude-official", name: "Claude 官方", base_url: "", token_field: DEFAULT_TOKEN_FIELD, category: "official", kind: "official" },
    Preset { id: "shengsuanyun", name: "胜算云", base_url: "https://router.shengsuanyun.com/api", token_field: DEFAULT_TOKEN_FIELD, category: "aggregator", kind: "key" },
    Preset { id: "patewayai", name: "PatewayAI", base_url: "https://api.pateway.ai", token_field: API_KEY_FIELD, category: "third_party", kind: "key" },
    Preset { id: "agentplan", name: "火山方舟 Agentplan", base_url: "https://ark.cn-beijing.volces.com/api/coding", token_field: DEFAULT_TOKEN_FIELD, category: "cn_official", kind: "key" },
    Preset { id: "byteplus", name: "BytePlus", base_url: "https://ark.ap-southeast.bytepluses.com/api/coding", token_field: DEFAULT_TOKEN_FIELD, category: "cn_official", kind: "key" },
    Preset { id: "doubaoseed", name: "豆包 Seed", base_url: "https://ark.cn-beijing.volces.com/api/compatible", token_field: DEFAULT_TOKEN_FIELD, category: "cn_official", kind: "key" },
    Preset { id: "gemini-native", name: "Gemini Native", base_url: "https://generativelanguage.googleapis.com", token_field: API_KEY_FIELD, category: "third_party", kind: "key" },
    Preset { id: "deepseek", name: "DeepSeek 深度求索", base_url: "https://api.deepseek.com/anthropic", token_field: DEFAULT_TOKEN_FIELD, category: "cn_official", kind: "key" },
    Preset { id: "zhipu-glm", name: "智谱 GLM", base_url: "https://open.bigmodel.cn/api/anthropic", token_field: DEFAULT_TOKEN_FIELD, category: "cn_official", kind: "key" },
    Preset { id: "zhipu-glm-en", name: "智谱 GLM 国际", base_url: "https://api.z.ai/api/anthropic", token_field: DEFAULT_TOKEN_FIELD, category: "cn_official", kind: "key" },
    Preset { id: "baidu-qianfan-coding-plan", name: "百度千帆 Coding", base_url: "https://qianfan.baidubce.com/anthropic/coding", token_field: DEFAULT_TOKEN_FIELD, category: "cn_official", kind: "key" },
    Preset { id: "bailian", name: "阿里百炼", base_url: "https://dashscope.aliyuncs.com/apps/anthropic", token_field: DEFAULT_TOKEN_FIELD, category: "cn_official", kind: "key" },
    Preset { id: "bailian-for-coding", name: "阿里百炼 Coding", base_url: "https://coding.dashscope.aliyuncs.com/apps/anthropic", token_field: DEFAULT_TOKEN_FIELD, category: "cn_official", kind: "key" },
    Preset { id: "kimi", name: "Kimi 月之暗面", base_url: "https://api.moonshot.cn/anthropic", token_field: DEFAULT_TOKEN_FIELD, category: "cn_official", kind: "key" },
    Preset { id: "kimi-for-coding", name: "Kimi For Coding", base_url: "https://api.kimi.com/coding/", token_field: DEFAULT_TOKEN_FIELD, category: "cn_official", kind: "key" },
    Preset { id: "stepfun", name: "StepFun 阶跃", base_url: "https://api.stepfun.com/step_plan", token_field: DEFAULT_TOKEN_FIELD, category: "cn_official", kind: "key" },
    Preset { id: "stepfun-en", name: "StepFun en", base_url: "https://api.stepfun.ai/step_plan", token_field: DEFAULT_TOKEN_FIELD, category: "cn_official", kind: "key" },
    Preset { id: "modelscope", name: "ModelScope 魔搭", base_url: "https://api-inference.modelscope.cn", token_field: DEFAULT_TOKEN_FIELD, category: "aggregator", kind: "key" },
    Preset { id: "kat-coder", name: "KAT-Coder", base_url: "https://vanchin.streamlake.ai/api/gateway/v1/endpoints/${ENDPOINT_ID}/claude-code-proxy", token_field: DEFAULT_TOKEN_FIELD, category: "cn_official", kind: "key" },
    Preset { id: "longcat", name: "LongCat", base_url: "https://api.longcat.chat/anthropic", token_field: DEFAULT_TOKEN_FIELD, category: "cn_official", kind: "key" },
    Preset { id: "minimax", name: "MiniMax", base_url: "https://api.minimaxi.com/anthropic", token_field: DEFAULT_TOKEN_FIELD, category: "cn_official", kind: "key" },
    Preset { id: "minimax-en", name: "MiniMax en", base_url: "https://api.minimax.io/anthropic", token_field: DEFAULT_TOKEN_FIELD, category: "cn_official", kind: "key" },
    Preset { id: "bailing", name: "百灵 BaiLing", base_url: "https://api.tbox.cn/api/anthropic", token_field: DEFAULT_TOKEN_FIELD, category: "cn_official", kind: "key" },
    Preset { id: "aihubmix", name: "AiHubMix", base_url: "https://aihubmix.com", token_field: API_KEY_FIELD, category: "aggregator", kind: "key" },
    Preset { id: "siliconflow", name: "SiliconFlow 硅基流动", base_url: "https://api.siliconflow.cn", token_field: DEFAULT_TOKEN_FIELD, category: "aggregator", kind: "key" },
    Preset { id: "siliconflow-en", name: "SiliconFlow en", base_url: "https://api.siliconflow.com", token_field: DEFAULT_TOKEN_FIELD, category: "aggregator", kind: "key" },
    Preset { id: "dmxapi", name: "DMXAPI", base_url: "https://www.dmxapi.cn", token_field: DEFAULT_TOKEN_FIELD, category: "aggregator", kind: "key" },
    Preset { id: "packycode", name: "PackyCode", base_url: "https://www.packyapi.com", token_field: DEFAULT_TOKEN_FIELD, category: "third_party", kind: "key" },
    Preset { id: "claudeapi", name: "ClaudeAPI", base_url: "https://gw.claudeapi.com", token_field: DEFAULT_TOKEN_FIELD, category: "aggregator", kind: "key" },
    Preset { id: "claudecn", name: "ClaudeCN", base_url: "https://claudecn.top", token_field: DEFAULT_TOKEN_FIELD, category: "third_party", kind: "key" },
    Preset { id: "runapi", name: "RunAPI", base_url: "https://runapi.co", token_field: DEFAULT_TOKEN_FIELD, category: "aggregator", kind: "key" },
    Preset { id: "relaxycode", name: "RelaxyCode", base_url: "https://www.relaxycode.com", token_field: DEFAULT_TOKEN_FIELD, category: "third_party", kind: "key" },
    Preset { id: "cubence", name: "Cubence", base_url: "https://api.cubence.com", token_field: DEFAULT_TOKEN_FIELD, category: "third_party", kind: "key" },
    Preset { id: "aigocode", name: "AIGoCode", base_url: "https://api.aigocode.com", token_field: DEFAULT_TOKEN_FIELD, category: "third_party", kind: "key" },
    Preset { id: "rightcode", name: "RightCode", base_url: "https://www.right.codes/claude", token_field: DEFAULT_TOKEN_FIELD, category: "third_party", kind: "key" },
    Preset { id: "aicodemirror", name: "AICodeMirror", base_url: "https://api.aicodemirror.com/api/claudecode", token_field: DEFAULT_TOKEN_FIELD, category: "third_party", kind: "key" },
    Preset { id: "crazyrouter", name: "CrazyRouter", base_url: "https://cn.crazyrouter.com", token_field: DEFAULT_TOKEN_FIELD, category: "third_party", kind: "key" },
    Preset { id: "sssaicode", name: "SSSAiCode", base_url: "https://node-hk.sssaicode.com/api", token_field: DEFAULT_TOKEN_FIELD, category: "third_party", kind: "key" },
    Preset { id: "compshare", name: "优云智算", base_url: "https://api.modelverse.cn", token_field: DEFAULT_TOKEN_FIELD, category: "aggregator", kind: "key" },
    Preset { id: "compshare-coding-plan", name: "优云智算 Coding", base_url: "https://cp.compshare.cn", token_field: DEFAULT_TOKEN_FIELD, category: "aggregator", kind: "key" },
    Preset { id: "micu", name: "Micu", base_url: "https://www.micuapi.ai", token_field: DEFAULT_TOKEN_FIELD, category: "third_party", kind: "key" },
    Preset { id: "ctok-ai", name: "CTok.ai", base_url: "https://api.ctok.ai", token_field: DEFAULT_TOKEN_FIELD, category: "third_party", kind: "key" },
    Preset { id: "e-flowcode", name: "E-FlowCode", base_url: "https://e-flowcode.cc", token_field: DEFAULT_TOKEN_FIELD, category: "third_party", kind: "key" },
    Preset { id: "openrouter", name: "OpenRouter", base_url: "https://openrouter.ai/api", token_field: DEFAULT_TOKEN_FIELD, category: "aggregator", kind: "key" },
    Preset { id: "therouter", name: "TheRouter", base_url: "https://api.therouter.ai", token_field: DEFAULT_TOKEN_FIELD, category: "aggregator", kind: "key" },
    Preset { id: "novita-ai", name: "Novita AI", base_url: "https://api.novita.ai/anthropic", token_field: DEFAULT_TOKEN_FIELD, category: "aggregator", kind: "key" },
    Preset { id: "github-copilot", name: "GitHub Copilot", base_url: "https://api.githubcopilot.com", token_field: DEFAULT_TOKEN_FIELD, category: "third_party", kind: "copilot" },
    Preset { id: "codex", name: "Codex (ChatGPT)", base_url: "https://chatgpt.com/backend-api/codex", token_field: DEFAULT_TOKEN_FIELD, category: "third_party", kind: "codex" },
    Preset { id: "lemondata", name: "LemonData", base_url: "https://api.lemondata.cc", token_field: API_KEY_FIELD, category: "third_party", kind: "key" },
    Preset { id: "nvidia", name: "Nvidia", base_url: "https://integrate.api.nvidia.com", token_field: DEFAULT_TOKEN_FIELD, category: "aggregator", kind: "key" },
    Preset { id: "pipellm", name: "PIPELLM", base_url: "https://cc-api.pipellm.ai", token_field: DEFAULT_TOKEN_FIELD, category: "aggregator", kind: "key" },
    Preset { id: "xiaomi-mimo", name: "小米 MiMo", base_url: "https://api.xiaomimimo.com/anthropic", token_field: DEFAULT_TOKEN_FIELD, category: "cn_official", kind: "key" },
    Preset { id: "xiaomi-mimo-token-plan-china", name: "小米 MiMo Token Plan", base_url: "https://token-plan-cn.xiaomimimo.com/anthropic", token_field: DEFAULT_TOKEN_FIELD, category: "cn_official", kind: "key" },
    Preset { id: "aws-bedrock-aksk", name: "AWS Bedrock (AKSK)", base_url: "https://bedrock-runtime.${AWS_REGION}.amazonaws.com", token_field: DEFAULT_TOKEN_FIELD, category: "cloud_provider", kind: "key" },
    Preset { id: "aws-bedrock-api-key", name: "AWS Bedrock (API Key)", base_url: "https://bedrock-runtime.${AWS_REGION}.amazonaws.com", token_field: DEFAULT_TOKEN_FIELD, category: "cloud_provider", kind: "key" },
];

fn preset_by_id(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.id == id)
}

/// 分类 → 状态点颜色 (统一色板, 比 50 个随机色更显高级感)
fn color_for(category: &str) -> &'static str {
    match category {
        "official" => "#D97757",
        "cn_official" => "#2c6fff",
        "aggregator" => "#7c5cff",
        "third_party" => "#e8833a",
        "cloud_provider" => "#ff9900",
        _ => "#2c4661",
    }
}

fn website_from_base(base: &str) -> String {
    let b = base.trim();
    if b.is_empty() {
        return String::new();
    }
    // 取 scheme://host 作为官网链接 (去掉路径与 ${占位})
    if let Some(rest) = b.strip_prefix("https://").or_else(|| b.strip_prefix("http://")) {
        let host = rest.split('/').next().unwrap_or(rest);
        if host.contains('$') {
            return String::new();
        }
        return format!("https://{host}");
    }
    String::new()
}

// ───────────────────────── 持久化 store ─────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StoredProvider {
    id: String,
    name: String,
    #[serde(default)]
    note: String,
    #[serde(default)]
    website_url: String,
    #[serde(default)]
    token_field: String,
    #[serde(default)]
    settings_config: Value,
}

// 旧版结构 (上一轮), 仅用于一次性迁移
#[derive(Debug, Clone, Default, Deserialize)]
struct LegacyCustom {
    id: String,
    name: String,
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    auth_token: String,
    #[serde(default)]
    token_field: String,
}
#[derive(Debug, Clone, Default, Deserialize)]
struct LegacyKey {
    #[serde(default)]
    auth_token: String,
    #[serde(default)]
    token_field: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    current_id: String,
    #[serde(default)]
    items: Vec<StoredProvider>,
    // legacy（迁移后清空, 不再写出）
    #[serde(default, skip_serializing)]
    custom: Vec<LegacyCustom>,
    #[serde(default, skip_serializing)]
    keys: HashMap<String, LegacyKey>,
}

static STORE: Lazy<RwLock<Store>> = Lazy::new(|| RwLock::new(Store::default()));
static STORE_PATH: Lazy<RwLock<PathBuf>> = Lazy::new(|| RwLock::new(PathBuf::new()));
/// 串行化对 settings.json / providers.json 的「读-改-写」。
/// Tauri 命令可并发跑在线程池上, 两个 provider_switch 同时进来若不串行化, 会交错写同一份
/// settings.json → 撕裂成半截。此锁保证整条 RMW 原子, 与 atomic_write 一起根治配置损坏。
static IO_LOCK: Lazy<parking_lot::Mutex<()>> = Lazy::new(|| parking_lot::Mutex::new(()));

/// 还原构建期注入的「粉丝福利」MiniMax key。
/// 二进制内为 XOR 混淆字节, 此处解出明文; 未注入(本地 dev 构建)时返回空串。
/// 提醒: 客户端解密逻辑随包一起分发, 混淆只是延缓提取, 不构成真正保护。
fn gift_minimax_key() -> String {
    if GIFT_MINIMAX_OBF.is_empty() || GIFT_MINIMAX_PAD.is_empty() {
        return String::new();
    }
    let bytes: Vec<u8> = GIFT_MINIMAX_OBF
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ GIFT_MINIMAX_PAD[i % GIFT_MINIMAX_PAD.len()])
        .collect();
    String::from_utf8(bytes).unwrap_or_default()
}

/// 首启一次性把「粉丝福利」MiniMax 供应商(含构建期注入的 key)种进 store。
/// 用 marker(`<data>/.gift_minimax_seeded`)记录, 之后即便用户在坞里删除/改空,
/// 重启也 **不会** 再种 —— 尊重用户的删除(沿用资料库播种的语义)。
/// 未注入 key(dev 构建)时直接跳过。返回是否新种了内容。
fn seed_gift_minimax(store: &mut Store, data_dir: &Path) -> bool {
    let key = gift_minimax_key();
    if key.is_empty() {
        return false;
    }
    let marker = data_dir.join(".gift_minimax_seeded");
    if marker.exists() {
        return false;
    }
    // 不管后面有没有真种进去, 都打 marker, 避免每次启动重试 + 尊重删除。
    let _ = fs::write(&marker, b"seeded\n");

    // 用户已自配同 id 供应商则不覆盖。
    if store.items.iter().any(|i| i.id == "minimax") {
        return false;
    }
    // 钉 MiniMax-M3(官方 Claude Code 文档推荐):四档全设成 M3, 后台小任务也走 M3,
    // 不再回落 Claude 默认 haiku 名被网关当未知模型。
    let cfg = config_with_model(
        default_config("https://api.minimaxi.com/anthropic", DEFAULT_TOKEN_FIELD, &key),
        "MiniMax-M3",
    );
    store.items.push(StoredProvider {
        id: "minimax".to_string(),
        name: "MiniMax".to_string(),
        note: "粉丝福利 · 预置额度，开箱即用 · MiniMax-M3".to_string(),
        website_url: "https://www.minimaxi.com".to_string(),
        token_field: DEFAULT_TOKEN_FIELD.to_string(),
        settings_config: cfg,
    });
    true
}

/// 往 settings_config 的 env 里钉模型:把 MODEL_ENV_KEYS 四档全设成同一个 model id。
/// model 为空则原样返回(不钉)。
fn config_with_model(mut cfg: Value, model: &str) -> Value {
    let model = model.trim();
    if model.is_empty() {
        return cfg;
    }
    if !cfg.is_object() {
        cfg = json!({});
    }
    let obj = cfg.as_object_mut().unwrap();
    let env = obj.entry("env".to_string()).or_insert_with(|| json!({}));
    if !env.is_object() {
        *env = json!({});
    }
    let env = env.as_object_mut().unwrap();
    for k in MODEL_ENV_KEYS {
        env.insert((*k).to_string(), Value::String(model.to_string()));
    }
    cfg
}

/// Codex 路由配置: 把 base_url 指到本地翻译代理, 钉模型为 gpt-5.5(含小任务档),
/// AUTH_TOKEN 给个占位串(代理只认 ~/.codex/auth.json, 不看这个), 让 claude 愿意发请求。
fn codex_route_config(port: u16) -> Value {
    let mut env = Map::new();
    env.insert(
        "ANTHROPIC_BASE_URL".into(),
        Value::String(format!("http://127.0.0.1:{port}")),
    );
    env.insert(
        "ANTHROPIC_AUTH_TOKEN".into(),
        Value::String("polaris-codex-proxy".into()),
    );
    for k in MODEL_ENV_KEYS {
        env.insert((*k).into(), Value::String("gpt-5.5".into()));
    }
    env.insert(
        "ANTHROPIC_SMALL_FAST_MODEL".into(),
        Value::String("gpt-5.5".into()),
    );
    json!({ "env": Value::Object(env) })
}

pub fn init(_app: &AppHandle) -> Result<()> {
    let user = UserDirs::new().ok_or_else(|| anyhow::anyhow!("no user dir"))?;
    let dir = user.home_dir().join("Polaris").join("data");
    fs::create_dir_all(&dir)?;
    let path = dir.join("providers.json");
    *STORE_PATH.write() = path.clone();

    let mut store: Store = if path.exists() {
        let txt = fs::read_to_string(&path).unwrap_or_default();
        match serde_json::from_str(&txt) {
            Ok(s) => s,
            Err(_) => {
                // 解析失败别静默 default —— 那会让用户所有已存供应商/API key 凭空消失。
                // 先把损坏文件留底, 用户仍可手工抢救, 再回落空 store。
                if !txt.trim().is_empty() {
                    let mut bak = path.as_os_str().to_owned();
                    bak.push(".corrupt.bak");
                    let _ = fs::copy(&path, PathBuf::from(bak));
                }
                Store::default()
            }
        }
    } else {
        Store::default()
    };

    // 一次性迁移旧 custom / keys → items
    let mut migrated = false;
    let legacy_custom = std::mem::take(&mut store.custom);
    let legacy_keys = std::mem::take(&mut store.keys);
    for c in legacy_custom {
        if store.items.iter().any(|i| i.id == c.id) {
            continue;
        }
        let field = if c.token_field.is_empty() {
            DEFAULT_TOKEN_FIELD.to_string()
        } else {
            c.token_field.clone()
        };
        store.items.push(StoredProvider {
            id: c.id,
            name: c.name,
            note: String::new(),
            website_url: String::new(),
            token_field: field.clone(),
            settings_config: default_config(&c.base_url, &field, &c.auth_token),
        });
        migrated = true;
    }
    for (pid, k) in legacy_keys {
        if store.items.iter().any(|i| i.id == pid) {
            continue;
        }
        if let Some(p) = preset_by_id(&pid) {
            let field = if k.token_field.is_empty() {
                p.token_field.to_string()
            } else {
                k.token_field.clone()
            };
            store.items.push(StoredProvider {
                id: pid.clone(),
                name: p.name.to_string(),
                note: String::new(),
                website_url: String::new(),
                token_field: field.clone(),
                settings_config: default_config(p.base_url, &field, &k.auth_token),
            });
            migrated = true;
        }
    }

    // 首启一次性种「粉丝福利」MiniMax(含构建期注入的 key)。
    let gifted = seed_gift_minimax(&mut store, &dir);

    *STORE.write() = store;
    if migrated || gifted {
        persist();
    }

    // 若上次退出时正路由到 Codex(本地代理), 重启后端口可能变 → 重新拉起代理并校正
    // ANTHROPIC_BASE_URL, 否则 settings.json 里残留的旧端口会让 claude 连不上。
    {
        let store = STORE.read().clone();
        let views = build_views(&store);
        if detect_current(&views, &store) == "codex" {
            if let Ok(port) = crate::codex_proxy::ensure_running() {
                let _ = apply_settings_config(&codex_route_config(port));
            }
        }
    }
    Ok(())
}

/// 原子落盘: 先写同目录临时文件, 再 rename 覆盖目标。
/// rename 在同一文件系统内是原子的 (Windows 的 `fs::rename` 用 MoveFileExW+REPLACE_EXISTING),
/// 故进程在写一半时崩溃/断电只会留下 `.polaris.tmp`, 目标文件要么旧内容要么新内容,
/// 绝不会被截成半截 JSON —— 这是「torn write 破坏 claude 配置 / 静默清空 API key」的根治。
fn atomic_write(path: &Path, contents: &str) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".polaris.tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, contents)?;
    fs::rename(&tmp, path)
}

fn persist() {
    let path = STORE_PATH.read().clone();
    if path.as_os_str().is_empty() {
        return;
    }
    let _io = IO_LOCK.lock();
    if let Ok(txt) = serde_json::to_string_pretty(&*STORE.read()) {
        let _ = atomic_write(&path, &txt);
    }
}

/// 用 base_url + token 构造最小 settings_config
fn default_config(base: &str, token_field: &str, token: &str) -> Value {
    let mut env = Map::new();
    let base = base.trim();
    if !base.is_empty() {
        env.insert("ANTHROPIC_BASE_URL".into(), Value::String(base.into()));
    }
    let token = token.trim();
    if !token.is_empty() {
        let field = if token_field.is_empty() {
            DEFAULT_TOKEN_FIELD
        } else {
            token_field
        };
        env.insert(field.into(), Value::String(token.into()));
    }
    json!({ "env": Value::Object(env) })
}

fn cfg_env_str(cfg: &Value, key: &str) -> String {
    cfg.get("env")
        .and_then(|e| e.get(key))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

// ───────────────────────── 视图模型 ─────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderView {
    pub id: String,
    pub name: String,
    pub note: String,
    pub base_url: String,
    pub token_field: String,
    pub category: String,
    pub website_url: String,
    pub color: String,
    pub kind: String,
    pub is_preset: bool,
    pub has_key: bool,
    pub auth_token: String,
    pub settings_config: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderListResult {
    pub providers: Vec<ProviderView>,
    pub current_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInput {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub website_url: String,
    #[serde(default)]
    pub token_field: Option<String>,
    #[serde(default)]
    pub settings_config: Value,
}

fn normalize_url(u: &str) -> String {
    u.trim().trim_end_matches('/').to_string()
}

fn make_view(
    id: &str,
    name: &str,
    note: &str,
    token_field: &str,
    category: &str,
    kind: &str,
    is_preset: bool,
    preset_base: &str,
    website: &str,
    cfg: Value,
) -> ProviderView {
    let env_base = cfg_env_str(&cfg, "ANTHROPIC_BASE_URL");
    let base_url = if env_base.is_empty() {
        preset_base.to_string()
    } else {
        env_base
    };
    let token = cfg_env_str(&cfg, token_field);
    let has_key = match kind {
        "official" => true,
        "codex" | "copilot" => false,
        _ => !token.is_empty(),
    };
    let website = if website.is_empty() {
        website_from_base(&base_url)
    } else {
        website.to_string()
    };
    ProviderView {
        id: id.to_string(),
        name: name.to_string(),
        note: note.to_string(),
        base_url,
        token_field: token_field.to_string(),
        category: category.to_string(),
        website_url: website,
        color: color_for(category).to_string(),
        kind: kind.to_string(),
        is_preset,
        has_key,
        auth_token: token,
        settings_config: cfg,
    }
}

fn build_views(store: &Store) -> Vec<ProviderView> {
    let mut out: Vec<ProviderView> = Vec::with_capacity(PRESETS.len() + store.items.len());

    for p in PRESETS {
        let stored = store.items.iter().find(|i| i.id == p.id);
        let token_field = stored
            .map(|s| s.token_field.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| p.token_field.to_string());
        let cfg = stored
            .map(|s| s.settings_config.clone())
            .unwrap_or_else(|| default_config(p.base_url, &token_field, ""));
        let note = stored.map(|s| s.note.as_str()).unwrap_or("");
        out.push(make_view(
            p.id, p.name, note, &token_field, p.category, p.kind, true, p.base_url, "", cfg,
        ));
    }

    for it in &store.items {
        if preset_by_id(&it.id).is_some() {
            continue; // 预设覆盖已在上方合并
        }
        let token_field = if it.token_field.is_empty() {
            DEFAULT_TOKEN_FIELD.to_string()
        } else {
            it.token_field.clone()
        };
        out.push(make_view(
            &it.id,
            &it.name,
            &it.note,
            &token_field,
            "custom",
            "custom",
            false,
            "",
            &it.website_url,
            it.settings_config.clone(),
        ));
    }

    out
}

fn detect_current(views: &[ProviderView], store: &Store) -> String {
    let live = read_live_env();
    let live_base = live
        .get("ANTHROPIC_BASE_URL")
        .and_then(|v| v.as_str())
        .map(normalize_url)
        .unwrap_or_default();

    if live_base.is_empty() {
        if store.current_id == "claude-official" || store.current_id.is_empty() {
            return "claude-official".to_string();
        }
        if let Some(v) = views.iter().find(|v| v.id == store.current_id) {
            if normalize_url(&v.base_url).is_empty() {
                return v.id.clone();
            }
        }
        return "claude-official".to_string();
    }

    if let Some(v) = views
        .iter()
        .find(|v| !v.base_url.is_empty() && normalize_url(&v.base_url) == live_base)
    {
        return v.id.clone();
    }
    if !store.current_id.is_empty() && views.iter().any(|v| v.id == store.current_id) {
        return store.current_id.clone();
    }
    String::new()
}

// ───────────────────────── ~/.claude/settings.json 读写 ─────────────────────────

fn claude_dir() -> Option<PathBuf> {
    UserDirs::new().map(|u| u.home_dir().join(".claude"))
}
fn claude_settings_path() -> Option<PathBuf> {
    claude_dir().map(|d| d.join("settings.json"))
}

fn read_live_env() -> Map<String, Value> {
    let Some(path) = claude_settings_path() else {
        return Map::new();
    };
    let Ok(txt) = fs::read_to_string(&path) else {
        return Map::new();
    };
    let Ok(v) = serde_json::from_str::<Value>(&txt) else {
        return Map::new();
    };
    v.get("env").and_then(|e| e.as_object()).cloned().unwrap_or_default()
}

/// 把供应商 settings_config 合并写进 live settings.json：
/// 先从 live 清掉受管 env/top 键，再套用 cfg 的 env 与顶层键，其余 live 键原样保留。
fn apply_settings_config(cfg: &Value) -> Result<(), String> {
    // 整条「读 settings.json → 合并 → 写回」串行化, 防并发切换交错撕裂用户实时配置。
    let _io = IO_LOCK.lock();
    let path = claude_settings_path().ok_or_else(|| "无法定位用户主目录".to_string())?;
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let mut root: Value = if path.exists() {
        let txt = fs::read_to_string(&path).map_err(|e| format!("读 settings.json 失败: {e}"))?;
        if txt.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&txt)
                .map_err(|e| format!("settings.json 不是合法 JSON, 已中止以免破坏: {e}"))?
        }
    } else {
        json!({})
    };

    if path.exists() {
        let bak = path.with_extension("json.polaris.bak");
        if !bak.exists() {
            let _ = fs::copy(&path, &bak);
        }
    }

    if !root.is_object() {
        root = json!({});
    }
    let obj = root.as_object_mut().unwrap();

    // 清受管顶层键
    for k in MANAGED_TOP_KEYS {
        obj.remove(*k);
    }
    // env: 清受管键后套用 cfg.env
    let env = obj
        .entry("env".to_string())
        .or_insert_with(|| json!({}));
    if !env.is_object() {
        *env = json!({});
    }
    let env = env.as_object_mut().unwrap();
    for k in MANAGED_ENV_KEYS {
        env.remove(*k);
    }
    if let Some(src_env) = cfg.get("env").and_then(|e| e.as_object()) {
        for (k, v) in src_env {
            env.insert(k.clone(), v.clone());
        }
    }
    // 顶层键 (除 env) 套用
    if let Some(src) = cfg.as_object() {
        for (k, v) in src {
            if k == "env" {
                continue;
            }
            obj.insert(k.clone(), v.clone());
        }
    }

    let txt = serde_json::to_string_pretty(&root)
        .map_err(|e| format!("序列化 settings.json 失败: {e}"))?;
    atomic_write(&path, &txt).map_err(|e| format!("写 settings.json 失败: {e}"))?;
    Ok(())
}

/// 给「生图」用的当前供应商画像：返回 (当前供应商展示名, 是否疑似具备真实生图能力)。
///
/// 真相：供应商坞里 55 家全部是 Anthropic 协议的文本 / 代码大模型，**没有一个能生图**；
/// 真要生图得另配一份独立的图像 API（如 OpenAI gpt-image）。所以默认「不支持」，
/// 仅当 settings.json 的 env 或进程环境里检测到 `OPENAI_API_KEY` 时才认为可尝试真实生图。
pub fn image_gen_capability() -> (String, bool) {
    let store = STORE.read().clone();
    let views = build_views(&store);
    let cur = detect_current(&views, &store);
    let name = views
        .iter()
        .find(|v| v.id == cur)
        .map(|v| v.name.clone())
        .unwrap_or_else(|| "Claude 官方".to_string());

    let live = read_live_env();
    let has_image_key = live
        .get("OPENAI_API_KEY")
        .and_then(|v| v.as_str())
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
        || std::env::var("OPENAI_API_KEY")
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);

    (name, has_image_key)
}

// ───────────────────────── Commands: 供应商 ─────────────────────────

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn provider_list() -> Result<ProviderListResult, String> {
    let store = STORE.read().clone();
    let providers = build_views(&store);
    let current_id = detect_current(&providers, &store);
    Ok(ProviderListResult { providers, current_id })
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn provider_switch(id: String) -> Result<String, String> {
    let store = STORE.read().clone();
    let views = build_views(&store);
    let v = views
        .iter()
        .find(|v| v.id == id)
        .ok_or_else(|| format!("供应商不存在: {id}"))?;

    if v.kind == "copilot" {
        return Err("GitHub Copilot 说 OpenAI 协议, 翻译代理暂未覆盖".to_string());
    }

    let cfg = if v.kind == "codex" {
        // Codex(ChatGPT) → 路由到本地翻译代理: 先确认已授权, 再拉起代理并把
        // ANTHROPIC_BASE_URL 指到 127.0.0.1:port, claude 即透明用上 ChatGPT 订阅。
        let authed = codex_auth_path()
            .map(|p| codex_auth_has_tokens(&p))
            .unwrap_or(false);
        if !authed {
            return Err("请先授权 ChatGPT (Codex), 再切换到它".to_string());
        }
        let port = crate::codex_proxy::ensure_running()?;
        codex_route_config(port)
    } else if v.kind == "official" {
        json!({ "env": {} })
    } else {
        if v.auth_token.trim().is_empty() {
            return Err("该供应商尚未配置 API Key, 请先在弹窗中填写".to_string());
        }
        v.settings_config.clone()
    };
    apply_settings_config(&cfg)?;

    // 同步当前进程 env: spawn 出去的 claude 子进程会继承父进程 env, 而进程 env 通常**优先于**
    // settings.json 的 env(实测), 不在此处先清后置就会出现:
    //   ① 切到 M3 → 进程被 set 了 ANTHROPIC_BASE_URL=minimaxi
    //   ② 切回官方 → 只清了 settings, 父进程残留仍把 claude 拖到 minimaxi → 一直只能用 M3
    // 先按 MANAGED_ENV_KEYS 全清, 再把新 cfg.env 写进当前进程 —— 切换结果确定。
    for k in MANAGED_ENV_KEYS {
        std::env::remove_var(k);
    }
    if let Some(src_env) = cfg.get("env").and_then(|e| e.as_object()) {
        for (k, val) in src_env {
            if let Some(s) = val.as_str() {
                std::env::set_var(k, s);
            }
        }
    }

    STORE.write().current_id = id.clone();
    persist();
    Ok(id)
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn provider_save(input: ProviderInput) -> Result<String, String> {
    let token_field = input
        .token_field
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_TOKEN_FIELD.to_string());

    // settings_config 兜底为 {env:{}}
    let cfg = if input.settings_config.is_object() {
        input.settings_config.clone()
    } else {
        json!({ "env": {} })
    };

    let id = input
        .id
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("custom-{}", now_ms()));

    let item = StoredProvider {
        id: id.clone(),
        name: input.name.trim().to_string(),
        note: input.note.trim().to_string(),
        website_url: normalize_url(&input.website_url),
        token_field,
        settings_config: cfg,
    };

    let mut store = STORE.write();
    if let Some(existing) = store.items.iter_mut().find(|i| i.id == id) {
        *existing = item;
    } else {
        store.items.push(item);
    }
    drop(store);
    persist();
    Ok(id)
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn provider_delete(id: String) -> Result<(), String> {
    let mut store = STORE.write();
    store.items.retain(|i| i.id != id);
    if store.current_id == id {
        store.current_id = "claude-official".to_string();
    }
    drop(store);
    persist();
    Ok(())
}

// ───────────────────────── Commands: Codex 授权 (原生 Device Code OAuth) ─────────────────────────
//
// 抄自 cc-switch 新版 `codex_oauth_auth.rs` 的 OpenAI Device Code 流程, 但**不背它的翻译代理**:
// Polaris 不路由 Codex, 拿到的 token 按官方 codex CLI 的 `~/.codex/auth.json` 格式落盘, 让外部
// `codex` CLI 直接复用。这样「点授权」彻底不依赖 codex CLI 是否已装, 后端直接拉起浏览器授权页。
//
// 三步: ① POST usercode 取 device_auth_id + user_code, 同时开浏览器到验证页;
//        ② 前端按 interval 轮询 token 端点 (403/404=等待用户授权);
//        ③ 用户授权后返回 authorization_code + code_verifier, 换 access/refresh/id_token 落盘。

/// OpenAI OAuth 客户端 ID (与官方 Codex CLI 相同)
pub(crate) const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CODEX_DEVICE_USERCODE_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
const CODEX_DEVICE_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
pub(crate) const CODEX_OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CODEX_DEVICE_VERIFY_URL: &str = "https://auth.openai.com/codex/device";
/// Device Code 流程约定的 redirect_uri (OpenAI 服务端固定)
const CODEX_DEVICE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
const CODEX_USER_AGENT: &str = "polaris-codex-oauth";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexStatus {
    pub installed: bool,
    pub logged_in: bool,
    pub auth_path: String,
}

pub(crate) fn codex_auth_path() -> Option<PathBuf> {
    UserDirs::new().map(|u| u.home_dir().join(".codex").join("auth.json"))
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn codex_status() -> Result<CodexStatus, String> {
    let installed = Command::new("codex")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let auth_path = codex_auth_path();
    // 授权与否只看 ~/.codex/auth.json 是否有 ChatGPT tokens —— 与 codex CLI 是否已装解耦。
    let logged_in = auth_path
        .as_ref()
        .map(|p| codex_auth_has_tokens(p))
        .unwrap_or(false);
    Ok(CodexStatus {
        installed,
        logged_in,
        auth_path: auth_path.map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
    })
}

/// auth.json 存在且带 ChatGPT OAuth tokens (区别于纯 API key 登录)
pub(crate) fn codex_auth_has_tokens(path: &Path) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| {
            v.get("tokens")
                .and_then(|t| t.get("access_token"))
                .and_then(|a| a.as_str())
                .map(|s| !s.is_empty())
        })
        .unwrap_or(false)
}

/// `codex_start_login` 返回给前端的设备授权信息
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexDeviceLogin {
    /// device_auth_id, 轮询时回传
    pub device_code: String,
    /// 展示给用户的配对码
    pub user_code: String,
    /// 浏览器验证页 (已自动打开, UI 也显示便于手动打开)
    pub verification_uri: String,
    /// 建议轮询间隔 (秒)
    pub interval: u64,
    /// 设备码有效期 (秒)
    pub expires_in: u64,
}

/// `codex_poll_login` 返回: status = "pending" | "ok"
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexPollResult {
    pub status: String,
}

#[derive(Deserialize)]
struct CodexDeviceCodeResp {
    device_auth_id: String,
    user_code: String,
    #[serde(default)]
    interval: Option<Value>,
    #[serde(default)]
    expires_in: Option<u64>,
}

#[derive(Deserialize)]
struct CodexDevicePollSuccess {
    authorization_code: String,
    code_verifier: String,
}

#[derive(Deserialize)]
struct CodexTokenResp {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
}

/// 提取 ureq 错误里的状态码/文案, 拼成可读消息
fn codex_http_err(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            let body = body.chars().take(300).collect::<String>();
            format!("HTTP {code} - {body}")
        }
        ureq::Error::Transport(t) => format!("网络错误: {t}"),
    }
}

/// 解析 interval 字段 (服务端可能给数字或字符串), 加 3 秒安全余量
fn codex_parse_interval(v: Option<&Value>) -> u64 {
    let raw = match v {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(5),
        Some(Value::String(s)) => s.parse::<u64>().unwrap_or(5),
        _ => 5,
    };
    raw.max(1) + 3
}

/// 带超时的 OAuth agent: 设备授权 / 轮询 / 换 token 都是非流式请求-响应, 给整条 call 30s
/// 全局 deadline, 防 OpenAI 认证端点黑洞把 Tauri 命令线程挂死 (轮询命令更会每次挂一条)。
fn codex_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .build()
}

/// ① 启动 Device Code 流程并打开浏览器验证页
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn codex_start_login() -> Result<CodexDeviceLogin, String> {
    let resp = codex_agent()
        .post(CODEX_DEVICE_USERCODE_URL)
        .set("Content-Type", "application/json")
        .set("User-Agent", CODEX_USER_AGENT)
        .send_json(json!({ "client_id": CODEX_CLIENT_ID }))
        .map_err(|e| format!("发起 ChatGPT 设备授权失败: {}", codex_http_err(e)))?;

    let device: CodexDeviceCodeResp = resp
        .into_json()
        .map_err(|e| format!("解析设备码响应失败: {e}"))?;

    let interval = codex_parse_interval(device.interval.as_ref());
    let expires_in = device.expires_in.unwrap_or(900);

    // 自动拉起浏览器到验证页 (失败不致命, UI 仍展示链接 + 配对码供手动打开)
    let _ = codex_open_browser(CODEX_DEVICE_VERIFY_URL);

    Ok(CodexDeviceLogin {
        device_code: device.device_auth_id,
        user_code: device.user_code,
        verification_uri: CODEX_DEVICE_VERIFY_URL.to_string(),
        interval,
        expires_in,
    })
}

/// ② 轮询授权状态; 成功则换 token 并落盘 ~/.codex/auth.json
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn codex_poll_login(device_code: String, user_code: String) -> Result<CodexPollResult, String> {
    let pending = || Ok(CodexPollResult { status: "pending".into() });

    let resp = match codex_agent()
        .post(CODEX_DEVICE_TOKEN_URL)
        .set("Content-Type", "application/json")
        .set("User-Agent", CODEX_USER_AGENT)
        .send_json(json!({ "device_auth_id": device_code, "user_code": user_code }))
    {
        Ok(r) => r,
        // 403/404 = 用户尚未在浏览器完成授权, 继续轮询
        Err(ureq::Error::Status(403, _)) | Err(ureq::Error::Status(404, _)) => return pending(),
        Err(ureq::Error::Status(410, _)) => {
            return Err("设备码已过期, 请重新发起授权".into())
        }
        Err(e) => return Err(format!("轮询授权状态失败: {}", codex_http_err(e))),
    };

    let success: CodexDevicePollSuccess = resp
        .into_json()
        .map_err(|e| format!("解析授权响应失败: {e}"))?;

    // ③ authorization_code + code_verifier 换 access/refresh/id_token
    let tokens = codex_exchange_code(&success.authorization_code, &success.code_verifier)?;
    let refresh_token = tokens
        .refresh_token
        .clone()
        .ok_or_else(|| "授权响应缺少 refresh_token".to_string())?;
    let account_id = codex_account_id(&tokens);

    codex_write_auth_json(&tokens, &refresh_token, account_id.as_deref())?;
    Ok(CodexPollResult { status: "ok".into() })
}

/// 用 authorization_code + code_verifier 换 token
fn codex_exchange_code(code: &str, code_verifier: &str) -> Result<CodexTokenResp, String> {
    let resp = codex_agent()
        .post(CODEX_OAUTH_TOKEN_URL)
        .set("User-Agent", CODEX_USER_AGENT)
        .send_form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", CODEX_DEVICE_REDIRECT_URI),
            ("client_id", CODEX_CLIENT_ID),
            ("code_verifier", code_verifier),
        ])
        .map_err(|e| format!("换取 Token 失败: {}", codex_http_err(e)))?;
    resp.into_json()
        .map_err(|e| format!("解析 Token 响应失败: {e}"))
}

/// 从 id_token / access_token (JWT) 中提取 chatgpt_account_id
fn codex_account_id(tokens: &CodexTokenResp) -> Option<String> {
    let from = |jwt: &str| -> Option<String> {
        let claims = codex_jwt_claims(jwt)?;
        claims
            .get("chatgpt_account_id")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| {
                claims
                    .get("https://api.openai.com/auth")
                    .and_then(|a| a.get("chatgpt_account_id"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .or_else(|| {
                claims
                    .get("organizations")
                    .and_then(|o| o.as_array())
                    .and_then(|a| a.first())
                    .and_then(|o| o.get("id"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
    };
    tokens
        .id_token
        .as_deref()
        .and_then(from)
        .or_else(|| from(&tokens.access_token))
}

/// 解析 JWT 的 payload (第二段) 为 JSON
fn codex_jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = codex_b64url_decode(payload)?;
    serde_json::from_slice(&bytes).ok()
}

/// base64url (无填充) 解码 —— 不引第三方 base64 crate
pub(crate) fn codex_b64url_decode(input: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for c in input.bytes() {
        if c == b'=' {
            break;
        }
        acc = (acc << 6) | val(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// 按官方 codex CLI 格式写 ~/.codex/auth.json, 外部 `codex` CLI 可直接复用
fn codex_write_auth_json(
    tokens: &CodexTokenResp,
    refresh_token: &str,
    account_id: Option<&str>,
) -> Result<(), String> {
    let path = codex_auth_path().ok_or_else(|| "无法定位用户主目录".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 ~/.codex 目录失败: {e}"))?;
    }

    let auth = json!({
        "OPENAI_API_KEY": Value::Null,
        "tokens": {
            "id_token": tokens.id_token.clone().unwrap_or_default(),
            "access_token": tokens.access_token,
            "refresh_token": refresh_token,
            "account_id": account_id.unwrap_or_default(),
        },
        "last_refresh": codex_rfc3339_now(),
    });

    let content = serde_json::to_string_pretty(&auth)
        .map_err(|e| format!("序列化 auth.json 失败: {e}"))?;
    fs::write(&path, content).map_err(|e| format!("写入 ~/.codex/auth.json 失败: {e}"))?;
    Ok(())
}

/// 当前 UTC 时间的 RFC3339 字符串 (codex CLI 解析 last_refresh 用), 不引 chrono
pub(crate) fn codex_rfc3339_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Howard Hinnant 的 civil_from_days 算法
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// 打开系统默认浏览器到指定 URL (跨平台)
fn codex_open_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW(0x0800_0000): 别闪黑窗
        Command::new("cmd")
            .args(["/C", "start", "", url])
            .creation_flags(0x0800_0000)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(url).spawn().map_err(|e| e.to_string())?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open").arg(url).spawn().map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ───────────────────────── Commands: 用量看板 ─────────────────────────

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenBucket {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_creation: u64,
    pub total: u64,
    pub requests: u64,
    pub cost: f64,
}

impl TokenBucket {
    fn add(&mut self, u: &Usage, cost: f64) {
        self.input += u.input;
        self.output += u.output;
        self.cache_read += u.cache_read;
        self.cache_creation += u.cache_creation;
        self.total += u.input + u.output + u.cache_read + u.cache_creation;
        self.requests += 1;
        self.cost += cost;
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyUsage {
    pub date: String,
    pub label: String,
    pub total: u64,
    pub cost: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub available: bool,
    pub today: TokenBucket,
    pub week: TokenBucket,
    pub month: TokenBucket,
    pub year: TokenBucket,
    pub daily: Vec<DailyUsage>,
}

struct Usage {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_creation: u64,
}

/// 模型 → (input, output, cache_write, cache_read) USD / 1M tokens。估算用。
fn model_price(model: &str) -> (f64, f64, f64, f64) {
    let m = model.to_ascii_lowercase();
    if m.contains("opus") {
        (15.0, 75.0, 18.75, 1.5)
    } else if m.contains("haiku") {
        (0.8, 4.0, 1.0, 0.08)
    } else if m.contains("sonnet") {
        (3.0, 15.0, 3.75, 0.3)
    } else if m.contains("gpt") || m.contains("codex") || m.starts_with("o1") || m.starts_with("o3")
    {
        (1.25, 10.0, 1.5625, 0.125)
    } else if m.contains("gemini") {
        (1.25, 10.0, 1.625, 0.31)
    } else if m.contains("deepseek") {
        (0.27, 1.1, 0.027, 0.027)
    } else if m.contains("glm") {
        (0.6, 2.2, 0.11, 0.11)
    } else if m.contains("kimi") || m.contains("moonshot") {
        (0.6, 2.5, 0.15, 0.15)
    } else if m.contains("qwen") || m.contains("minimax") {
        (0.4, 1.2, 0.08, 0.08)
    } else {
        (3.0, 15.0, 3.75, 0.3) // 未知 → Sonnet 档
    }
}

fn line_cost(u: &Usage, model: &str) -> f64 {
    let (pin, pout, pcw, pcr) = model_price(model);
    (u.input as f64 * pin
        + u.output as f64 * pout
        + u.cache_creation as f64 * pcw
        + u.cache_read as f64 * pcr)
        / 1_000_000.0
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub fn usage_summary() -> Result<UsageSummary, String> {
    let Some(dir) = claude_dir().map(|d| d.join("projects")) else {
        return Ok(empty_summary());
    };
    if !dir.exists() {
        return Ok(empty_summary());
    }

    let today_days = today_utc_days();
    let today_str = ymd_string(today_days);
    let week_cut = ymd_string(today_days - 6);
    let month_cut = ymd_string(today_days - 29);
    let year_cut = ymd_string(today_days - 364);

    // 14 天趋势窗
    let mut trend_window: Vec<(String, String)> = Vec::with_capacity(14);
    for off in (0..14).rev() {
        let d = today_days - off;
        let s = ymd_string(d);
        let label = s.get(5..).unwrap_or(&s).to_string();
        trend_window.push((s, label));
    }
    let trend_set: HashSet<String> = trend_window.iter().map(|(s, _)| s.clone()).collect();
    let mut by_day: HashMap<String, (u64, f64)> = HashMap::new();

    let mut today = TokenBucket::default();
    let mut week = TokenBucket::default();
    let mut month = TokenBucket::default();
    let mut year = TokenBucket::default();
    let mut seen: HashSet<String> = HashSet::new();

    for entry in WalkDir::new(&dir).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.path().extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(file) = fs::File::open(entry.path()) else {
            continue;
        };
        let reader = BufReader::new(file);
        for line in reader.lines() {
            let Ok(line) = line else { continue };
            if line.trim().is_empty() || !line.contains("\"usage\"") {
                continue;
            }
            let Ok(v) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
                continue;
            }
            let Some(msg) = v.get("message") else { continue };
            let Some(usage_v) = msg.get("usage") else {
                continue;
            };
            if let Some(mid) = msg.get("id").and_then(|x| x.as_str()) {
                if !seen.insert(mid.to_string()) {
                    continue;
                }
            }
            let u = Usage {
                input: usage_v.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
                output: usage_v.get("output_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
                cache_read: usage_v.get("cache_read_input_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
                cache_creation: usage_v.get("cache_creation_input_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
            };
            let line_tokens = u.input + u.output + u.cache_read + u.cache_creation;
            if line_tokens == 0 {
                continue;
            }
            let model = msg.get("model").and_then(|x| x.as_str()).unwrap_or("");
            let cost = line_cost(&u, model);

            let date = v
                .get("timestamp")
                .and_then(|t| t.as_str())
                .map(|s| s.chars().take(10).collect::<String>())
                .unwrap_or_default();
            if date.is_empty() {
                continue;
            }

            if date.as_str() >= year_cut.as_str() {
                year.add(&u, cost);
                if date.as_str() >= month_cut.as_str() {
                    month.add(&u, cost);
                    if date.as_str() >= week_cut.as_str() {
                        week.add(&u, cost);
                        if date == today_str {
                            today.add(&u, cost);
                        }
                    }
                }
            }
            if trend_set.contains(&date) {
                let e = by_day.entry(date).or_insert((0, 0.0));
                e.0 += line_tokens;
                e.1 += cost;
            }
        }
    }

    let daily: Vec<DailyUsage> = trend_window
        .into_iter()
        .map(|(date, label)| {
            let (total, cost) = by_day.get(&date).copied().unwrap_or((0, 0.0));
            DailyUsage { date, label, total, cost }
        })
        .collect();

    Ok(UsageSummary {
        available: true,
        today,
        week,
        month,
        year,
        daily,
    })
}

fn empty_summary() -> UsageSummary {
    UsageSummary {
        available: false,
        today: TokenBucket::default(),
        week: TokenBucket::default(),
        month: TokenBucket::default(),
        year: TokenBucket::default(),
        daily: Vec::new(),
    }
}

// ───────────────────────── 工具函数 ─────────────────────────

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn today_utc_days() -> i64 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    (secs / 86400) as i64
}

/// 天数 → YYYY-MM-DD (Howard Hinnant civil_from_days, 无外部依赖)
fn ymd_string(z: i64) -> String {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    format!("{:04}-{:02}-{:02}", y, m, d)
}
