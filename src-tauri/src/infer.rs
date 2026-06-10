//! 推理后端抽象(R3) —— 让 Polaris 的「嵌入 / 重排 / 语音转写(ASR)」走可配置的外部 GPU 节点。
//!
//! 背景: 群晖 Container Manager 不开放容器内 GPU 透传(见桌面 PRD §04 的调研结论), 故 A2 推理
//! 不在本容器内跑, 而是部署在一台能正常 GPU 直通的独立 Linux 主机上, 经 HTTP 对接。本模块就是
//! 那个对接客户端:
//!   · 设了 `POLARIS_INFER_ENDPOINT` → embed/rerank/transcribe 走该端点(GPU 提速)
//!   · 未设                          → 返回 `NotConfigured`, 调用方自行降级到 CPU 兜底
//!
//! 端点契约(GPU 节点侧实现, 与本客户端约定):
//!   POST {endpoint}/embed      {"texts":[..]}                  -> {"vectors":[[..],..]}
//!   POST {endpoint}/rerank     {"query":..,"documents":[..]}   -> {"scores":[..]}
//!   POST {endpoint}/transcribe {"path":..}                     -> {"text":..}
//!   GET  {endpoint}/health                                     -> 2xx/3xx
//!
//! 说明: KB 增强管线(SQLite 落盘 / BGE 嵌入 / reranker / WhisperX)尚在规划, 暂无调用方; 这里
//! 先把抽象与连通性探测落地, 接口 ready, 等管线接入即用。故 embed/rerank/transcribe 暂标
//! `dead_code`(它们是给将来 KB 管线的稳定 API 面)。

use serde_json::Value;
use std::time::Duration;

const ENDPOINT_ENV: &str = "POLARIS_INFER_ENDPOINT";

/// 推理端点(去掉尾部 `/`)。未配置或空 → None。
pub fn endpoint() -> Option<String> {
    std::env::var(ENDPOINT_ENV)
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(3))
        .timeout(Duration::from_secs(60)) // ASR 转写可能较慢, 给宽松上限
        .build()
}

/// 给 `/api/status` 与前端用的端点状态, 含一次轻量连通性探测(GET /health)。
pub fn status_json() -> Value {
    match endpoint() {
        None => serde_json::json!({
            "configured": false,
            "mode": "cpu-fallback",
            "note": "未设 POLARIS_INFER_ENDPOINT；嵌入/重排/ASR 走本机 CPU 兜底(较慢)。"
        }),
        Some(ep) => {
            let reachable = agent()
                .get(&format!("{ep}/health"))
                .call()
                .map(|r| r.status() < 500)
                .unwrap_or(false);
            serde_json::json!({
                "configured": true,
                "endpoint": ep,
                "reachable": reachable,
                "mode": if reachable { "gpu-node" } else { "configured-unreachable" },
                "note": if reachable {
                    "已接外部 GPU 推理节点。"
                } else {
                    "已配置端点但探测不通，请检查 GPU 节点服务与内网连通。"
                }
            })
        }
    }
}

/// 推理调用的错误。`NotConfigured` 是常态信号(端点没配), 调用方据此降级 CPU, 不当真错处理。
#[derive(Debug)]
pub enum InferError {
    NotConfigured,
    Http(String),
}

impl std::fmt::Display for InferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InferError::NotConfigured => write!(f, "推理端点未配置 (POLARIS_INFER_ENDPOINT)"),
            InferError::Http(e) => write!(f, "推理端点请求失败: {e}"),
        }
    }
}
impl std::error::Error for InferError {}

fn post_json(path: &str, body: Value) -> Result<Value, InferError> {
    let ep = endpoint().ok_or(InferError::NotConfigured)?;
    let resp = agent()
        .post(&format!("{ep}{path}"))
        .send_json(body)
        .map_err(|e| InferError::Http(e.to_string()))?;
    resp.into_json::<Value>()
        .map_err(|e| InferError::Http(format!("响应解析失败: {e}")))
}

/// 文本嵌入: 一批文本 → 一批向量。供 KB 向量检索的入库/查询调用。
#[allow(dead_code)]
pub fn embed(texts: &[String]) -> Result<Vec<Vec<f32>>, InferError> {
    let v = post_json("/embed", serde_json::json!({ "texts": texts }))?;
    let rows = v
        .get("vectors")
        .and_then(|x| x.as_array())
        .ok_or_else(|| InferError::Http("响应缺 vectors 字段".into()))?;
    Ok(rows
        .iter()
        .map(|row| {
            row.as_array()
                .map(|r| r.iter().filter_map(|n| n.as_f64().map(|f| f as f32)).collect())
                .unwrap_or_default()
        })
        .collect())
}

/// 重排序: 给定 query 与候选文档, 返回每篇的相关性分数(与 documents 同序)。
#[allow(dead_code)]
pub fn rerank(query: &str, documents: &[String]) -> Result<Vec<f32>, InferError> {
    let v = post_json(
        "/rerank",
        serde_json::json!({ "query": query, "documents": documents }),
    )?;
    let scores = v
        .get("scores")
        .and_then(|x| x.as_array())
        .ok_or_else(|| InferError::Http("响应缺 scores 字段".into()))?;
    Ok(scores.iter().filter_map(|n| n.as_f64().map(|f| f as f32)).collect())
}

/// 语音转写(ASR): 服务端可达的音/视频文件路径 → 文本。供视频/音频入库的转写链路调用。
#[allow(dead_code)]
pub fn transcribe(path: &str) -> Result<String, InferError> {
    let v = post_json("/transcribe", serde_json::json!({ "path": path }))?;
    v.get("text")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| InferError::Http("响应缺 text 字段".into()))
}
