//! Tauri command 层：前端唯一入口。所有业务状态变更都落在 SQLite。

use std::path::PathBuf;

use serde_json::json;
use tauri::{AppHandle, Manager, State};

use crate::db;
use crate::models::{
    AgentConfig, AgentInput, AppSettings, ChatMessage, NewTaskInput, StartOutcome, TaskConfigInput,
    TaskDetail, TaskMain, TaskSummary,
};
use crate::orchestrator;
use crate::storage;
use crate::AppState;

fn ensure_not_running(app: &AppHandle, task_id: i64) -> Result<(), String> {
    let state = app.state::<AppState>();
    let running = state.running.lock().map_err(|_| "任务锁异常".to_string())?;
    if running.contains(&task_id) {
        return Err("任务正在执行中，请等待当前步骤结束".into());
    }
    Ok(())
}

// ------------------------------------------------------------------
// Agent 配置
// ------------------------------------------------------------------
#[tauri::command]
pub fn list_agents(state: State<'_, AppState>) -> Result<Vec<AgentConfig>, String> {
    let conn = db::open(&state.db_path()?)?;
    db::list_agents(&conn)
}

#[tauri::command]
pub fn save_agent(state: State<'_, AppState>, input: AgentInput) -> Result<AgentConfig, String> {
    let conn = db::open(&state.db_path()?)?;
    db::save_agent(&conn, &input)
}

#[tauri::command]
pub fn delete_agent(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let conn = db::open(&state.db_path()?)?;
    db::delete_agent(&conn, id)
}

#[tauri::command]
pub fn export_agents(state: State<'_, AppState>, path: String) -> Result<usize, String> {
    let conn = db::open(&state.db_path()?)?;
    let agents = db::list_agents(&conn)?;
    let payload = json!({
        "version": 1,
        "exported_at": db::now(),
        "agents": agents,
    });
    let text = serde_json::to_string_pretty(&payload)
        .map_err(|e| format!("序列化 Agent 配置失败: {e}"))?;
    std::fs::write(&path, text).map_err(|e| format!("写入导出文件失败（{path}）: {e}"))?;
    Ok(agents.len())
}

#[tauri::command]
pub fn import_agents(state: State<'_, AppState>, path: String) -> Result<usize, String> {
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("读取导入文件失败（{path}）: {e}"))?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("导入文件不是合法 JSON: {e}"))?;
    // 兼容两种格式：{"agents":[...]} 或直接 [...]
    let arr = value
        .get("agents")
        .and_then(|v| v.as_array())
        .cloned()
        .or_else(|| value.as_array().cloned())
        .ok_or_else(|| "导入文件缺少 agents 数组".to_string())?;

    let conn = db::open(&state.db_path()?)?;
    let mut count = 0usize;
    for item in arr {
        let mut input: AgentInput =
            serde_json::from_value(item).map_err(|e| format!("Agent 配置字段不合法: {e}"))?;
        input.id = None;
        if input.role_type != "reviewer" && input.role_type != "coder" {
            return Err(format!("导入的 Agent「{}」角色类型不合法", input.name));
        }
        db::save_agent(&conn, &input)?;
        count += 1;
    }
    Ok(count)
}

// ------------------------------------------------------------------
// 任务
// ------------------------------------------------------------------
#[tauri::command]
pub fn list_tasks(state: State<'_, AppState>) -> Result<Vec<TaskSummary>, String> {
    let conn = db::open(&state.db_path()?)?;
    db::list_tasks(&conn)
}

#[tauri::command]
pub fn create_task(state: State<'_, AppState>, input: NewTaskInput) -> Result<i64, String> {
    let conn = db::open(&state.db_path()?)?;
    let reviewer = db::get_agent(&conn, input.reviewer_agent_id)?
        .ok_or_else(|| "评审 Agent 不存在".to_string())?;
    if reviewer.role_type != "reviewer" {
        return Err("所选评审 Agent 角色不是 reviewer".into());
    }
    for coder_id in &input.coder_agent_ids {
        let coder = db::get_agent(&conn, *coder_id)?
            .ok_or_else(|| format!("Coder Agent #{coder_id} 不存在"))?;
        if coder.role_type != "coder" {
            return Err(format!("Agent「{}」不是 coder 角色", coder.name));
        }
    }
    let work_dir = PathBuf::from(input.work_dir.trim());
    if work_dir.as_os_str().is_empty() {
        return Err("项目本地工作目录不能为空".into());
    }

    let task_id = db::create_task(&conn, &input)?;
    let mode_label = match input.execution_mode.as_str() {
        "immediate" => "立即执行",
        "scheduled" => "定时执行",
        _ => "稍后执行",
    };
    db::add_log(
        &conn,
        task_id,
        "info",
        &format!(
            "任务创建：{}；评审 Agent「{}」；执行模式：{}；串行 Coder 使用列表第一个",
            input.task_name, reviewer.name, mode_label
        ),
    )?;
    Ok(task_id)
}

#[tauri::command]
pub fn start_task(app: AppHandle, task_id: i64) -> Result<StartOutcome, String> {
    orchestrator::start_task(&app, task_id)
}

