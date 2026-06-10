// ── 引擎模块（桌面 + Docker 两种外壳共用同一份源码）──
pub mod accounts;
pub mod chat;
pub mod claude_md;
pub mod codex_proxy;
pub mod conv;
pub mod convert;
pub mod doctor;
pub mod feishu;
pub mod forge;
pub mod forge_pptx;
pub mod forge_tts;
pub mod forge_video;
pub mod infer;
pub mod kb;
pub mod persona;
pub mod project;
pub mod provider;
pub mod skills;
pub mod wecom;
// 自动更新依赖 Tauri updater/restart/package_info → 桌面专属（Docker 用 docker pull 更新）。
#[cfg(feature = "desktop")]
pub mod updater;

// ── Docker(server) 外壳：shim AppHandle + axum HTTP/WS 服务 ──
#[cfg(feature = "server")]
pub mod host;
#[cfg(feature = "server")]
pub mod server;

#[cfg(feature = "desktop")]
use polaris_core::KbLocator;
#[cfg(feature = "desktop")]
use std::sync::Arc;
#[cfg(feature = "desktop")]
use tauri::Manager;

/// host 适配器：把板块② `kb` 的 `kb_root()` 适配成 core 的 [`KbLocator`] 契约，
/// 在启动时注入给板块⑤ `polaris-sandbox`，从而打破 `sandbox → kb` 的直接依赖。
/// （架构重构 Phase 1：依赖反转的落地点）
#[cfg(feature = "desktop")]
struct HostKbLocator;
#[cfg(feature = "desktop")]
impl KbLocator for HostKbLocator {
    fn kb_root(&self) -> std::path::PathBuf {
        std::path::PathBuf::from(kb::kb_root())
    }
}

