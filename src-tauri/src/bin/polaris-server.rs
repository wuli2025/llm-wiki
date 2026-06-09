//! Docker(server) 二进制入口：起 axum HTTP/WS 服务，复用全部 Rust 引擎。
//! 构建：cargo build --release --bin polaris-server --no-default-features --features server

#[cfg(feature = "server")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if let Err(e) = polaris_app_lib::server::serve().await {
        eprintln!("[polaris-server] 致命错误: {e:#}");
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(not(feature = "server"))]
fn main() {
    eprintln!("polaris-server 需要 `--features server` 构建。");
}