#[tauri::command]
pub fn append_requirement(
    app: AppHandle,
    task_id: i64,
    requirement: String,
    mode: String,
) -> Result<(), String> {
    ensure_not_running(&app, task_id)?;
    orchestrator::append_requirement(Some(&app), task_id, &requirement, &mode)
}

#[tauri::command]
pub fn resume_task(app: AppHandle, task_id: i64) -> Result<(), String> {
    ensure_not_running(&app, task_id)?;
    orchestrator::resume_task(Some(&app), task_id)
}

#[tauri::command]
pub fn get_task_detail(state: State<'_, AppState>, task_id: i64) -> Result<TaskDetail, String> {
    let conn = db::open(&state.db_path()?)?;
    db::get_task_detail(&conn, task_id)
}

#[tauri::command]
pub fn list_task_events(
    state: State<'_, AppState>,
    task_id: i64,
) -> Result<Vec<crate::models::TaskEvent>, String> {
    let conn = db::open(&state.db_path()?)?;
    db::list_all_events(&conn, task_id)
}

#[tauri::command]
pub fn list_iterations(
    state: State<'_, AppState>,
    task_id: i64,
) -> Result<Vec<crate::models::Iteration>, String> {
    let conn = db::open(&state.db_path()?)?;
    db::list_iterations(&conn, task_id)
}

#[tauri::command]
pub fn get_iteration_detail(
    state: State<'_, AppState>,
    task_id: i64,
    iteration_id: i64,
) -> Result<crate::models::IterationDetail, String> {
    let conn = db::open(&state.db_path()?)?;
    db::get_iteration_detail(&conn, task_id, iteration_id)
}

#[tauri::command]
pub fn delete_task(app: AppHandle, task_id: i64) -> Result<(), String> {
    ensure_not_running(&app, task_id)?;
    let state = app.state::<AppState>();
    let conn = db::open(&state.db_path()?)?;
    db::delete_task(&conn, task_id)
}

#[tauri::command]
pub fn human_force_pass(app: AppHandle, task_id: i64, subtask_id: i64) -> Result<(), String> {
    ensure_not_running(&app, task_id)?;
    orchestrator::human_force_pass(Some(&app), task_id, subtask_id)
}

#[tauri::command]
pub fn extend_subtask_rounds(
    app: AppHandle,
    task_id: i64,
    subtask_id: i64,
    extra_rounds: i64,
) -> Result<(), String> {
    ensure_not_running(&app, task_id)?;
    orchestrator::extend_subtask_rounds(&app, task_id, subtask_id, extra_rounds)
}

#[tauri::command]
pub fn human_reject(
    app: AppHandle,
    task_id: i64,
    subtask_id: i64,
    feedback: String,
) -> Result<(), String> {
    ensure_not_running(&app, task_id)?;
    orchestrator::human_reject(Some(&app), task_id, subtask_id, &feedback)
}

#[tauri::command]
pub fn global_human_force_pass(app: AppHandle, task_id: i64) -> Result<(), String> {
    ensure_not_running(&app, task_id)?;
    orchestrator::global_human_force_pass(Some(&app), task_id)
}

#[tauri::command]
pub fn global_human_rework(
    app: AppHandle,
    task_id: i64,
    subtask_id: i64,
    feedback: String,
) -> Result<(), String> {
    ensure_not_running(&app, task_id)?;
    orchestrator::global_human_rework(Some(&app), task_id, subtask_id, &feedback)
}

// ------------------------------------------------------------------
// 运行中配置 / 方案与设计审批 / 人工消息 / 终止
// ------------------------------------------------------------------
#[tauri::command]
pub fn update_task_config(
    app: AppHandle,
    task_id: i64,
    input: TaskConfigInput,
) -> Result<TaskMain, String> {
    // 允许任务运行中修改，不要求停止调度；下一次状态机迭代会重新读取配置。
    orchestrator::update_runtime_config(Some(&app), task_id, &input)
}

#[tauri::command]
pub fn approve_plan(app: AppHandle, task_id: i64) -> Result<(), String> {
    ensure_not_running(&app, task_id)?;
    orchestrator::approve_plan(Some(&app), task_id)
}

#[tauri::command]
pub fn reject_plan(app: AppHandle, task_id: i64, feedback: String) -> Result<(), String> {
    ensure_not_running(&app, task_id)?;
    orchestrator::reject_plan(Some(&app), task_id, &feedback)
}

#[tauri::command]
pub fn approve_design(app: AppHandle, task_id: i64, subtask_id: i64) -> Result<(), String> {
    ensure_not_running(&app, task_id)?;
    orchestrator::approve_design(Some(&app), task_id, subtask_id)
}

#[tauri::command]
pub fn reject_design(
    app: AppHandle,
    task_id: i64,
    subtask_id: i64,
    feedback: String,
) -> Result<(), String> {
    ensure_not_running(&app, task_id)?;
    orchestrator::reject_design(Some(&app), task_id, subtask_id, &feedback)
}

#[tauri::command]
pub fn send_human_message(app: AppHandle, task_id: i64, content: String) -> Result<(), String> {
    orchestrator::send_human_message(Some(&app), task_id, &content)
}

