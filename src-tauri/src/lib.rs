mod commands;
pub mod db;
mod dsh;
mod llm;
pub mod models;
pub mod orchestrator;
mod prompts;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;

use tauri::Manager;

/// 应用全局状态：数据库位置、运行中的任务集合。
pub struct AppState {
    pub db_path: PathBuf,
    pub app_data_dir: PathBuf,
    pub running: Mutex<HashSet<i64>>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let app_data_dir = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("无法定位应用数据目录: {e}"))?;
            std::fs::create_dir_all(&app_data_dir)?;
            let db_path = db::init(&app_data_dir)?;

            // 重启恢复：优雅地把中断任务标记为 failed。
            if let Ok(conn) = db::open(&db_path) {
                let _ = db::recover_interrupted(&conn);
            }

            app.manage(AppState {
                db_path,
                app_data_dir,
                running: Mutex::new(HashSet::new()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_agents,
            commands::save_agent,
            commands::delete_agent,
            commands::export_agents,
            commands::import_agents,
            commands::list_tasks,
            commands::create_task,
            commands::start_task,
            commands::resume_task,
            commands::get_task_detail,
            commands::delete_task,
            commands::human_force_pass,
            commands::human_reject,
            commands::global_human_force_pass,
            commands::global_human_rework,
            commands::update_task_config,
            commands::approve_plan,
            commands::reject_plan,
            commands::approve_design,
            commands::reject_design,
            commands::send_human_message,
            commands::cancel_task,
            commands::get_settings,
            commands::save_settings,
        ])
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}
