// Hide console on Windows release builds
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(feature = "desktop")]
fn main() {
    polaris_app_lib::run();
}

// server (Docker) 构建走独立二进制 src/bin/polaris-server.rs;
// 此 main 仅桌面构建启用。无 desktop feature 时给个空 main 让 crate 仍可作为库构建。
#[cfg(not(feature = "desktop"))]
fn main() {}