#[tauri::command]
pub fn cancel_task(app: AppHandle, task_id: i64) -> Result<(), String> {
    orchestrator::cancel_task(&app, task_id)
}

#[tauri::command]
pub fn pause_task(app: AppHandle, task_id: i64) -> Result<(), String> {
    orchestrator::pause_task(&app, task_id)
}

#[tauri::command]
pub fn terminate_task(app: AppHandle, task_id: i64) -> Result<(), String> {
    orchestrator::terminate_task(&app, task_id)
}

#[tauri::command]
pub fn update_requirements(
    app: AppHandle,
    task_id: i64,
    top_requirement: String,
    accept_criteria: String,
) -> Result<TaskMain, String> {
    orchestrator::update_requirements(&app, task_id, &top_requirement, &accept_criteria)
}

#[tauri::command]
pub async fn send_chat_message(
    app: AppHandle,
    task_id: i64,
    content: String,
) -> Result<ChatMessage, String> {
    // LLM HTTP 请求放到阻塞线程，避免卡住 Tauri IPC / 页面。
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        orchestrator::send_chat_message(&handle, task_id, &content)
    })
    .await
    .map_err(|e| format!("临时问答线程异常: {e}"))?
}

#[tauri::command]
pub fn clear_chat_messages(app: AppHandle, task_id: i64) -> Result<(), String> {
    orchestrator::clear_chat_messages(&app, task_id)
}

// ------------------------------------------------------------------
// 设置
// ------------------------------------------------------------------
#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {
    let conn = db::open(&state.db_path()?)?;
    let mut settings = db::get_settings(&conn)?;
    settings.storage_root = state.storage_root()?.to_string_lossy().to_string();
    Ok(settings)
}

#[tauri::command]
pub fn save_settings(
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    if settings
        .llm_timeout_sec
        .trim()
        .parse::<u64>()
        .map(|v| v < 5)
        .unwrap_or(true)
    {
        return Err("LLM 调用超时必须是 >=5 的整数秒".into());
    }
    if settings
        .dsh_timeout_sec
        .trim()
        .parse::<u64>()
        .map(|v| v < 30)
        .unwrap_or(true)
    {
        return Err("dsh 超时必须是 >=30 的整数秒".into());
    }
    if settings.dsh_bin.trim().is_empty() {
        return Err("dsh 可执行文件不能为空".into());
    }
    if !matches!(
        settings.default_execution_mode.as_str(),
        "immediate" | "later" | "scheduled"
    ) {
        return Err("默认执行模式不合法".into());
    }
    if settings
        .max_concurrent_tasks
        .trim()
        .parse::<usize>()
        .map(|v| !(1..=32).contains(&v))
        .unwrap_or(true)
    {
        return Err("最大并发任务数必须是 1–32 的整数".into());
    }
    if settings
        .llm_rate_limit_per_min
        .trim()
        .parse::<u64>()
        .is_err()
    {
        return Err("LLM 每分钟请求上限必须是 >=0 的整数".into());
    }
    for raw in [
        &settings.default_module_max_round,
        &settings.default_global_max_round,
    ] {
        if raw.trim().parse::<i64>().map(|v| v < 1).unwrap_or(true) {
            return Err("默认评审轮次必须是 >=1 的整数".into());
        }
    }

    let current_root = state.storage_root()?;
    let current_db = state.db_path()?;
    let target_root = if settings.storage_root.trim().is_empty() {
        state.app_data_dir.clone()
    } else {
        PathBuf::from(settings.storage_root.trim())
    };
    if !target_root.is_absolute() {
        return Err("存储路径必须是绝对路径".into());
    }

    // 存储目录不能落在任一项目工作目录内部，避免任务数据和代码混在一起。
    {
        let conn = db::open(&current_db)?;
        for task in db::list_tasks(&conn)? {
            let work_dir = PathBuf::from(task.task.work_dir.trim());
            if !work_dir.as_os_str().is_empty() && target_root.starts_with(&work_dir) {
                return Err(format!(
                    "存储目录不能位于任务项目目录内部：{}",
                    work_dir.display()
                ));
            }
        }
    }

    let mut effective = settings;
    if target_root != current_root {
        let _running_guard = state.running.lock().map_err(|_| "任务锁异常".to_string())?;
        if !_running_guard.is_empty() {
            return Err("仍有任务在运行，请先暂停所有任务再迁移存储目录".into());
        }
        storage::migrate_root(&current_root, &target_root, &current_db)?;
        storage::write_pointer(&state.app_data_dir, &target_root)?;
        state.update_storage(target_root.clone())?;
        effective.storage_root = target_root.to_string_lossy().to_string();
    } else {
        effective.storage_root = current_root.to_string_lossy().to_string();
    }

    let db_path = state.db_path()?;
    let conn = db::open(&db_path)?;
    db::save_settings(&conn, &effective)?;
    db::set_log_persistence(effective.log_persistence);
    let rpm = effective
        .llm_rate_limit_per_min
        .trim()
        .parse::<u64>()
        .unwrap_or(0);
    crate::llm::set_rate_limit_per_min(rpm);
    db::get_settings(&conn)
}
