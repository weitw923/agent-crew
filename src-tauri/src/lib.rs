mod commands;
pub mod db;
mod dsh;
mod llm;
pub mod models;
pub mod orchestrator;
mod prompts;
mod schedule;
pub mod storage;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use tauri::Manager;

/// 应用全局状态：可切换的存储位置、应用配置目录、运行中的任务集合。
pub struct AppState {
    pub db_path: Mutex<PathBuf>,
    pub storage_root: Mutex<PathBuf>,
    pub app_data_dir: PathBuf,
    pub running: Mutex<HashSet<i64>>,
}

impl AppState {
    pub fn db_path(&self) -> Result<PathBuf, String> {
        self.db_path
            .lock()
            .map(|p| p.clone())
            .map_err(|_| "数据库路径锁异常".to_string())
    }

    pub fn storage_root(&self) -> Result<PathBuf, String> {
        self.storage_root
            .lock()
            .map(|p| p.clone())
            .map_err(|_| "存储路径锁异常".to_string())
    }

    pub fn update_storage(&self, root: PathBuf) -> Result<(), String> {
        let db = root.join("orchestrator.db");
        *self
            .storage_root
            .lock()
            .map_err(|_| "存储路径锁异常".to_string())? = root;
        *self
            .db_path
            .lock()
            .map_err(|_| "数据库路径锁异常".to_string())? = db;
        Ok(())
    }
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
            let resolved = storage::resolve(&app_data_dir);
            std::fs::create_dir_all(&resolved.root)?;
            let db_path = db::init(&resolved.root)?;

            // 重启恢复 + 运行时设置。
            if let Ok(conn) = db::open(&db_path) {
                if let Ok(mut settings) = db::get_settings(&conn) {
                    db::set_log_persistence(settings.log_persistence);
                    let rpm = settings
                        .llm_rate_limit_per_min
                        .trim()
                        .parse::<u64>()
                        .unwrap_or(0);
                    llm::set_rate_limit_per_min(rpm);
                    let effective_root = resolved.root.to_string_lossy().to_string();
                    if settings.storage_root != effective_root {
                        settings.storage_root = effective_root;
                        let _ = db::save_settings(&conn, &settings);
                    }
                }
                let _ = db::recover_interrupted(&conn);
            }

            app.manage(AppState {
                db_path: Mutex::new(db_path),
                storage_root: Mutex::new(resolved.root),
                app_data_dir,
                running: Mutex::new(HashSet::new()),
            });

            // 定时/并发调度器：每 20 秒检查一次；窗口内自动启动/恢复任务。
            let scheduler_handle = app.handle().clone();
            std::thread::Builder::new()
                .name("agentcrew-scheduler".into())
                .spawn(move || loop {
                    std::thread::sleep(Duration::from_secs(20));
                    orchestrator::scheduler_tick(&scheduler_handle);
                })?;
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
            commands::append_requirement,
            commands::get_task_detail,
            commands::list_task_events,
            commands::delete_task,
            commands::human_force_pass,
            commands::extend_subtask_rounds,
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
            commands::pause_task,
            commands::terminate_task,
            commands::update_requirements,
            commands::send_chat_message,
            commands::clear_chat_messages,
            commands::get_settings,
            commands::save_settings,
        ])
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}