#[cfg(feature = "desktop")]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        // 自动更新（前端在启动时检查 GitHub Releases）+ 重启
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let h = app.handle();
            kb::init(h).map_err(|e| -> Box<dyn std::error::Error> { e.to_string().into() })?;
            // 注入 KbLocator 给 sandbox 板块 (须在 kb::init 之后, 命令执行之前)
            app.manage(Arc::new(HostKbLocator) as Arc<dyn KbLocator>);
            polaris_sandbox::init()
                .map_err(|e| -> Box<dyn std::error::Error> { e.to_string().into() })?;
            conv::init(h).map_err(|e| -> Box<dyn std::error::Error> { e.to_string().into() })?;
            chat::init(h).map_err(|e| -> Box<dyn std::error::Error> { e.to_string().into() })?;
            claude_md::init(h)
                .map_err(|e| -> Box<dyn std::error::Error> { e.to_string().into() })?;
            provider::init(h)
                .map_err(|e| -> Box<dyn std::error::Error> { e.to_string().into() })?;
            // 确保「课件视频工坊」技能落盘（支撑「生成课件类视频」UI 的基础设施技能，
            // 编译期内嵌 → 全新安装即可用、脚本修复随 App 更新下发）。best-effort，不阻断启动。
            skills::seed_video_studio_skill();
            // 确保「演示工坊」技能落盘（支撑「PPT 演示」入口）。
            skills::seed_deck_studio_skill();
            // 确保「网站生成」技能落盘（支撑「网站生成」入口）。
            skills::seed_web_studio_skill();
            // 确保「壹伴排版优化」技能落盘（含 wechat_yiban.py：壹伴样式引擎 + CloakBrowser 驱动，
            // spawn 的 claude agent 才能在磁盘上直接 python 跑它）。best-effort，不阻断启动。
            skills::seed_wechat_typesetter_skill();
            // 老用户迁移：早期版本首启播种过毛主席资料库的，补装 consult-mao 技能
            //（改版后该技能随「毛主席」名人资料包一起装，老用户没装过会失效）。
            skills::migrate_consult_mao_for_seeded_kb();
            // 环境预热: 后台把 claude / pwsh 目录塞进进程 PATH + 设 Git Bash 路径,
            // 让之后 spawn 的 claude CLI 直接「找得到、有 shell」, 无需重启 (见 doctor.rs)。
            doctor::prime_path_for_claude();
            // 自动更新状态机初始化（记录当前版本 + 持久化路径 + 重启续提示）。best-effort。
            let _ = updater::init(h);
            // 飞书网关「开机自动启动」：若用户开了 auto_start 且凭证齐全，后台自动拉起（不阻塞启动）。
            feishu::auto_start_if_enabled(h);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // KB
            kb::kb_root,
            kb::kb_default_root,
            kb::kb_set_root,
            kb::kb_scan,
            kb::kb_compile,
            kb::kb_list,
            kb::kb_read,
            kb::kb_delete,
            kb::kb_clear,
            kb::kb_search,
            kb::kb_ingest,
            kb::kb_upload_files,
            kb::kb_convert_batch,
            kb::kb_graph,
            kb::kb_lint,
            kb::kb_enrich_links,
            kb::kb_dedup,
            // 名人资料包（下载到自己的资料库，附带配套 skill）
            kb::kb_pack_list,
            kb::kb_pack_install,
            kb::kb_pack_remove,
            // Sandbox (板块⑤ 已抽离为 polaris-sandbox crate, 命令名不变)
            polaris_sandbox::commands::sandbox_status,
            polaris_sandbox::commands::sandbox_build_image,
            polaris_sandbox::commands::sandbox_start,
            polaris_sandbox::commands::sandbox_stop,
            polaris_sandbox::commands::sandbox_exec,
            // CubeSandbox (E2B) 后端 — 「替换 Docker」可选后端
            polaris_sandbox::e2b::cube_config_get,
            polaris_sandbox::e2b::cube_config_set,
            polaris_sandbox::e2b::cube_status,
            // Conv (项目 + 对话历史)
            conv::conv_list_projects,
            conv::conv_create_project,
            conv::conv_archive_project,
            conv::conv_open_project_dir,
            conv::conv_list_conversations,
            conv::conv_create_conversation,
            conv::conv_delete_conversation,
            conv::conv_rename_conversation,
            conv::conv_get_messages,
            conv::conv_set_project_kb_scope,
            // 人格模块 (板块⑫)
            persona::persona_list,
            persona::persona_apply,
            // 飞书网关 (板块⑭ 阶段 A)
            feishu::feishu_get_config,
            feishu::feishu_set_config,
            feishu::feishu_test_connection,
            feishu::feishu_create_qr,
            feishu::feishu_open_console,
            // 飞书对话引擎（阶段B：Node 桥长连接 → headless claude → 回发）
            feishu::feishu_gateway_start,
            feishu::feishu_gateway_stop,
            feishu::feishu_gateway_status,
            // 企业微信智能机器人「扫码自动配置」(OAuth 回环, 绕开 Tauri 弹窗限制)
            wecom::wecom_scan_create,
            // 自媒体「账号管理」: 探测平台登录态 + 解绑（删 profile）
            accounts::media_accounts_status,
            accounts::media_account_forget,
            // Chat
            chat::chat_send,
            chat::chat_cancel,
            chat::chat_attach_files,
            chat::chat_build_manifest,
            chat::artifact_read,
            chat::artifact_write,
            chat::artifact_open_external,
            chat::artifact_reveal,
            chat::artifact_list,
            chat::artifact_search,
            // 可运行项目 (板块⑮): 一键启动前后端 + 内嵌预览
            project::project_list,
            project::project_status,
            project::project_run,
            project::project_stop,
            // CLAUDE.md
            claude_md::claude_md_list_projects,
            claude_md::claude_md_kb_info,
            claude_md::claude_md_read,
            claude_md::claude_md_write,
            // Skills
            skills::list_skills,
            skills::get_skill,
            skills::create_skill,
            skills::install_skill,
            skills::import_skill,
            skills::delete_skill,
            // API 供应商坞 + 用量看板
            provider::provider_list,
            provider::provider_switch,
            provider::provider_save,
            provider::provider_delete,
            provider::usage_summary,
            provider::codex_status,
            provider::codex_start_login,
            provider::codex_poll_login,
            codex_proxy::codex_proxy_info,
            // Forge 跨平台渲染能力 preflight（能出 PPT/视频吗、缺啥降级，三平台各报各的阶梯）
            forge::forge_preflight,
            // Forge 渲染引擎首落地：deck 截图 → 纯 Rust OOXML 打 .pptx（替 pptxgenjs，三平台同一份）
            forge::forge_build_pptx,
            forge::forge_screenshot,
            forge::forge_deck_to_pptx,
            forge::forge_deck_to_video,
            forge::forge_tts,
            // 环境医生 (环境监测 + 配置安装)
            doctor::env_check,
            doctor::env_fix_path,
            doctor::env_install_claude,
            doctor::env_install_node,
            doctor::env_install_pwsh,
            doctor::env_claude_update_check,
            doctor::env_update_claude,
            doctor::env_cancel,
            // 自动更新状态机 (借鉴 OpenCode updater-controller: 单飞 + 可观测 + 持久化续提示)
            updater::updater_get_state,
            updater::updater_check,
            updater::updater_apply,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Polaris application")
        .run(|_app, event| {
            // App 退出 (关窗 / 主动退出) 时回收所有在飞的 claude 子进程树, 防孤儿继续占端口/CPU。
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                chat::kill_all_children();
            }
        });
}
