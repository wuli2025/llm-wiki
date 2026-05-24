mod chat;
mod claude_md;
mod conv;
mod kb;
mod sandbox;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let h = app.handle();
            kb::init(h).map_err(|e| -> Box<dyn std::error::Error> { e.to_string().into() })?;
            sandbox::init(h)
                .map_err(|e| -> Box<dyn std::error::Error> { e.to_string().into() })?;
            conv::init(h).map_err(|e| -> Box<dyn std::error::Error> { e.to_string().into() })?;
            chat::init(h).map_err(|e| -> Box<dyn std::error::Error> { e.to_string().into() })?;
            claude_md::init(h)
                .map_err(|e| -> Box<dyn std::error::Error> { e.to_string().into() })?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // KB
            kb::kb_root,
            kb::kb_default_root,
            kb::kb_set_root,
            kb::kb_scan,
            kb::kb_list,
            kb::kb_read,
            kb::kb_search,
            kb::kb_ingest,
            kb::kb_graph,
            // Sandbox
            sandbox::sandbox_status,
            sandbox::sandbox_build_image,
            sandbox::sandbox_start,
            sandbox::sandbox_stop,
            sandbox::sandbox_exec,
            // Conv (项目 + 对话历史)
            conv::conv_list_projects,
            conv::conv_create_project,
            conv::conv_archive_project,
            conv::conv_list_conversations,
            conv::conv_create_conversation,
            conv::conv_delete_conversation,
            conv::conv_rename_conversation,
            conv::conv_get_messages,
            // Chat
            chat::chat_send,
            chat::chat_cancel,
            // CLAUDE.md
            claude_md::claude_md_list_projects,
            claude_md::claude_md_kb_info,
            claude_md::claude_md_read,
            claude_md::claude_md_write,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Polaris application");
}
