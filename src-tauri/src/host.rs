//! Docker(server) 外壳的「宿主句柄」shim。
//!
//! 桌面版的引擎模块函数签名里写的是 `app: AppHandle`（tauri），函数体里调用
//! `app.emit("topic", payload)`、`app.clone()`、`app.path().resource_dir()`。
//! server 构建下用 `#[cfg(not(feature = "desktop"))] use crate::host::AppHandle;`
//! 把这些调用原样接到这里——**因此 17 个引擎模块的函数体一行都不用改**，
//! 桌面 / Docker 共用同一份源码（满足「Windows 更新后 Docker 快速更新」）。

use serde::Serialize;
use std::path::PathBuf;
use tokio::sync::broadcast;

/// 一条推给浏览器前端的事件：topic（对应桌面 `listen(topic)`）+ JSON payload。
#[derive(Clone, Debug)]
pub struct Event {
    pub topic: String,
    pub payload: serde_json::Value,
}

/// server 模式下替代 `tauri::AppHandle` 的轻量句柄（Clone + Send + Sync）。
/// 内部持有一个广播发送端：所有 emit 都广播给全部 WS 订阅者，前端按 reqId/runId 自行过滤。
#[derive(Clone)]
pub struct AppHandle {
    tx: broadcast::Sender<Event>,
}

impl AppHandle {
    pub fn new(tx: broadcast::Sender<Event>) -> Self {
        Self { tx }
    }

    /// 新建一个订阅端（每个 WS 连接一个）。
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    /// 克隆底层发送端（极少用到；emit 已覆盖绝大多数场景）。
    pub fn sender(&self) -> broadcast::Sender<Event> {
        self.tx.clone()
    }

    /// 对应 `tauri::Emitter::emit`：序列化 payload → 广播。
    /// 无 WS 订阅者时 `send` 返回 Err（频道里暂时没人），按桌面 `let _ = emit` 的语义忽略。
    pub fn emit<S: Serialize>(&self, topic: &str, payload: S) -> Result<(), serde_json::Error> {
        let value = serde_json::to_value(payload)?;
        let _ = self.tx.send(Event {
            topic: topic.to_string(),
            payload: value,
        });
        Ok(())
    }

    /// 对应 `tauri::Manager::path()`，仅实现引擎用到的 `resource_dir()`。
    pub fn path(&self) -> PathShim {
        PathShim
    }
}

/// 对应 tauri 的 PathResolver 的极简替身。
pub struct PathShim;

impl PathShim {
    /// 资源目录：镜像把 `src-tauri/resources` 拷到 `$POLARIS_RESOURCE_DIR`(默认 `/app/resources`)，
    /// kb.rs `seed_source` 会在其下找 `seed-kb/`（默认资料库种子）。
    pub fn resource_dir(&self) -> Result<PathBuf, std::io::Error> {
        let dir = std::env::var("POLARIS_RESOURCE_DIR")
            .unwrap_or_else(|_| "/app/resources".to_string());
        Ok(PathBuf::from(dir))
    }
}
