//! 编排状态机。
//!
//! 持久化状态机 + 串行调度：
//!   planning -> module_dev（当前子任务 coding -> code_review -> summary_generation -> summary_review）
//!   -> 全部模块完成 -> global_review ->（发现问题则模块返工回到 module_dev）
//!   -> 通过 / 轮次耗尽人工介入。
//!
//! 所有阶段都从 SQLite 恢复，因此应用重启后可以「恢复调度」继续。

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::db;
use crate::dsh;
use crate::llm;
use crate::models::{
    AgentConfig, AppSettings, ChatMessage, GlobalReviewIssue, GlobalReviewRecord, PendingPlan,
    StartOutcome, Subtask, TaskConfigInput, TaskMain,
};
use crate::prompts;
use crate::schedule;
use crate::storage;
use crate::AppState;

static RUN_SEQ: AtomicI64 = AtomicI64::new(1);

const MAX_STATE_STEPS: usize = 600;
const MAX_CODE_DIFF_CHARS: usize = 90_000;
const MAX_GLOBAL_DIFF_CHARS: usize = 140_000;

// ------------------------------------------------------------------
// 人工终止信号注册表（模块级，避免把 Arc 穿过所有函数签名）
// ------------------------------------------------------------------
fn cancel_registry() -> &'static Mutex<HashMap<i64, Arc<AtomicBool>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<i64, Arc<AtomicBool>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_cancel(task_id: i64) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    if let Ok(mut map) = cancel_registry().lock() {
        map.insert(task_id, flag.clone());
    }
    flag
}

fn cancel_flag(task_id: i64) -> Arc<AtomicBool> {
    if let Ok(map) = cancel_registry().lock() {
        if let Some(flag) = map.get(&task_id) {
            return flag.clone();
        }
    }
    Arc::new(AtomicBool::new(false))
}

fn remove_cancel(task_id: i64) {
    if let Ok(mut map) = cancel_registry().lock() {
        map.remove(&task_id);
    }
}

/// 供 command 层调用：请求终止某个任务。
pub fn request_cancel(task_id: i64) {
    let flag = cancel_flag(task_id);
    flag.store(true, Ordering::SeqCst);
}

fn is_canceled(task_id: i64) -> bool {
    cancel_flag(task_id).load(Ordering::SeqCst)
}

// ------------------------------------------------------------------
// 执行事件 / LLM 调用审计 / 人工消息注入
// ------------------------------------------------------------------
fn record_event(
    app: Option<&AppHandle>,
    conn: &Connection,
    task_id: i64,
    subtask_id: Option<i64>,
    kind: &str,
    content: &str,
) {
    if let Ok(event) = db::add_event(conn, task_id, subtask_id, kind, content) {
        if let Some(handle) = app {
            let _ = handle.emit("task-event", json!({ "task_id": task_id, "event": event }));
        }
    }
}

fn human_instructions(app: Option<&AppHandle>, conn: &Connection, task_id: i64) -> String {
    match db::consume_human_messages(conn, task_id) {
        Ok(messages) if !messages.is_empty() => {
            let text = messages
                .iter()
                .map(|m| format!("- {m}"))
                .collect::<Vec<_>>()
                .join("\n");
            record_event(app, conn, task_id, None, "human_message_delivered", &text);
            text
        }
        _ => String::new(),
    }
}

#[allow(clippy::too_many_arguments)]
fn llm_json(
    app: Option<&AppHandle>,
    conn: &Connection,
    task_id: i64,
    subtask_id: Option<i64>,
    stage: &str,
    agent: &AgentConfig,
    system: &str,
    user: &str,
    llm_timeout_sec: &str,
) -> Result<Value, String> {
    record_event(
        app,
        conn,
        task_id,
        subtask_id,
        &format!("llm_request:{stage}"),
        &format!(
            "Agent: {}\nModel: {}\n\n===== SYSTEM =====\n{system}\n\n===== USER =====\n{user}",
            agent.name, agent.model_name
        ),
    );
    let result =
        llm::chat_json_cancellable(agent, system, user, llm_timeout_sec, cancel_flag(task_id));
    match &result {
        Ok(value) => {
            let pretty = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
            record_event(
                app,
                conn,
                task_id,
                subtask_id,
                &format!("llm_response:{stage}"),
                &pretty,
            );
        }
        Err(err) => {
            record_event(
                app,
                conn,
                task_id,
                subtask_id,
                &format!("llm_error:{stage}"),
                err,
            );
        }
    }
    result
}

// ------------------------------------------------------------------
// 启动调度线程
// ------------------------------------------------------------------
pub fn spawn_task(app: AppHandle, task_id: i64) -> Result<(), String> {
    {
        let state = app.state::<AppState>();
        let mut running = state.running.lock().map_err(|_| "任务锁异常".to_string())?;
        if running.contains(&task_id) {
            return Err("该任务正在执行中".into());
        }
        running.insert(task_id);
    }
    register_cancel(task_id);

    let db_path = app.state::<AppState>().db_path()?;
    let app_data_dir = app.state::<AppState>().storage_root()?;
    let handle = app.clone();
    let seq = RUN_SEQ.fetch_add(1, Ordering::Relaxed);
    let spawn_result = thread::Builder::new()
        .name(format!("orchestrator-task-{task_id}-{seq}"))
        .spawn(move || {
            let result = run_loop(Some(&handle), &db_path, &app_data_dir, task_id);
            if let Err(err) = &result {
                if let Ok(conn) = db::open(&db_path) {
                    if is_canceled(task_id) {
                        let terminated = db::get_task(&conn, task_id)
                            .ok()
                            .flatten()
                            .map(|t| t.status == "terminated")
                            .unwrap_or(false);
                        if !terminated {
                            let _ =
                                db::set_paused(&conn, task_id, "任务已暂停，可点击「恢复」继续");
                        }
                    } else if let Ok(Some(mut task)) = db::get_task(&conn, task_id) {
                        if !matches!(
                            task.status.as_str(),
                            "completed" | "needs_human" | "paused" | "terminated"
                        ) {
                            task.status = "failed".to_string();
                            let _ = db::save_task(&conn, &task);
                        }
                        let _ = db::add_log(&conn, task_id, "error", &format!("调度终止：{err}"));
                    }
                }
                let _ = handle.emit("task-updated", json!({ "task_id": task_id }));
            }

            if let Ok(conn) = db::open(&db_path) {
                let _ = db::set_cancel_requested(&conn, task_id, false);
            }
            remove_cancel(task_id);
            if let Some(state) = handle.try_state::<AppState>() {
                if let Ok(mut running) = state.running.lock() {
                    running.remove(&task_id);
                }
            }
            let _ = handle.emit("task-updated", json!({ "task_id": task_id }));
        });

    if let Err(e) = spawn_result {
        remove_cancel(task_id);
        if let Some(state) = app.try_state::<AppState>() {
            if let Ok(mut running) = state.running.lock() {
                running.remove(&task_id);
            }
        }
        return Err(format!("创建调度线程失败: {e}"));
    }
    Ok(())
}

fn emit_update(app: Option<&AppHandle>, task_id: i64) {
    if let Some(handle) = app {
        let _ = handle.emit("task-updated", json!({ "task_id": task_id }));
    }
}

fn log(app: Option<&AppHandle>, conn: &Connection, task_id: i64, level: &str, message: &str) {
    let _ = db::add_log(conn, task_id, level, message);
    // 日志持久化关闭时，仍然把关键日志作为实时事件推送到页面。
    if !db::log_persistence() {
        if let Some(handle) = app {
            let event =
                db::live_event(task_id, None, "system_log", &format!("[{level}] {message}"));
            let _ = handle.emit("task-event", json!({ "task_id": task_id, "event": event }));
        }
    }
    emit_update(app, task_id);
}

fn parse_u64(raw: &str, default: u64, min: u64, max: u64) -> u64 {
    raw.trim().parse::<u64>().unwrap_or(default).clamp(min, max)
}

/// 将 Agent 自定义 systemPrompt 叠加在业务强约束 prompt 之前。
/// 业务 JSON 约束放在最后，保证模型优先遵守结构化输出。
fn tech_stack_text(task: &TaskMain) -> String {
    let stack = task.tech_stack.trim();
    if stack.is_empty() {
        "（历史任务未单独记录技术栈，请从全局契约和顶层需求推断，不要自行发挥）".to_string()
    } else {
        prompts::truncate_chars(stack, 12_000)
    }
}

fn requirements_text(task: &TaskMain) -> String {
    format!(
        "## 顶层需求\n{}\n\n## 项目验收标准\n{}",
        prompts::truncate_chars(&task.top_requirement, 20_000),
        prompts::truncate_chars(&task.global_accept_criteria, 8_000)
    )
}

fn agent_system(agent: &AgentConfig, base: &str) -> String {
    if agent.system_prompt.trim().is_empty() {
        base.to_string()
    } else {
        format!("{}\n\n{base}", agent.system_prompt.trim())
    }
}

// ------------------------------------------------------------------
// 主循环
// ------------------------------------------------------------------
/// 是否需要重新拆解。除正常 planning 外，还兜底旧版本恢复后只把 status
/// 改成 running、但契约和子任务都为空的损坏中间态。
fn should_replan(status: &str, has_contract: bool, no_subtasks: bool) -> bool {
    status == "planning" || (!has_contract && no_subtasks)
}

fn run_loop(
    app: Option<&AppHandle>,
    db_path: &Path,
    app_data_dir: &Path,
    task_id: i64,
) -> Result<(), String> {
    let settings = {
        let conn = db::open(db_path)?;
        db::get_settings(&conn)?
    };
    // 让前端先感知到调度开始。
    emit_update(app, task_id);

    for _ in 0..MAX_STATE_STEPS {
        let conn = db::open(db_path)?;
        let mut task = db::get_task(&conn, task_id)?.ok_or_else(|| "任务不存在".to_string())?;

        if matches!(
            task.status.as_str(),
            "completed" | "failed" | "needs_human" | "paused" | "terminated"
        ) {
            emit_update(app, task_id);
            return Ok(());
        }

        if is_canceled(task_id) {
            if task.status != "terminated" {
                db::set_paused(&conn, task_id, "任务已暂停，可点击「恢复调度」继续")?;
            }
            emit_update(app, task_id);
            return Ok(());
        }

        // 定时窗口边界：当前模块（含本轮评审/摘要）允许跑完；在派发下一个新模块前
        // 如果已经离开窗口，则自动暂停等待下一个窗口。
        if task.execution_mode == "scheduled"
            && task.schedule_enabled
            && !task.schedule_override
            && !schedule::now_in_window(&task.schedule_start, &task.schedule_end)
        {
            let active = db::list_subtasks(&conn, task_id)?
                .iter()
                .any(|s| s.status == "in_progress" || s.status == "needs_human");
            if !active {
                task.status = "paused".into();
                task.pause_reason = "waiting_window".into();
                db::save_task(&conn, &task)?;
                log(
                    app,
                    &conn,
                    task_id,
                    "info",
                    "已离开定时执行窗口：当前模块已收尾，等待下一个时间窗口自动继续",
                );
                emit_update(app, task_id);
                return Ok(());
            }
        }

        // 已完成任务追加需求：先由评审做增量梳理，不重新拆解整个项目。
        if task.task_phase == "requirement_intake" {
            do_requirement_intake(app, &conn, db_path, &settings, &mut task)?;
            continue;
        }

        // 拆解恢复保护：只要还没有全局契约且没有任何子任务，就一定是拆解未完成
        //（包括旧版本在 planning 阶段失败后只把 status 恢复成 running 的情况），
        // 此时必须重新拆解，不能直接进入模块队列检查。
        let has_contract = !task.global_contract.trim().is_empty();
        let no_subtasks = db::list_subtasks(&conn, task_id)?.is_empty();
        if should_replan(&task.status, has_contract, no_subtasks) {
            if task.status != "planning" {
                task.status = "planning".into();
                db::save_task(&conn, &task)?;
                log(
                    app,
                    &conn,
                    task_id,
                    "info",
                    "检测到拆解阶段未完成（无契约/无子任务），重新进入需求拆解",
                );
            }
            do_plan(app, &conn, db_path, app_data_dir, &settings, &mut task)?;
            continue;
        }

        // 任务启动时先落地 git 基线，供每轮 diff 与全局 diff 使用。
        if task.baseline_tree.trim().is_empty() {
            std::fs::create_dir_all(&task.work_dir)
                .map_err(|e| format!("创建工作目录失败（{}）: {e}", task.work_dir))?;
            dsh::ensure_git_repo(Path::new(&task.work_dir))?;
            task.baseline_tree = dsh::capture_tree(Path::new(&task.work_dir))?;
            db::save_task(&conn, &task)?;
            log(
                app,
                &conn,
                task_id,
                "info",
                &format!(
                    "已建立 git 基线（tree {}），用于计算模块 diff 与全局 diff",
                    task.baseline_tree
                ),
            );
            continue;
        }

        if task.task_phase == "module_dev" {
            let subtasks = db::list_subtasks(&conn, task_id)?;

            if subtasks.iter().any(|s| s.status == "needs_human") {
                task.status = "needs_human".into();
                db::save_task(&conn, &task)?;
                log(
                    app,
                    &conn,
                    task_id,
                    "warn",
                    "存在需要人工介入的子任务，调度暂停",
                );
                return Ok(());
            }

            if let Some(sub) = subtasks.iter().find(|s| s.status == "in_progress").cloned() {
                // 兼容旧任务/运行中打开人工审核：Coder 尚未开工的当前模块补生成设计并进入审批。
                if task.manual_review_enabled
                    && sub.design_content.trim().is_empty()
                    && sub.current_stage == "coding"
                {
                    let mut sub = sub;
                    sub.current_stage = "design_generation".into();
                    db::save_subtask(&conn, &sub)?;
                    log(
                        app,
                        &conn,
                        task_id,
                        "info",
                        &format!(
                            "人工审核已开启：为子任务 #{}「{}」补生成模块设计文档并等待审批",
                            sub.sort_index, sub.subtask_name
                        ),
                    );
                    continue;
                }
                do_module_step(app, &conn, app_data_dir, &settings, &mut task, sub)?;
                continue;
            }

            if let Some(sub) = subtasks.iter().find(|s| s.status == "waiting").cloned() {
                let mut sub = sub;
                sub.status = "in_progress".into();
                sub.current_round = 1;
                sub.current_stage = if sub.design_content.trim().is_empty() {
                    "design_generation".into()
                } else {
                    "coding".into()
                };
                db::save_subtask(&conn, &sub)?;
                let coder_name = sub
                    .assigned_coder_id
                    .and_then(|id| db::get_agent(&conn, id).ok().flatten())
                    .map(|a| a.name)
                    .unwrap_or_else(|| "未分配".into());
                log(
                    app,
                    &conn,
                    task_id,
                    "info",
                    &format!(
                        "派发子任务 #{}「{}」：先由技术总监生成模块设计，再交给 Coder「{}」；第 1/{} 轮",
                        sub.sort_index, sub.subtask_name, coder_name, sub.max_round
                    ),
                );
                continue;
            }

            if !subtasks.is_empty() && subtasks.iter().all(|s| s.status == "completed") {
                task.task_phase = "global_review".into();
                task.status = "running".into();
                db::save_task(&conn, &task)?;
                log(
                    app,
                    &conn,
                    task_id,
                    "info",
                    "全部子任务已完成，进入【全局评审收口阶段】",
                );
                continue;
            }

            return Err("子任务队列状态异常：没有等待/进行中的模块，也没有全部完成".into());
        }

        if task.task_phase == "global_review" {
            let subtasks = db::list_subtasks(&conn, task_id)?;
            if subtasks.iter().any(|s| s.status != "completed") {
                task.task_phase = "module_dev".into();
                task.status = "running".into();
                db::save_task(&conn, &task)?;
                log(
                    app,
                    &conn,
                    task_id,
                    "info",
                    "全局返工模块尚未全部完成，回到模块开发阶段",
                );
                continue;
            }
            do_global_step(
                app,
                &conn,
                db_path,
                app_data_dir,
                &settings,
                &mut task,
                &subtasks,
            )?;
            continue;
        }

        return Err(format!("未知 task_phase: {}", task.task_phase));
    }

    Err(format!(
        "状态机超过 {} 步保护上限，已停止以避免死循环",
        MAX_STATE_STEPS
    ))
}

// ------------------------------------------------------------------
// 阶段 1：拆解需求
// ------------------------------------------------------------------
#[derive(Debug, Deserialize)]
struct PlannerItem {
    name: String,
    #[serde(default, alias = "acceptance_criteria")]
    accept_criteria: String,
}

#[derive(Debug, Deserialize)]
struct PlannerOutput {
    #[serde(default)]
    tech_stack: String,
    global_contract: String,
    subtasks: Vec<PlannerItem>,
}

fn do_plan(
    app: Option<&AppHandle>,
    conn: &Connection,
    db_path: &Path,
    _app_data_dir: &Path,
    settings: &AppSettings,
    task: &mut TaskMain,
) -> Result<(), String> {
    log(
        app,
        conn,
        task.id,
        "info",
        "评审 Agent 开始拆解顶层需求、选择技术栈并生成全局契约…",
    );
    let reviewer = db::get_agent(conn, task.reviewer_agent_id)?
        .ok_or_else(|| "评审 Agent 不存在".to_string())?;
    if reviewer.role_type != "reviewer" {
        return Err("任务绑定的 Agent 不是 Reviewer".into());
    }

    let human = human_instructions(app, conn, task.id);
    let base_user = prompts::planner_user(
        &task.task_name,
        &task.top_requirement,
        &task.global_accept_criteria,
    );
    let base_user = if human.trim().is_empty() {
        base_user
    } else {
        format!("{base_user}\n\n# 人工实时介入消息（必须遵守）\n{human}")
    };
    let mut user = base_user.clone();
    let mut parsed: Option<PlannerOutput> = None;
    let mut last_error = String::new();

    for attempt in 0..2 {
        let system = agent_system(&reviewer, &prompts::planner_system());
        let value = llm_json(
            app,
            conn,
            task.id,
            None,
            "planner",
            &reviewer,
            &system,
            &user,
            &settings.llm_timeout_sec,
        )?;
        match validate_plan(value) {
            Ok(plan) => {
                parsed = Some(plan);
                break;
            }
            Err(e) => {
                last_error = e.clone();
                log(
                    app,
                    conn,
                    task.id,
                    "warn",
                    &format!("评审拆解输出语义不合法（第 {} 次）：{}", attempt + 1, e),
                );
                user = format!(
                    "{base_user}\n\n注意：你上一次输出不符合要求：{e}。请修正后重新只输出合法 JSON。"
                );
            }
        }
    }
    let plan = parsed.ok_or_else(|| format!("拆解结果多次校验失败：{last_error}"))?;
    record_event(
        app,
        conn,
        task.id,
        None,
        "agent_message:reviewer",
        &format!(
            "技术总监完成拆解：{} 个模块，技术栈与全局契约已生成。\n模块列表：{}",
            plan.subtasks.len(),
            plan.subtasks
                .iter()
                .enumerate()
                .map(|(i, s)| format!("#{} {}", i + 1, s.name))
                .collect::<Vec<_>>()
                .join("；")
        ),
    );

    let first_coder = task
        .coder_agent_ids
        .first()
        .copied()
        .ok_or_else(|| "任务未绑定 Coder".to_string())?;
    let coder =
        db::get_agent(conn, first_coder)?.ok_or_else(|| "任务绑定的 Coder 不存在".to_string())?;
    if coder.role_type != "coder" {
        return Err("任务绑定的 Agent 不是 Coder 角色".into());
    }

    let pending = PendingPlan {
        tech_stack: plan.tech_stack.trim().to_string(),
        global_contract: plan.global_contract.trim().to_string(),
        subtasks: plan
            .subtasks
            .iter()
            .map(|i| crate::models::PendingPlanItem {
                name: i.name.trim().to_string(),
                accept_criteria: i.accept_criteria.trim().to_string(),
            })
            .collect(),
    };
    task.tech_stack = pending.tech_stack.clone();
    task.global_contract = pending.global_contract.clone();

    if task.manual_review_enabled {
        task.pending_plan_json =
            serde_json::to_string(&pending).map_err(|e| format!("方案序列化失败: {e}"))?;
        task.task_phase = "plan_approval".into();
        task.status = "needs_human".into();
        db::save_task(conn, task)?;
        record_event(
            app,
            conn,
            task.id,
            None,
            "plan_approval",
            &format!(
                "技术总监完成拆解：{} 个模块，技术栈与全局契约已生成，等待人工审批方案。",
                pending.subtasks.len()
            ),
        );
        log(
            app,
            conn,
            task.id,
            "warn",
            &format!(
                "拆解完成：共 {} 个模块，等待人工审批方案后才能创建子任务并开始开发",
                pending.subtasks.len()
            ),
        );
        return Ok(());
    }

    let mut conn_tx = db::open(db_path)?;
    db::apply_pending_plan(&mut conn_tx, task, &pending)?;
    record_event(
        app,
        conn,
        task.id,
        None,
        "plan_accepted",
        &format!(
            "技术总监完成拆解：技术栈、全局契约、{} 个模块已确认，开始串行开发。",
            pending.subtasks.len()
        ),
    );
    log(
        app,
        conn,
        task.id,
        "info",
        &format!(
            "拆解任务完成：共 {} 个子任务，技术栈/契约已生成，串行开发开始",
            pending.subtasks.len()
        ),
    );
    Ok(())
}

fn validate_plan(value: Value) -> Result<PlannerOutput, String> {
    let plan: PlannerOutput =
        serde_json::from_value(value).map_err(|e| format!("JSON 结构不符合 Planner 约定: {e}"))?;
    if plan.tech_stack.trim().is_empty() {
        return Err("tech_stack 技术栈说明不能为空".into());
    }
    if plan.global_contract.trim().len() < 10 {
        return Err("global_contract 太短或为空".into());
    }
    if plan.subtasks.is_empty() {
        return Err("subtasks 不能为空".into());
    }
    if plan.subtasks.len() > 30 {
        return Err("subtasks 超过 30 个，拆解粒度不合理".into());
    }
    let mut seen = std::collections::HashSet::new();
    for item in &plan.subtasks {
        if item.name.trim().is_empty() {
            return Err("存在空的子任务名称".into());
        }
        if item.accept_criteria.trim().is_empty() {
            return Err(format!("子任务「{}」缺少验收标准", item.name));
        }
        if !seen.insert(item.name.trim().to_string()) {
            return Err(format!("子任务名称重复：{}", item.name));
        }
    }
    Ok(plan)
}

// ------------------------------------------------------------------
// 阶段 1.5：已完成任务追加需求（增量梳理）
// ------------------------------------------------------------------
#[derive(Debug, Deserialize)]
struct IntakeOutput {
    decision: String,
    #[serde(default)]
    analysis: String,
    subtasks: Vec<PlannerItem>,
}

fn validate_intake(
    value: Value,
    existing_names: &[String],
    force_single: bool,
) -> Result<IntakeOutput, String> {
    let out: IntakeOutput = serde_json::from_value(value)
        .map_err(|e| format!("JSON 结构不符合增量需求梳理约定: {e}"))?;
    let decision = out.decision.trim().to_lowercase();
    if decision != "simple" && decision != "complex" {
        return Err("decision 必须是 simple 或 complex".into());
    }
    if out.analysis.trim().is_empty() {
        return Err("analysis 梳理结论不能为空".into());
    }
    if force_single && decision != "simple" {
        return Err("用户要求单模块快改时，decision 必须为 simple".into());
    }
    if force_single && out.subtasks.len() != 1 {
        return Err("用户要求单模块快改时，subtasks 必须恰好 1 个".into());
    }
    if decision == "simple" && out.subtasks.len() != 1 {
        return Err("简单需求必须只输出 1 个子任务，禁止过度拆解".into());
    }
    if decision == "complex" && !(2..=6).contains(&out.subtasks.len()) {
        return Err("复杂需求必须拆成 2-6 个子任务".into());
    }
    if out.subtasks.is_empty() {
        return Err("subtasks 不能为空".into());
    }
    if out.subtasks.len() > 6 {
        return Err("本次增量最多 6 个子任务，请合并相近的改动".into());
    }
    let mut seen = std::collections::HashSet::new();
    for item in &out.subtasks {
        let name = item.name.trim();
        if name.is_empty() {
            return Err("存在空的子任务名称".into());
        }
        if item.accept_criteria.trim().is_empty() {
            return Err(format!("子任务「{name}」缺少验收标准"));
        }
        if !seen.insert(name.to_string()) {
            return Err(format!("子任务名称重复：{name}"));
        }
        if existing_names.iter().any(|n| n.trim() == name) {
            return Err(format!("子任务名称与已完成模块重复：{name}"));
        }
    }
    Ok(out)
}

fn do_requirement_intake(
    app: Option<&AppHandle>,
    conn: &Connection,
    db_path: &Path,
    settings: &AppSettings,
    task: &mut TaskMain,
) -> Result<(), String> {
    let requirement = task.followup_requirement.trim().to_string();
    if requirement.is_empty() {
        return Err("没有待梳理的追加需求".into());
    }
    let reviewer = db::get_agent(conn, task.reviewer_agent_id)?
        .ok_or_else(|| "评审 Agent 不存在".to_string())?;
    if reviewer.role_type != "reviewer" {
        return Err("任务绑定的 Agent 不是 Reviewer".into());
    }
    let force_single = task.followup_mode == "single";
    log(
        app,
        conn,
        task.id,
        "info",
        if force_single {
            "评审开始梳理追加需求（单模块快改模式）：只创建一个子任务…"
        } else {
            "评审开始梳理追加需求：自动判断简单改动或复杂需求…"
        },
    );
    record_event(
        app,
        conn,
        task.id,
        None,
        "agent_message:reviewer",
        &format!(
            "收到追加需求：{}\n\n开始梳理增量范围；简单改动只创建 1 个子任务，不重新拆解已完成项目。",
            prompts::truncate_chars(&requirement, 3000)
        ),
    );

    let subtasks = db::list_subtasks(conn, task.id)?;
    let completed = db::completed_summaries(&subtasks);
    let existing_names = subtasks
        .iter()
        .map(|s| s.subtask_name.trim().to_string())
        .collect::<Vec<_>>()
        .join("；");
    let human = human_instructions(app, conn, task.id);
    let base_user = prompts::requirement_intake_user(
        &task.task_name,
        &task.top_requirement,
        &task.global_accept_criteria,
        &task.tech_stack,
        &task.global_contract,
        &completed,
        &existing_names,
        &requirement,
        &human,
        force_single,
    );
    let mut user = base_user.clone();
    let mut parsed: Option<IntakeOutput> = None;
    let mut last_error = String::new();
    for attempt in 0..2 {
        let system = agent_system(&reviewer, &prompts::requirement_intake_system());
        let value = llm_json(
            app,
            conn,
            task.id,
            None,
            "requirement_intake",
            &reviewer,
            &system,
            &user,
            &settings.llm_timeout_sec,
        )?;
        match validate_intake(
            value,
            &subtasks
                .iter()
                .map(|s| s.subtask_name.clone())
                .collect::<Vec<_>>(),
            force_single,
        ) {
            Ok(out) => {
                parsed = Some(out);
                break;
            }
            Err(e) => {
                last_error = e.clone();
                log(
                    app,
                    conn,
                    task.id,
                    "warn",
                    &format!("追加需求梳理输出不合法（第 {} 次）：{}", attempt + 1, e),
                );
                user = format!(
                    "{base_user}\n\n注意：你上一次输出不符合要求：{e}。请修正后重新只输出合法 JSON。"
                );
            }
        }
    }
    let output = parsed.ok_or_else(|| format!("追加需求梳理多次校验失败：{last_error}"))?;
    let decision = output.decision.trim().to_lowercase();
    let items = output
        .subtasks
        .iter()
        .map(|i| {
            (
                i.name.trim().to_string(),
                i.accept_criteria.trim().to_string(),
            )
        })
        .collect::<Vec<_>>();

    record_event(
        app,
        conn,
        task.id,
        None,
        "agent_message:reviewer",
        &format!(
            "追加需求梳理完成（{}）：{}\n本次新增 {} 个子任务：{}",
            if decision == "simple" {
                "简单改动 · 单模块"
            } else {
                "复杂需求 · 少量模块"
            },
            output.analysis.trim(),
            items.len(),
            output
                .subtasks
                .iter()
                .map(|i| i.name.trim().to_string())
                .collect::<Vec<_>>()
                .join("；")
        ),
    );

    let mut conn_tx = db::open(db_path)?;
    let new_subtasks = db::apply_followup_requirement(
        &mut conn_tx,
        task,
        &requirement,
        output.analysis.trim(),
        &items,
    )?;
    record_event(
        app,
        conn,
        task.id,
        None,
        "requirement_accepted",
        &format!(
            "追加需求已落成 {} 个新子任务，从模块 #{} 继续串行开发；原已完成模块不会重跑。",
            items.len(),
            new_subtasks
                .iter()
                .filter(|s| s.status == "waiting")
                .map(|s| s.sort_index)
                .min()
                .unwrap_or_default()
        ),
    );
    log(
        app,
        conn,
        task.id,
        "info",
        &format!(
            "追加需求梳理完成：{}，新增 {} 个子任务，继续交给 Coder 开发",
            if decision == "simple" {
                "简单改动只创建 1 个模块"
            } else {
                "复杂需求拆成少量模块"
            },
            items.len()
        ),
    );
    Ok(())
}

// ------------------------------------------------------------------
// 阶段 2：单个子任务内部循环
// ------------------------------------------------------------------
fn do_module_step(
    app: Option<&AppHandle>,
    conn: &Connection,
    app_data_dir: &Path,
    settings: &AppSettings,
    task: &mut TaskMain,
    mut sub: Subtask,
) -> Result<(), String> {
    match sub.current_stage.as_str() {
        "design_generation" => {
            design_generation_step(app, conn, app_data_dir, settings, task, &mut sub)
        }
        "design_approval" => {
            // 只有人工审批通过命令才会把 stage 改为 coding；
            // 走到这里说明仍在等待审批。
            task.status = "needs_human".into();
            db::save_task(conn, task)?;
            log(
                app,
                conn,
                task.id,
                "info",
                "模块设计文档等待人工审批通过后才会派发 Coder",
            );
            Ok(())
        }
        "coding" => coder_step(app, conn, app_data_dir, settings, task, &mut sub),
        "code_review" => code_review_step(app, conn, app_data_dir, settings, task, &mut sub),
        "summary_generation" => {
            summary_generation_step(app, conn, app_data_dir, settings, task, &mut sub)
        }
        "summary_review" => summary_review_step(app, conn, app_data_dir, settings, task, &mut sub),
        other => Err(format!("非法子任务阶段: {other}")),
    }
}

#[derive(Debug, Deserialize)]
struct DesignOutput {
    design_doc: String,
}

fn design_generation_step(
    app: Option<&AppHandle>,
    conn: &Connection,
    app_data_dir: &Path,
    settings: &AppSettings,
    task: &mut TaskMain,
    sub: &mut Subtask,
) -> Result<(), String> {
    let reviewer = db::get_agent(conn, task.reviewer_agent_id)?
        .ok_or_else(|| "评审 Agent 不存在".to_string())?;
    let subtasks = db::list_subtasks(conn, task.id)?;
    let completed = db::completed_summaries(&subtasks);
    let human = human_instructions(app, conn, task.id);
    let base_user = prompts::design_user(
        &tech_stack_text(task),
        &prompts::truncate_chars(&task.global_contract, 20_000),
        &prompts::truncate_chars(&completed, 40_000),
        sub,
        &prompts::truncate_chars(&task.top_requirement, 20_000),
        &human,
    );
    let system = agent_system(&reviewer, &prompts::design_system());
    let mut user = base_user.clone();
    let mut design = String::new();
    let mut last_error = String::new();
    for attempt in 0..2 {
        let value = llm_json(
            app,
            conn,
            task.id,
            Some(sub.id),
            "design",
            &reviewer,
            &system,
            &user,
            &settings.llm_timeout_sec,
        )?;
        match parse_design_output(&value) {
            Ok(doc) => {
                design = doc;
                break;
            }
            Err(e) => {
                last_error = e.clone();
                log(
                    app,
                    conn,
                    task.id,
                    "warn",
                    &format!("模块设计输出语义不合法（第 {} 次）：{}", attempt + 1, e),
                );
                user = format!(
                    "{base_user}\n\n注意：上一次输出不符合要求：{e}。请重新只输出 {{\"design_doc\":\"Markdown\"}}。"
                );
            }
        }
    }
    if design.trim().is_empty() {
        return Err(format!("模块设计多次校验失败：{last_error}"));
    }
    sub.design_content = design;
    if let Err(e) = storage::save_design(app_data_dir, task.id, sub, &sub.design_content) {
        log(
            app,
            conn,
            task.id,
            "warn",
            &format!("设计快照写入外部存储失败：{e}"),
        );
    }
    record_event(
        app,
        conn,
        task.id,
        Some(sub.id),
        "module_design",
        &sub.design_content,
    );

    if task.manual_review_enabled {
        sub.current_stage = "design_approval".into();
        task.status = "needs_human".into();
        db::save_subtask(conn, sub)?;
        db::save_task(conn, task)?;
        log(
            app,
            conn,
            task.id,
            "warn",
            &format!(
                "子任务 #{}「{}」设计文档已生成，等待人工审批后才能交给 Coder",
                sub.sort_index, sub.subtask_name
            ),
        );
    } else {
        sub.current_stage = "coding".into();
        db::save_subtask(conn, sub)?;
        log(
            app,
            conn,
            task.id,
            "info",
            &format!(
                "子任务 #{}「{}」设计文档已生成并自动通过，开始派发 Coder",
                sub.sort_index, sub.subtask_name
            ),
        );
    }
    Ok(())
}

fn parse_design_output(value: &Value) -> Result<String, String> {
    let output: DesignOutput = serde_json::from_value(value.clone())
        .map_err(|e| format!("JSON 结构不符合模块设计约定: {e}"))?;
    let doc = output.design_doc.trim().to_string();
    if doc.len() < 20 {
        return Err("design_doc 太短或为空".into());
    }
    if doc.chars().count() > 40_000 {
        return Err("design_doc 过长，请控制在 4 万字符以内".into());
    }
    Ok(doc)
}

/// 把 dsh 的 NDJSON 事件压缩成页面上可读的 Agent 对话 / 工具调用事件。
fn record_dsh_line(
    app: Option<&AppHandle>,
    conn: &Connection,
    task_id: i64,
    subtask_id: i64,
    line: &str,
) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }
    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        record_event(
            app,
            conn,
            task_id,
            Some(subtask_id),
            "dsh_raw",
            &prompts::truncate_chars(trimmed, 2000),
        );
        return;
    };
    match value.get("type").and_then(Value::as_str).unwrap_or("") {
        "text" => {
            let text = value
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            if !text.is_empty() && text != "SUMMARY_DONE" {
                record_event(
                    app,
                    conn,
                    task_id,
                    Some(subtask_id),
                    "agent_message:coder",
                    text,
                );
            }
        }
        "tool_call" => {
            let tool = value.get("tool").and_then(Value::as_str).unwrap_or("tool");
            let input = value.get("input").cloned().unwrap_or(Value::Null);
            let content = json!({ "tool": tool, "input": input }).to_string();
            record_event(
                app,
                conn,
                task_id,
                Some(subtask_id),
                "tool_call",
                &prompts::truncate_chars(&content, 5000),
            );
        }
        "tool_result" => {
            let status = value
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("completed");
            let result = value.get("result").cloned().unwrap_or(Value::Null);
            let text = match result {
                Value::String(s) => s,
                other => other.to_string(),
            };
            let content = json!({ "status": status, "result": text }).to_string();
            record_event(
                app,
                conn,
                task_id,
                Some(subtask_id),
                "tool_result",
                &prompts::truncate_chars(&content, 4000),
            );
        }
        "final" => {
            let text = value
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            if !text.is_empty() && text != "SUMMARY_DONE" {
                record_event(
                    app,
                    conn,
                    task_id,
                    Some(subtask_id),
                    "agent_message:coder",
                    &prompts::truncate_chars(text, 5000),
                );
            }
        }
        "error" => {
            record_event(
                app,
                conn,
                task_id,
                Some(subtask_id),
                "dsh_error",
                &prompts::truncate_chars(trimmed, 3000),
            );
        }
        _ => {}
    }
}

fn coder_step(
    app: Option<&AppHandle>,
    conn: &Connection,
    app_data_dir: &Path,
    settings: &AppSettings,
    task: &TaskMain,
    sub: &mut Subtask,
) -> Result<(), String> {
    let coder_id = sub
        .assigned_coder_id
        .or_else(|| task.coder_agent_ids.first().copied())
        .ok_or_else(|| "子任务未分配 Coder".to_string())?;
    let coder = db::get_agent(conn, coder_id)?.ok_or_else(|| "Coder Agent 不存在".to_string())?;
    if coder.role_type != "coder" {
        return Err("子任务分配的 Agent 不是 Coder".into());
    }

    let subtasks = db::list_subtasks(conn, task.id)?;
    let completed = db::completed_summaries(&subtasks);
    let feedback = latest_action_feedback(sub);
    let human = human_instructions(app, conn, task.id);
    let prompt = format!(
        "{}\n\n{}",
        prompts::coder_system(&coder),
        prompts::coder_implementation_user(
            &tech_stack_text(task),
            &prompts::truncate_chars(&task.global_contract, 20_000),
            &requirements_text(task),
            &prompts::truncate_chars(&sub.design_content, 40_000),
            &prompts::truncate_chars(&completed, 50_000),
            sub,
            &feedback,
            &human,
        )
    );

    let baseline = dsh::capture_tree(Path::new(&task.work_dir))?;
    log(
        app,
        conn,
        task.id,
        "info",
        &format!(
            "Coder「{}」开始实现子任务 #{}「{}」（第 {}/{} 轮）",
            coder.name, sub.sort_index, sub.subtask_name, sub.current_round, sub.max_round
        ),
    );
    record_event(app, conn, task.id, Some(sub.id), "coder_prompt", &prompt);

    let timeout = parse_u64(&settings.dsh_timeout_sec, 1800, 30, 86_400);
    let sub_id = sub.id;
    let mut on_stdout = |line: &str| {
        record_dsh_line(app, conn, task.id, sub_id, line);
    };
    let result = dsh::run_coder(
        app_data_dir,
        settings,
        &coder,
        Path::new(&task.work_dir),
        &prompt,
        timeout,
        Some(cancel_flag(task.id)),
        &mut on_stdout,
    )?;

    if !result.stderr.trim().is_empty() {
        record_event(
            app,
            conn,
            task.id,
            Some(sub.id),
            "dsh_stderr",
            &result.stderr,
        );
    }
    if !result.success() {
        log(
            app,
            conn,
            task.id,
            "error",
            &format!(
                "Coder「{}」执行失败（exit={}）：{}",
                coder.name,
                result.exit_code,
                prompts::truncate_chars(
                    format!("{}\n{}", result.final_text, result.stderr).trim(),
                    2000
                )
            ),
        );
        return Err("Coder dsh 执行失败，任务已暂停，可人工恢复调度".into());
    }

    let diff = dsh::diff_since(Path::new(&task.work_dir), &baseline)?;
    if let Err(e) = storage::save_round_diff(
        app_data_dir,
        task.id,
        sub,
        sub.current_round,
        &prompts::truncate_chars(&diff, MAX_CODE_DIFF_CHARS),
    ) {
        log(
            app,
            conn,
            task.id,
            "warn",
            &format!("diff 快照写入外部存储失败：{e}"),
        );
    }
    record_event(
        app,
        conn,
        task.id,
        Some(sub.id),
        "git_diff",
        &prompts::truncate_chars(&diff, MAX_CODE_DIFF_CHARS),
    );
    let record = crate::models::IterationRecord {
        round: sub.current_round,
        record_type: "code_review".into(),
        coder_agent_id: Some(coder.id),
        diff: prompts::truncate_chars(&diff, MAX_CODE_DIFF_CHARS),
        review_feedback: String::new(),
        review_decision: String::new(),
        summary_content: String::new(),
        created_at: db::now(),
    };
    sub.iteration_records.push(record);
    sub.current_stage = "code_review".into();
    db::save_subtask(conn, sub)?;
    emit_update(app, task.id);
    log(
        app,
        conn,
        task.id,
        "info",
        &format!(
            "Coder 已提交子任务 #{} 第 {} 轮 diff（{} 字符），等待评审",
            sub.sort_index,
            sub.current_round,
            diff.chars().count()
        ),
    );
    Ok(())
}

fn code_review_step(
    app: Option<&AppHandle>,
    conn: &Connection,
    storage_root: &Path,
    settings: &AppSettings,
    task: &mut TaskMain,
    sub: &mut Subtask,
) -> Result<(), String> {
    let reviewer = db::get_agent(conn, task.reviewer_agent_id)?
        .ok_or_else(|| "评审 Agent 不存在".to_string())?;
    let subtasks = db::list_subtasks(conn, task.id)?;
    let completed = db::completed_summaries(&subtasks);

    let diff = sub
        .iteration_records
        .last()
        .map(|r| r.diff.clone())
        .unwrap_or_default();

    let (passed, feedback) = if diff.trim().is_empty() {
        (
            false,
            "本轮 Coder 没有产生任何代码变更。请检查是否真正实现了当前子任务。".to_string(),
        )
    } else {
        let human = human_instructions(app, conn, task.id);
        let mut user = prompts::code_review_user(
            &prompts::truncate_chars(&task.global_contract, 20_000),
            &requirements_text(task),
            &prompts::truncate_chars(&completed, 50_000),
            sub,
            &prompts::truncate_chars(&diff, MAX_CODE_DIFF_CHARS),
        );
        if !human.trim().is_empty() {
            user.push_str(&format!("\n\n# 人工实时介入消息\n{human}"));
        }
        call_reviewer(
            app,
            conn,
            task.id,
            Some(sub.id),
            "code_review",
            &reviewer,
            &settings.llm_timeout_sec,
            prompts::json_reviewer_system("代码 diff 评审"),
            &user,
        )?
    };

    update_last_record(sub, &feedback, if passed { "pass" } else { "reject" });
    if let Some(rec) = sub.iteration_records.last() {
        if let Ok(text) = serde_json::to_string_pretty(rec) {
            let _ = storage::save_review_snapshot(
                storage_root,
                task.id,
                sub,
                sub.current_round,
                "code-review",
                &text,
            );
        }
    }
    if passed {
        sub.current_stage = "summary_generation".into();
        sub.status = "in_progress".into();
        db::save_subtask(conn, sub)?;
        log(
            app,
            conn,
            task.id,
            "info",
            &format!(
                "✅ 代码评审通过：子任务 #{}「{}」第 {} 轮；要求 Coder 生成 summary.md",
                sub.sort_index, sub.subtask_name, sub.current_round
            ),
        );
        Ok(())
    } else {
        log(
            app,
            conn,
            task.id,
            "warn",
            &format!(
                "❌ 评审驳回：子任务 #{}「{}」第 {} 轮；意见：{}",
                sub.sort_index,
                sub.subtask_name,
                sub.current_round,
                prompts::truncate_chars(&feedback, 600)
            ),
        );
        if sub.current_round >= sub.max_round {
            mark_subtask_needs_human(app, conn, task, sub, "代码评审达到子任务最大轮次")?;
            Ok(())
        } else {
            sub.current_round += 1;
            sub.current_stage = "coding".into();
            db::save_subtask(conn, sub)?;
            log(
                app,
                conn,
                task.id,
                "info",
                &format!(
                    "子任务 #{} 轮次 +1，当前第 {}/{} 轮，重新派发 Coder 修改",
                    sub.sort_index, sub.current_round, sub.max_round
                ),
            );
            Ok(())
        }
    }
}

fn summary_generation_step(
    app: Option<&AppHandle>,
    conn: &Connection,
    app_data_dir: &Path,
    settings: &AppSettings,
    task: &mut TaskMain,
    sub: &mut Subtask,
) -> Result<(), String> {
    let coder_id = sub
        .assigned_coder_id
        .or_else(|| task.coder_agent_ids.first().copied())
        .ok_or_else(|| "子任务未分配 Coder".to_string())?;
    let coder = db::get_agent(conn, coder_id)?.ok_or_else(|| "Coder Agent 不存在".to_string())?;
    let rel_path = dsh::summary_rel_path(sub);
    let feedback = latest_summary_feedback(sub);
    let mut current_feedback = feedback.clone();
    let mut summary_content = String::new();
    let mut summary_diff = String::new();
    let human = human_instructions(app, conn, task.id);

    for attempt in 0..2 {
        let base_prompt = prompts::coder_summary_user(
            &prompts::truncate_chars(&task.global_contract, 20_000),
            &requirements_text(task),
            sub,
            &rel_path,
            &current_feedback,
        );
        let mut prompt = if coder.system_prompt.trim().is_empty() {
            base_prompt
        } else {
            format!("{}\n\n{base_prompt}", coder.system_prompt.trim())
        };
        if !human.trim().is_empty() {
            prompt.push_str(&format!("\n\n# 人工实时介入消息\n{human}"));
        }
        record_event(app, conn, task.id, Some(sub.id), "summary_prompt", &prompt);
        let baseline = dsh::capture_tree(Path::new(&task.work_dir))?;
        log(
            app,
            conn,
            task.id,
            "info",
            &format!(
                "Coder「{}」开始生成子任务 #{} 的 summary.md（{}）",
                coder.name, sub.sort_index, rel_path
            ),
        );
        let timeout = parse_u64(&settings.dsh_timeout_sec, 1800, 30, 86_400);
        let sub_id = sub.id;
        let mut on_stdout = |line: &str| {
            record_dsh_line(app, conn, task.id, sub_id, line);
        };
        let result = dsh::run_coder(
            app_data_dir,
            settings,
            &coder,
            Path::new(&task.work_dir),
            &prompt,
            timeout,
            Some(cancel_flag(task.id)),
            &mut on_stdout,
        )?;
        if !result.stderr.trim().is_empty() {
            record_event(
                app,
                conn,
                task.id,
                Some(sub.id),
                "dsh_stderr",
                &result.stderr,
            );
        }
        if !result.success() {
            log(
                app,
                conn,
                task.id,
                "error",
                &format!(
                    "Coder 生成摘要失败（exit={}）：{}",
                    result.exit_code,
                    prompts::truncate_chars(
                        format!("{}\n{}", result.final_text, result.stderr).trim(),
                        1500
                    )
                ),
            );
            return Err("Coder 生成摘要失败，任务已暂停".into());
        }
        summary_diff = dsh::diff_since(Path::new(&task.work_dir), &baseline).unwrap_or_default();
        summary_content =
            dsh::read_summary(Path::new(&task.work_dir), &rel_path).unwrap_or_default();
        if !summary_content.trim().is_empty() {
            record_event(
                app,
                conn,
                task.id,
                Some(sub.id),
                "summary_file",
                &summary_content,
            );
            break;
        }
        if attempt == 0 {
            current_feedback = format!(
                "你没有在指定路径 {} 写入摘要文件。请使用 write 工具创建该文件，严格按模板写入，然后回复 SUMMARY_DONE。",
                rel_path
            );
            log(
                app,
                conn,
                task.id,
                "warn",
                "未检测到 summary 文件，自动要求 Coder 重试一次（不额外消耗轮次）",
            );
        }
    }

    let record = crate::models::IterationRecord {
        round: sub.current_round,
        record_type: "summary_review".into(),
        coder_agent_id: Some(coder.id),
        diff: prompts::truncate_chars(&summary_diff, 40_000),
        review_feedback: String::new(),
        review_decision: if summary_content.trim().is_empty() {
            "reject".into()
        } else {
            String::new()
        },
        summary_content: prompts::truncate_chars(&summary_content, 30_000),
        created_at: db::now(),
    };
    sub.iteration_records.push(record);

    if summary_content.trim().is_empty() {
        let msg = "Coder 未在指定路径生成 summary.md；摘要属于子任务验收的一部分，不能通过。";
        update_last_record(sub, msg, "reject");
        sub.current_stage = "summary_review".into();
        db::save_subtask(conn, sub)?;
        return handle_summary_reject(app, conn, task, sub, msg);
    }

    sub.current_stage = "summary_review".into();
    db::save_subtask(conn, sub)?;
    log(
        app,
        conn,
        task.id,
        "info",
        &format!(
            "Coder 已生成子任务 #{} summary.md，提交摘要单独校验",
            sub.sort_index
        ),
    );
    Ok(())
}

fn summary_review_step(
    app: Option<&AppHandle>,
    conn: &Connection,
    storage_root: &Path,
    settings: &AppSettings,
    task: &mut TaskMain,
    sub: &mut Subtask,
) -> Result<(), String> {
    let reviewer = db::get_agent(conn, task.reviewer_agent_id)?
        .ok_or_else(|| "评审 Agent 不存在".to_string())?;
    let summary = sub
        .iteration_records
        .last()
        .map(|r| r.summary_content.clone())
        .unwrap_or_default();
    if summary.trim().is_empty() {
        return handle_summary_reject(app, conn, task, sub, "摘要内容为空，无法验收。");
    }
    // 固定模板是硬约束：先做确定性校验，不合格直接计入轮次，节省一次 LLM 调用。
    if let Err(msg) = validate_summary_template(&summary) {
        log(
            app,
            conn,
            task.id,
            "warn",
            &format!("摘要模板校验不通过：{msg}"),
        );
        update_last_record(sub, &msg, "reject");
        db::save_subtask(conn, sub)?;
        return handle_summary_reject(app, conn, task, sub, &msg);
    }
    let approved_diff = sub
        .iteration_records
        .iter()
        .rev()
        .find(|r| r.record_type == "code_review" && r.review_decision == "pass")
        .map(|r| r.diff.clone())
        .unwrap_or_default();

    let human = human_instructions(app, conn, task.id);
    let mut user = prompts::summary_review_user(
        &prompts::truncate_chars(&task.global_contract, 20_000),
        sub,
        &prompts::truncate_chars(&approved_diff, MAX_CODE_DIFF_CHARS),
        &summary,
    );
    if !human.trim().is_empty() {
        user.push_str(&format!("\n\n# 人工实时介入消息\n{human}"));
    }
    let (passed, feedback) = call_reviewer(
        app,
        conn,
        task.id,
        Some(sub.id),
        "summary_review",
        &reviewer,
        &settings.llm_timeout_sec,
        prompts::summary_review_system(),
        &user,
    )?;

    update_last_record(sub, &feedback, if passed { "pass" } else { "reject" });
    if let Some(rec) = sub.iteration_records.last() {
        if let Ok(text) = serde_json::to_string_pretty(rec) {
            let _ = storage::save_review_snapshot(
                storage_root,
                task.id,
                sub,
                sub.current_round,
                "summary-review",
                &text,
            );
        }
    }
    if !passed {
        log(
            app,
            conn,
            task.id,
            "warn",
            &format!(
                "摘要校验不通过：子任务 #{} 第 {} 轮；意见：{}",
                sub.sort_index,
                sub.current_round,
                prompts::truncate_chars(&feedback, 600)
            ),
        );
        return handle_summary_reject(app, conn, task, sub, &feedback);
    }

    // 摘要校验通过：快照写入外部存储根目录（项目目录只保留代码），并归档。
    let path = storage::persist_summary(storage_root, task.id, sub, &summary)?;
    storage::remove_project_summary(Path::new(&task.work_dir), &dsh::summary_rel_path(sub));
    sub.summary_content = summary;
    sub.summary_file_path = path.to_string_lossy().to_string();
    sub.status = "completed".into();
    sub.current_stage = "done".into();
    db::save_subtask(conn, sub)?;
    log(
        app,
        conn,
        task.id,
        "info",
        &format!(
            "✅ 摘要验收通过：子任务 #{}「{}」已完成；summary 已写入 {}，全部 {} 轮迭代记录已归档",
            sub.sort_index,
            sub.subtask_name,
            path.display(),
            sub.iteration_records.len()
        ),
    );
    Ok(())
}

/// 校验 summary.md 是否严格使用固定模板；不依赖 LLM，避免明显不合格内容浪费评审调用。
fn validate_summary_template(summary: &str) -> Result<(), String> {
    const REQUIRED: [&str; 7] = [
        "# 模块名称",
        "1. 对外接口列表",
        "2. 关联数据库表",
        "3. 核心业务逻辑简述",
        "4. 对外依赖",
        "5. 本模块对外提供的能力",
        "6. 已知限制",
    ];
    let missing: Vec<&str> = REQUIRED
        .iter()
        .copied()
        .filter(|marker| !summary.contains(marker))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "summary.md 缺少固定模板部分：{}。请严格按模板重写。",
            missing.join("、")
        ));
    }
    let chars = summary.chars().count();
    if chars > 12_000 {
        return Err(format!(
            "summary.md 过长（{chars} 字符），疑似粘贴源码/对话或流水账；请压缩到简短摘要。"
        ));
    }
    if summary.contains("```") {
        return Err("summary.md 包含代码块围栏，疑似粘贴源码；摘要中禁止粘贴源码。".into());
    }
    Ok(())
}

fn handle_summary_reject(
    app: Option<&AppHandle>,
    conn: &Connection,
    task: &mut TaskMain,
    sub: &mut Subtask,
    feedback: &str,
) -> Result<(), String> {
    if sub.current_round >= sub.max_round {
        mark_subtask_needs_human(app, conn, task, sub, "摘要校验达到子任务最大轮次")?;
        Ok(())
    } else {
        sub.current_round += 1;
        sub.current_stage = "summary_generation".into();
        sub.status = "in_progress".into();
        db::save_subtask(conn, sub)?;
        log(
            app,
            conn,
            task.id,
            "info",
            &format!(
                "摘要打回 Coder 修改，计入子任务轮次：当前第 {}/{} 轮。意见：{}",
                sub.current_round,
                sub.max_round,
                prompts::truncate_chars(feedback, 500)
            ),
        );
        Ok(())
    }
}

fn mark_subtask_needs_human(
    app: Option<&AppHandle>,
    conn: &Connection,
    task: &mut TaskMain,
    sub: &mut Subtask,
    reason: &str,
) -> Result<(), String> {
    sub.status = "needs_human".into();
    task.status = "needs_human".into();
    db::save_subtask(conn, sub)?;
    db::save_task(conn, task)?;
    log(
        app,
        conn,
        task.id,
        "warn",
        &format!(
            "⛔ 子任务 #{}「{}」轮次耗尽（{}/{}）：{}，任务标记【需要人工介入】",
            sub.sort_index, sub.subtask_name, sub.current_round, sub.max_round, reason
        ),
    );
    Ok(())
}

fn update_last_record(sub: &mut Subtask, feedback: &str, decision: &str) {
    if let Some(rec) = sub.iteration_records.last_mut() {
        rec.review_feedback = feedback.to_string();
        rec.review_decision = decision.to_string();
        if rec.created_at.is_empty() {
            rec.created_at = db::now();
        }
    }
}

fn latest_action_feedback(sub: &Subtask) -> String {
    sub.iteration_records
        .iter()
        .rev()
        .find(|r| {
            !r.review_feedback.trim().is_empty()
                && (r.review_decision == "reject"
                    || r.record_type == "global_rework"
                    || r.record_type == "human_reject")
        })
        .map(|r| r.review_feedback.clone())
        .unwrap_or_default()
}

fn latest_summary_feedback(sub: &Subtask) -> String {
    sub.iteration_records
        .iter()
        .rev()
        .find(|r| {
            (r.record_type == "summary_review" && r.review_decision == "reject")
                || r.record_type == "human_reject"
        })
        .map(|r| r.review_feedback.clone())
        .unwrap_or_default()
}

// ------------------------------------------------------------------
// 阶段 3：全局评审收口
// ------------------------------------------------------------------
#[derive(Debug, Deserialize)]
struct GlobalIssueInput {
    subtask_id: Option<i64>,
    subtask_name: Option<String>,
    #[serde(default)]
    feedback: String,
}

#[derive(Debug, Deserialize)]
struct GlobalReviewOutput {
    passed: bool,
    #[serde(default)]
    feedback: String,
    #[serde(default)]
    issues: Vec<GlobalIssueInput>,
}

fn do_global_step(
    app: Option<&AppHandle>,
    conn: &Connection,
    db_path: &Path,
    _app_data_dir: &Path,
    settings: &AppSettings,
    task: &mut TaskMain,
    subtasks: &[Subtask],
) -> Result<(), String> {
    let reviewer = db::get_agent(conn, task.reviewer_agent_id)?
        .ok_or_else(|| "评审 Agent 不存在".to_string())?;

    let next_round = task.current_global_round + 1;
    log(
        app,
        conn,
        task.id,
        "info",
        &format!(
            "进入全局评审第 {}/{} 轮：校验跨模块一致性、命名、安全规范与整体优化点…",
            next_round, task.global_max_round
        ),
    );

    let cumulative_diff =
        dsh::task_cumulative_diff(Path::new(&task.work_dir), &task.baseline_tree)?;
    let summaries = db::completed_summaries(subtasks);
    let base_user = prompts::global_review_user(
        &prompts::truncate_chars(&task.global_contract, 25_000),
        &prompts::truncate_chars(&task.top_requirement, 20_000),
        &prompts::truncate_chars(&summaries, 60_000),
        &prompts::truncate_chars(&cumulative_diff, MAX_GLOBAL_DIFF_CHARS),
        &task.global_accept_criteria,
        subtasks,
        &task.global_review_records,
    );

    let human = human_instructions(app, conn, task.id);
    let mut user = if human.trim().is_empty() {
        base_user.clone()
    } else {
        format!("{base_user}\n\n# 人工实时介入消息\n{human}")
    };
    let mut parsed: Option<GlobalReviewOutput> = None;
    let mut last_error = String::new();
    for attempt in 0..2 {
        let system = agent_system(&reviewer, &prompts::global_review_system());
        let value = llm_json(
            app,
            conn,
            task.id,
            None,
            "global_review",
            &reviewer,
            &system,
            &user,
            &settings.llm_timeout_sec,
        )?;
        match validate_global_output(value, subtasks) {
            Ok(out) => {
                parsed = Some(out);
                break;
            }
            Err(e) => {
                last_error = e.clone();
                log(
                    app,
                    conn,
                    task.id,
                    "warn",
                    &format!("全局评审输出语义不合法（第 {} 次）：{}", attempt + 1, e),
                );
                user = format!(
                    "{base_user}\n\n注意：你上一次输出不符合要求：{e}。请修正后重新只输出合法 JSON。"
                );
            }
        }
    }
    let output = parsed.ok_or_else(|| format!("全局评审结果多次校验失败：{last_error}"))?;

    let mut issues: Vec<GlobalReviewIssue> = output
        .issues
        .iter()
        .map(|i| GlobalReviewIssue {
            subtask_id: i.subtask_id,
            subtask_name: i.subtask_name.clone().unwrap_or_default(),
            feedback: i.feedback.trim().to_string(),
        })
        .collect();
    if output.passed {
        // 全局通过时不应携带返工问题，避免 UI/后续逻辑误读。
        issues.clear();
    }

    // 只有本轮评审成功产出合法 JSON 才消耗全局轮次，避免网络/解析失败偷跑轮次。
    task.current_global_round = next_round;
    let record = GlobalReviewRecord {
        cycle: task.global_review_cycle.max(1),
        round: task.current_global_round,
        passed: output.passed,
        feedback: output.feedback.trim().to_string(),
        issues: issues.clone(),
        created_at: db::now(),
    };
    task.global_review_records.push(record);
    record_event(
        app,
        conn,
        task.id,
        None,
        "agent_message:reviewer",
        &format!(
            "全局评审第 {} 轮：{}\n{}",
            task.current_global_round,
            if output.passed { "通过" } else { "不通过" },
            output.feedback.trim()
        ),
    );

    if output.passed {
        task.status = "completed".into();
        db::save_task(conn, task)?;
        log(
            app,
            conn,
            task.id,
            "info",
            &format!(
                "🎉 全局评审通过（第 {}/{} 轮），整个任务已完成",
                task.current_global_round, task.global_max_round
            ),
        );
        return Ok(());
    }

    log(
        app,
        conn,
        task.id,
        "warn",
        &format!(
            "全局评审不通过（第 {}/{} 轮）：{}",
            task.current_global_round,
            task.global_max_round,
            prompts::truncate_chars(&output.feedback, 800)
        ),
    );

    if task.current_global_round >= task.global_max_round {
        task.status = "needs_human".into();
        db::save_task(conn, task)?;
        log(
            app,
            conn,
            task.id,
            "warn",
            "⛔ 全局评审轮次耗尽，任务标记【需要人工介入】",
        );
        return Ok(());
    }

    // 全局返工：问题必须归属到模块；模块轮次重置为 1，回到增量评审循环。
    let mut conn_tx = db::open(db_path)?;
    let mut applied = 0usize;
    let tx = conn_tx.transaction().map_err(|e| e.to_string())?;
    for issue in &issues {
        let target_idx = subtasks.iter().position(|s| {
            issue.subtask_id == Some(s.id)
                || (!issue.subtask_name.is_empty() && issue.subtask_name == s.subtask_name)
        });
        let Some(idx) = target_idx else { continue };
        let mut target = subtasks[idx].clone();
        target.status = "in_progress".into();
        target.current_round = 1;
        target.current_stage = "coding".into();
        target
            .iteration_records
            .push(crate::models::IterationRecord {
                round: 1,
                record_type: "global_rework".into(),
                coder_agent_id: target.assigned_coder_id,
                diff: String::new(),
                review_feedback: issue.feedback.clone(),
                review_decision: "reject".into(),
                summary_content: String::new(),
                created_at: db::now(),
            });
        // 直接写库，避免与外部 conn 造成嵌套事务。
        let records =
            serde_json::to_string(&target.iteration_records).map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE subtask SET status=?1,current_round=?2,current_stage=?3,iteration_records=?4 WHERE id=?5",
            rusqlite::params!["in_progress", 1, "coding", records, target.id],
        )
        .map_err(|e| e.to_string())?;
        applied += 1;
    }
    tx.commit().map_err(|e| e.to_string())?;

    if applied == 0 {
        task.status = "needs_human".into();
        db::save_task(conn, task)?;
        log(
            app,
            conn,
            task.id,
            "warn",
            "全局评审发现问题但无法归属到任何子任务，请人工介入",
        );
        return Ok(());
    }

    task.task_phase = "module_dev".into();
    task.status = "running".into();
    db::save_task(conn, task)?;
    log(
        app,
        conn,
        task.id,
        "info",
        &format!(
            "全局评审驳回，已下发 {} 个模块返工（模块轮次重置为 1），回到模块开发阶段",
            applied
        ),
    );
    Ok(())
}

fn validate_global_output(
    value: Value,
    subtasks: &[Subtask],
) -> Result<GlobalReviewOutput, String> {
    let out: GlobalReviewOutput =
        serde_json::from_value(value).map_err(|e| format!("JSON 结构不符合全局评审约定: {e}"))?;
    if !out.passed {
        if out.issues.is_empty() {
            return Err("passed=false 时 issues 不能为空，必须标注问题归属模块".into());
        }
        let mut valid = 0;
        for issue in &out.issues {
            if issue.feedback.trim().is_empty() {
                return Err("issues[].feedback 不能为空".into());
            }
            let matched = subtasks.iter().any(|s| {
                issue.subtask_id == Some(s.id)
                    || issue
                        .subtask_name
                        .as_deref()
                        .map(|n| n == s.subtask_name)
                        .unwrap_or(false)
            });
            if matched {
                valid += 1;
            }
        }
        if valid == 0 {
            return Err("所有 issues 都无法归属到已有子任务模块".into());
        }
    }
    Ok(out)
}

// ------------------------------------------------------------------
// Reviewer 调用 + 语义校验重试
// ------------------------------------------------------------------
fn call_reviewer(
    app: Option<&AppHandle>,
    conn: &Connection,
    task_id: i64,
    subtask_id: Option<i64>,
    stage: &str,
    reviewer: &AgentConfig,
    llm_timeout_sec: &str,
    system: String,
    user: &str,
) -> Result<(bool, String), String> {
    let system = agent_system(reviewer, &system);
    let mut prompt = user.to_string();
    let mut last_error = String::new();
    for attempt in 0..2 {
        let value = llm_json(
            app,
            conn,
            task_id,
            subtask_id,
            stage,
            reviewer,
            &system,
            &prompt,
            llm_timeout_sec,
        )?;
        match parse_review_output(&value) {
            Ok((passed, feedback)) => {
                let text = format!(
                    "评审结论：{}\n{}",
                    if passed { "通过" } else { "不通过" },
                    if feedback.trim().is_empty() {
                        "（无补充意见）"
                    } else {
                        feedback.trim()
                    }
                );
                record_event(
                    app,
                    conn,
                    task_id,
                    subtask_id,
                    "agent_message:reviewer",
                    &text,
                );
                return Ok((passed, feedback));
            }
            Err(e) => {
                last_error = e.clone();
                log(
                    app,
                    conn,
                    task_id,
                    "warn",
                    &format!("评审 JSON 语义不合法（第 {} 次）：{}", attempt + 1, e),
                );
                prompt = format!(
                    "{user}\n\n注意：你上一次输出结构不合法：{e}。请严格只输出 {{\"passed\":bool,\"feedback\":\"...\"}}。"
                );
            }
        }
    }
    Err(format!("评审结果多次校验失败：{last_error}"))
}

fn parse_review_output(value: &Value) -> Result<(bool, String), String> {
    let passed = value
        .get("passed")
        .and_then(Value::as_bool)
        .ok_or_else(|| "缺少布尔字段 passed".to_string())?;
    let feedback = value
        .get("feedback")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if !passed && feedback.is_empty() {
        return Err("未通过时 feedback 不能为空".into());
    }
    Ok((passed, feedback))
}

// ------------------------------------------------------------------
// 无 UI 执行入口（供集成测试使用；生产走 spawn_task）
// ------------------------------------------------------------------
pub fn run_task_headless(db_path: &Path, app_data_dir: &Path, task_id: i64) -> Result<(), String> {
    // 测试入口：把待运行任务直接推进到 planning，模拟调度器启动。
    if let Ok(conn) = db::open(db_path) {
        if let Ok(Some(mut task)) = db::get_task(&conn, task_id) {
            if task.status == "pending" {
                task.status = "planning".into();
                task.start_requested = false;
                task.pause_reason.clear();
                let _ = db::save_task(&conn, &task);
            }
        }
    }
    register_cancel(task_id);
    let result = run_loop(None, db_path, app_data_dir, task_id);
    remove_cancel(task_id);
    result
}

// ------------------------------------------------------------------
// 人工兜底命令实现（由 commands 层调用）
// ------------------------------------------------------------------
// ------------------------------------------------------------------
// 方案审批 / 模块设计审批 / 运行中配置 / 人工消息 / 终止
// ------------------------------------------------------------------
fn load_pending_plan(task: &TaskMain) -> Result<PendingPlan, String> {
    serde_json::from_str::<PendingPlan>(&task.pending_plan_json)
        .map_err(|e| format!("待审批方案解析失败: {e}"))
}

pub fn approve_plan(app: Option<&AppHandle>, task_id: i64) -> Result<(), String> {
    let handle = app;
    let db_path = if let Some(h) = handle {
        h.state::<AppState>().db_path()?
    } else {
        return Err("缺少 UI 上下文".into());
    };
    let conn = db::open(&db_path)?;
    let mut task = db::get_task(&conn, task_id)?.ok_or_else(|| "任务不存在".to_string())?;
    if task.task_phase != "plan_approval" {
        return Err("当前不在方案审批阶段".into());
    }
    let plan = load_pending_plan(&task)?;
    let mut conn_tx = db::open(&db_path)?;
    db::apply_pending_plan(&mut conn_tx, &mut task, &plan)?;
    record_event(
        handle,
        &conn,
        task_id,
        None,
        "plan_approved",
        "人工审批通过拆解方案，已创建子任务并开始开发。",
    );
    log(
        handle,
        &conn,
        task_id,
        "info",
        &format!(
            "人工审批通过拆解方案：{} 个模块开始串行开发",
            plan.subtasks.len()
        ),
    );
    drop(conn);
    if let Some(h) = app {
        spawn_task(h.clone(), task_id)?;
    }
    Ok(())
}

pub fn reject_plan(app: Option<&AppHandle>, task_id: i64, feedback: &str) -> Result<(), String> {
    if feedback.trim().is_empty() {
        return Err("驳回意见不能为空".into());
    }
    let handle = app.ok_or_else(|| "缺少 UI 上下文".to_string())?;
    let db_path = handle.state::<AppState>().db_path()?;
    let conn = db::open(&db_path)?;
    let mut task = db::get_task(&conn, task_id)?.ok_or_else(|| "任务不存在".to_string())?;
    if task.task_phase != "plan_approval" {
        return Err("当前不在方案审批阶段".into());
    }
    task.status = "planning".into();
    task.task_phase = "module_dev".into();
    task.global_contract.clear();
    task.tech_stack.clear();
    task.pending_plan_json.clear();
    db::save_task(&conn, &task)?;
    db::add_human_message(&conn, task_id, feedback.trim())?;
    record_event(
        Some(handle),
        &conn,
        task_id,
        None,
        "plan_rejected",
        feedback.trim(),
    );
    log(
        Some(handle),
        &conn,
        task_id,
        "warn",
        &format!(
            "人工驳回拆解方案，技术总监将按意见重新拆解：{}",
            prompts::truncate_chars(feedback, 500)
        ),
    );
    drop(conn);
    spawn_task(handle.clone(), task_id)?;
    Ok(())
}

pub fn approve_design(
    app: Option<&AppHandle>,
    task_id: i64,
    subtask_id: i64,
) -> Result<(), String> {
    let handle = app.ok_or_else(|| "缺少 UI 上下文".to_string())?;
    let db_path = handle.state::<AppState>().db_path()?;
    let conn = db::open(&db_path)?;
    let task = db::get_task(&conn, task_id)?.ok_or_else(|| "任务不存在".to_string())?;
    let mut sub = db::get_subtask(&conn, subtask_id)?.ok_or_else(|| "子任务不存在".to_string())?;
    if sub.task_id != task_id {
        return Err("子任务不属于该任务".into());
    }
    if sub.current_stage != "design_approval" {
        return Err("当前模块不在设计审批阶段".into());
    }
    sub.current_stage = "coding".into();
    sub.status = "in_progress".into();
    db::save_subtask(&conn, &sub)?;
    let mut task = task;
    task.status = "running".into();
    db::save_task(&conn, &task)?;
    record_event(
        Some(handle),
        &conn,
        task_id,
        Some(sub.id),
        "design_approved",
        "人工审批通过模块设计文档。",
    );
    log(
        Some(handle),
        &conn,
        task_id,
        "info",
        &format!(
            "人工审批通过子任务 #{}「{}」设计文档，开始派发 Coder",
            sub.sort_index, sub.subtask_name
        ),
    );
    drop(conn);
    spawn_task(handle.clone(), task_id)?;
    Ok(())
}

pub fn reject_design(
    app: Option<&AppHandle>,
    task_id: i64,
    subtask_id: i64,
    feedback: &str,
) -> Result<(), String> {
    if feedback.trim().is_empty() {
        return Err("设计修改意见不能为空".into());
    }
    let handle = app.ok_or_else(|| "缺少 UI 上下文".to_string())?;
    let db_path = handle.state::<AppState>().db_path()?;
    let conn = db::open(&db_path)?;
    let mut task = db::get_task(&conn, task_id)?.ok_or_else(|| "任务不存在".to_string())?;
    let mut sub = db::get_subtask(&conn, subtask_id)?.ok_or_else(|| "子任务不存在".to_string())?;
    if sub.task_id != task_id {
        return Err("子任务不属于该任务".into());
    }
    if sub.current_stage != "design_approval" {
        return Err("当前模块不在设计审批阶段".into());
    }
    sub.current_stage = "design_generation".into();
    sub.status = "in_progress".into();
    sub.design_content.clear();
    db::save_subtask(&conn, &sub)?;
    task.status = "running".into();
    db::save_task(&conn, &task)?;
    db::add_human_message(&conn, task_id, feedback.trim())?;
    record_event(
        Some(handle),
        &conn,
        task_id,
        Some(sub.id),
        "design_rejected",
        feedback.trim(),
    );
    log(
        Some(handle),
        &conn,
        task_id,
        "warn",
        &format!(
            "人工驳回子任务 #{}「{}」设计文档，技术总监将重新设计",
            sub.sort_index, sub.subtask_name
        ),
    );
    drop(conn);
    spawn_task(handle.clone(), task_id)?;
    Ok(())
}

pub fn update_runtime_config(
    app: Option<&AppHandle>,
    task_id: i64,
    cfg: &TaskConfigInput,
) -> Result<TaskMain, String> {
    let handle = app.ok_or_else(|| "缺少 UI 上下文".to_string())?;
    let db_path = handle.state::<AppState>().db_path()?;
    let conn = db::open(&db_path)?;
    let current = db::get_task(&conn, task_id)?.ok_or_else(|| "任务不存在".to_string())?;
    if current.status == "completed" {
        return Err("任务已完成，无法修改运行中配置".into());
    }
    let task = db::update_task_config(&conn, task_id, cfg)?;
    record_event(
        Some(handle),
        &conn,
        task_id,
        None,
        "config_updated",
        &format!(
            "运行中配置已更新：模块轮次={}，全局轮次={}，人工审核={}，执行模式={}，窗口={}-{}（{}）",
            cfg.module_max_round,
            cfg.global_max_round,
            if cfg.manual_review_enabled {
                "开启"
            } else {
                "关闭"
            },
            cfg.execution_mode,
            cfg.schedule_start,
            cfg.schedule_end,
            if cfg.schedule_enabled { "启用" } else { "关闭" }
        ),
    );
    log(
        Some(handle),
        &conn,
        task_id,
        "info",
        &format!(
            "运行中配置更新：模块最大轮次 {}，全局最大轮次 {}，人工审核 {}，执行模式 {}，窗口 {}-{}",
            cfg.module_max_round,
            cfg.global_max_round,
            if cfg.manual_review_enabled {
                "开启"
            } else {
                "关闭"
            },
            cfg.execution_mode,
            cfg.schedule_start,
            cfg.schedule_end
        ),
    );

    // 关闭人工审核时，自动放行正在等待的方案/设计，避免卡死。
    let mut auto_continue = false;
    if !cfg.manual_review_enabled {
        if task.task_phase == "plan_approval" {
            if let Ok(plan) = load_pending_plan(&task) {
                let mut conn_tx = db::open(&db_path)?;
                let mut task_mut = task.clone();
                if db::apply_pending_plan(&mut conn_tx, &mut task_mut, &plan).is_ok() {
                    log(
                        Some(handle),
                        &conn,
                        task_id,
                        "info",
                        "人工审核已关闭，自动通过待审批拆解方案",
                    );
                    auto_continue = true;
                }
            }
        } else if task.status == "needs_human" {
            if let Some(mut sub) = db::list_subtasks(&conn, task_id)?
                .into_iter()
                .find(|s| s.current_stage == "design_approval")
            {
                sub.current_stage = "coding".into();
                sub.status = "in_progress".into();
                db::save_subtask(&conn, &sub)?;
                let mut task_mut = task.clone();
                task_mut.status = "running".into();
                db::save_task(&conn, &task_mut)?;
                log(
                    Some(handle),
                    &conn,
                    task_id,
                    "info",
                    "人工审核已关闭，自动通过当前模块设计文档",
                );
                auto_continue = true;
            }
        }
    }
    emit_update(Some(handle), task_id);
    drop(conn);
    if auto_continue {
        spawn_task(handle.clone(), task_id)?;
    }
    let final_conn = db::open(&db_path)?;
    db::get_task(&final_conn, task_id)?.ok_or_else(|| "任务不存在".to_string())
}

pub fn send_human_message(
    app: Option<&AppHandle>,
    task_id: i64,
    content: &str,
) -> Result<(), String> {
    if content.trim().is_empty() {
        return Err("消息不能为空".into());
    }
    let handle = app.ok_or_else(|| "缺少 UI 上下文".to_string())?;
    let db_path = handle.state::<AppState>().db_path()?;
    let conn = db::open(&db_path)?;
    let task = db::get_task(&conn, task_id)?.ok_or_else(|| "任务不存在".to_string())?;
    if task.status == "completed" {
        return Err("任务已完成，无法再发送人工消息".into());
    }
    db::add_human_message(&conn, task_id, content.trim())?;
    record_event(
        Some(handle),
        &conn,
        task_id,
        None,
        "human_message",
        content.trim(),
    );
    log(
        Some(handle),
        &conn,
        task_id,
        "warn",
        &format!(
            "人工输入消息已排队，将在下一个 Agent 步骤注入：{}",
            prompts::truncate_chars(content, 500)
        ),
    );
    Ok(())
}

pub fn cancel_task(app: &AppHandle, task_id: i64) -> Result<(), String> {
    // 兼容旧前端：旧版【终止执行】语义等同于可恢复的暂停。
    pause_task(app, task_id)
}

/// 任务运行中修改顶层需求与验收标准。新需求会作为人工消息注入下一个 Agent 步骤。
pub fn update_requirements(
    app: &AppHandle,
    task_id: i64,
    top_requirement: &str,
    accept_criteria: &str,
) -> Result<TaskMain, String> {
    let state = app.state::<AppState>();
    let db_path = state.db_path()?;
    let conn = db::open(&db_path)?;
    let current = db::get_task(&conn, task_id)?.ok_or_else(|| "任务不存在".to_string())?;
    if current.status == "completed" {
        return Err("任务已完成，无法修改需求".into());
    }
    let task = db::update_requirements(&conn, task_id, top_requirement, accept_criteria)?;
    let message = format!(
        "【需求已更新】\n## 最新顶层需求\n{}\n\n## 最新项目验收标准\n{}",
        task.top_requirement, task.global_accept_criteria
    );
    db::add_human_message(&conn, task_id, &message)?;
    record_event(
        Some(app),
        &conn,
        task_id,
        None,
        "requirements_updated",
        &message,
    );
    log(
        Some(app),
        &conn,
        task_id,
        "warn",
        "需求与验收标准已更新，将在下一个 Agent 步骤注入最新内容",
    );
    emit_update(Some(app), task_id);
    Ok(task)
}

/// 临时问答通道：独立会话，只调用 Agent 文本回复，不写入流水线事件、不触发调度。
/// 临时问答核心逻辑：不依赖 AppHandle，便于集成测试。
pub fn chat_with_connection(
    conn: &Connection,
    task_id: i64,
    content: &str,
    settings: &AppSettings,
) -> Result<ChatMessage, String> {
    if content.trim().is_empty() {
        return Err("问题不能为空".into());
    }
    let task = db::get_task(conn, task_id)?.ok_or_else(|| "任务不存在".to_string())?;
    let reviewer = db::get_agent(conn, task.reviewer_agent_id)?
        .ok_or_else(|| "任务绑定的技术总监 Agent 不存在".to_string())?;

    let previous = db::list_chat_messages(conn, task_id, 12)?;
    let history = previous
        .iter()
        .map(|m| {
            format!(
                "{}：{}",
                if m.role == "user" {
                    "用户"
                } else {
                    "技术顾问"
                },
                prompts::truncate_chars(&m.content, 3000)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let subtasks = db::list_subtasks(conn, task_id)?;
    let summaries = db::completed_summaries(&subtasks);
    let user_prompt = prompts::chat_user(
        &task.task_name,
        &task.top_requirement,
        &task.global_accept_criteria,
        &task.global_contract,
        &summaries,
        &history,
        content.trim(),
    );
    let system = agent_system(&reviewer, &prompts::chat_system());

    db::add_chat_message(conn, task_id, "user", content.trim())?;
    let reply = match llm::chat_text(&reviewer, &system, &user_prompt, &settings.llm_timeout_sec) {
        Ok(text) => text,
        Err(e) => {
            let _ = db::add_log(conn, task_id, "error", &format!("临时问答调用失败：{e}"));
            return Err(format!("临时问答失败：{e}"));
        }
    };
    db::add_chat_message(conn, task_id, "assistant", &reply)
}

/// 临时问答通道：独立会话，只调用 Agent 文本回复，不写入流水线事件、不触发调度。
pub fn send_chat_message(
    app: &AppHandle,
    task_id: i64,
    content: &str,
) -> Result<ChatMessage, String> {
    let state = app.state::<AppState>();
    let db_path = state.db_path()?;
    let conn = db::open(&db_path)?;
    let settings = db::get_settings(&conn)?;
    let message = chat_with_connection(&conn, task_id, content, &settings)?;
    emit_update(Some(app), task_id);
    Ok(message)
}

pub fn clear_chat_messages(app: &AppHandle, task_id: i64) -> Result<(), String> {
    let state = app.state::<AppState>();
    let db_path = state.db_path()?;
    let conn = db::open(&db_path)?;
    db::clear_chat_messages(&conn, task_id)?;
    emit_update(Some(app), task_id);
    Ok(())
}

/// 人工追加轮次：在保留历史记录的前提下，从当前失败阶段继续自动执行。
pub fn extend_subtask_rounds(
    app: &AppHandle,
    task_id: i64,
    subtask_id: i64,
    extra_rounds: i64,
) -> Result<(), String> {
    if !(1..=20).contains(&extra_rounds) {
        return Err("追加轮次必须是 1–20 的整数".into());
    }
    let state = app.state::<AppState>();
    let db_path = state.db_path()?;
    let conn = db::open(&db_path)?;
    let mut task = db::get_task(&conn, task_id)?.ok_or_else(|| "任务不存在".to_string())?;
    let mut sub = db::get_subtask(&conn, subtask_id)?.ok_or_else(|| "子任务不存在".to_string())?;
    if sub.task_id != task_id {
        return Err("子任务不属于该任务".into());
    }
    if task.status == "completed" || sub.status == "completed" {
        return Err("任务或子任务已完成，无法追加轮次".into());
    }
    if task.status == "terminated" {
        return Err("任务已终止，请先点击「继续任务」恢复".into());
    }
    let stage = sub.current_stage.as_str();
    let next_stage = match stage {
        "code_review" => "coding",
        "summary_review" => "summary_generation",
        other => {
            return Err(format!(
                "当前阶段（{other}）没有待重试的评审结果，无法追加轮次"
            ))
        }
    };
    let old_max = sub.max_round;
    sub.current_round += 1;
    sub.max_round = old_max + extra_rounds;
    sub.current_stage = next_stage.into();
    sub.status = "in_progress".into();
    task.status = "running".into();
    task.pause_reason.clear();
    task.cancel_requested = false;
    db::save_subtask(&conn, &sub)?;
    db::save_task(&conn, &task)?;
    record_event(
        Some(app),
        &conn,
        task_id,
        Some(sub.id),
        "human_extend_rounds",
        &format!(
            "人工追加 {} 轮：子任务 #{} 从第 {}/{} 轮继续自动执行",
            extra_rounds, sub.sort_index, sub.current_round, sub.max_round
        ),
    );
    log(
        Some(app),
        &conn,
        task_id,
        "warn",
        &format!(
            "人工追加 {} 轮：子任务 #{}「{}」当前第 {}/{} 轮，继续自动执行",
            extra_rounds, sub.sort_index, sub.subtask_name, sub.current_round, sub.max_round
        ),
    );
    drop(conn);
    if let Err(e) = spawn_task(app.clone(), task_id) {
        if let Ok(conn) = db::open(&db_path) {
            if let Ok(Some(mut failed)) = db::get_task(&conn, task_id) {
                failed.status = "failed".into();
                let _ = db::save_task(&conn, &failed);
            }
        }
        return Err(format!("追加轮次后启动调度失败：{e}"));
    }
    Ok(())
}

pub fn human_force_pass(
    app: Option<&AppHandle>,
    task_id: i64,
    subtask_id: i64,
) -> Result<(), String> {
    let handle = app.ok_or_else(|| "缺少 UI 上下文".to_string())?;
    let db_path = handle.state::<AppState>().db_path()?;
    let conn = db::open(&db_path)?;
    let mut task = db::get_task(&conn, task_id)?.ok_or_else(|| "任务不存在".to_string())?;
    let mut sub = db::get_subtask(&conn, subtask_id)?.ok_or_else(|| "子任务不存在".to_string())?;
    if sub.task_id != task_id {
        return Err("子任务不属于该任务".into());
    }
    if task.status == "completed" {
        return Err("任务已完成".into());
    }
    if task.status == "terminated" {
        return Err("任务已终止，请先点击「继续任务」恢复后再人工处理".into());
    }
    match sub.current_stage.as_str() {
        "code_review" => {
            update_last_record(&mut sub, "人工强制通过代码评审", "pass");
            sub.current_stage = "summary_generation".into();
            sub.status = "in_progress".into();
            log(
                app,
                &conn,
                task_id,
                "warn",
                &format!(
                    "人工强制通过子任务 #{} 代码评审，继续生成摘要",
                    sub.sort_index
                ),
            );
        }
        "summary_review" => {
            let summary = sub
                .iteration_records
                .last()
                .map(|r| r.summary_content.clone())
                .unwrap_or_default();
            if summary.trim().is_empty() {
                return Err("当前摘要内容为空，不能强制通过；请驳回并让 Coder 重新生成".into());
            }
            update_last_record(&mut sub, "人工强制通过摘要校验", "pass");
            let storage_root = handle.state::<AppState>().storage_root()?;
            let path = storage::persist_summary(&storage_root, task.id, &sub, &summary)?;
            storage::remove_project_summary(
                Path::new(&task.work_dir),
                &dsh::summary_rel_path(&sub),
            );
            sub.summary_content = summary;
            sub.summary_file_path = path.to_string_lossy().to_string();
            sub.status = "completed".into();
            sub.current_stage = "done".into();
            log(
                app,
                &conn,
                task_id,
                "warn",
                &format!(
                    "人工强制通过子任务 #{} 摘要校验，模块已完成",
                    sub.sort_index
                ),
            );
        }
        other => {
            return Err(format!(
                "当前阶段（{other}）没有待验收内容，无法强制通过；请使用「恢复调度」或「人工驳回」"
            ))
        }
    }
    task.status = "running".into();
    db::save_subtask(&conn, &sub)?;
    db::save_task(&conn, &task)?;
    drop(conn);
    spawn_task(handle.clone(), task_id)
}

pub fn human_reject(
    app: Option<&AppHandle>,
    task_id: i64,
    subtask_id: i64,
    feedback: &str,
) -> Result<(), String> {
    if feedback.trim().is_empty() {
        return Err("人工修改意见不能为空".into());
    }
    let handle = app.ok_or_else(|| "缺少 UI 上下文".to_string())?;
    let db_path = handle.state::<AppState>().db_path()?;
    let conn = db::open(&db_path)?;
    let mut task = db::get_task(&conn, task_id)?.ok_or_else(|| "任务不存在".to_string())?;
    let mut sub = db::get_subtask(&conn, subtask_id)?.ok_or_else(|| "子任务不存在".to_string())?;
    if sub.task_id != task_id {
        return Err("子任务不属于该任务".into());
    }
    if task.status == "completed" || sub.status == "completed" {
        return Err("任务或子任务已完成，无法驳回".into());
    }
    if task.status == "terminated" {
        return Err("任务已终止，请先点击「继续任务」恢复后再人工处理".into());
    }

    let stage_before = sub.current_stage.clone();
    let has_pending = sub
        .iteration_records
        .last()
        .map(|r| r.review_decision.is_empty())
        .unwrap_or(false);
    if has_pending {
        update_last_record(&mut sub, feedback, "reject");
    } else {
        sub.iteration_records.push(crate::models::IterationRecord {
            round: sub.current_round,
            record_type: "human_reject".into(),
            coder_agent_id: sub.assigned_coder_id,
            diff: String::new(),
            review_feedback: feedback.trim().to_string(),
            review_decision: "reject".into(),
            summary_content: String::new(),
            created_at: db::now(),
        });
    }

    sub.current_round += 1;
    if sub.max_round < sub.current_round {
        // 人工驳回应当真正获得一轮新的执行预算，而不是立刻再次判定超限。
        sub.max_round = sub.current_round;
    }
    sub.current_stage = if stage_before == "summary_review" {
        "summary_generation".into()
    } else {
        "coding".into()
    };
    sub.status = "in_progress".into();
    task.status = "running".into();
    db::save_subtask(&conn, &sub)?;
    db::save_task(&conn, &task)?;
    log(
        app,
        &conn,
        task_id,
        "warn",
        &format!(
            "人工驳回子任务 #{}，第 {} 轮重新执行；意见：{}",
            sub.sort_index,
            sub.current_round,
            prompts::truncate_chars(feedback, 500)
        ),
    );
    drop(conn);
    spawn_task(handle.clone(), task_id)
}

fn start_status(conn: &Connection, task: &TaskMain) -> Result<String, String> {
    let subtasks = db::list_subtasks(conn, task.id)?;
    Ok(
        if task.global_contract.trim().is_empty() && subtasks.is_empty() {
            "planning".to_string()
        } else {
            "running".to_string()
        },
    )
}

fn running_count(app: &AppHandle) -> usize {
    app.state::<AppState>()
        .running
        .lock()
        .map(|set| set.len())
        .unwrap_or(0)
}

fn max_concurrent(settings: &AppSettings) -> usize {
    settings
        .max_concurrent_tasks
        .trim()
        .parse::<usize>()
        .unwrap_or(1)
        .clamp(1, 32)
}

fn mark_and_spawn(
    app: &AppHandle,
    conn: &Connection,
    mut task: TaskMain,
    manual: bool,
) -> Result<(), String> {
    task.status = start_status(conn, &task)?;
    task.cancel_requested = false;
    task.start_requested = false;
    task.pause_reason.clear();
    if manual {
        task.schedule_override = task.execution_mode == "scheduled";
    } else {
        task.schedule_override = false;
    }
    db::save_task(conn, &task)?;
    db::set_cancel_requested(conn, task.id, false)?;
    log(
        Some(app),
        conn,
        task.id,
        "info",
        if manual {
            "任务由人工启动，等待 Agent 继续执行…"
        } else {
            "任务由调度器启动，等待 Agent 继续执行…"
        },
    );
    emit_update(Some(app), task.id);
    if let Err(e) = orchestrator_spawn(app, task.id) {
        if let Ok(Some(mut failed)) = db::get_task(conn, task.id) {
            failed.status = "failed".into();
            let _ = db::save_task(conn, &failed);
        }
        return Err(e);
    }
    Ok(())
}

fn orchestrator_spawn(app: &AppHandle, task_id: i64) -> Result<(), String> {
    spawn_task(app.clone(), task_id)
}

/// 任务完成后追加新需求：由评审先梳理增量，再继续派发 Coder。
/// simple 只创建一个子任务；complex 也限制在少量模块，避免重复过度设计。
pub fn append_requirement(
    app: Option<&AppHandle>,
    task_id: i64,
    requirement: &str,
    mode: &str,
) -> Result<(), String> {
    let handle = app.ok_or_else(|| "缺少 UI 上下文".to_string())?;
    let requirement = requirement.trim();
    if requirement.is_empty() {
        return Err("新需求不能为空".into());
    }
    if requirement.chars().count() > 12_000 {
        return Err("新需求过长，请控制在 12000 字以内".into());
    }
    if !matches!(mode, "auto" | "single") {
        return Err("追加需求模式必须是 auto 或 single".into());
    }
    let db_path = handle.state::<AppState>().db_path()?;
    let conn = db::open(&db_path)?;
    let mut task = db::get_task(&conn, task_id)?.ok_or_else(|| "任务不存在".to_string())?;
    if task.status != "completed" {
        return Err("只有已完成的任务才支持追加新需求".into());
    }
    if task.task_phase == "requirement_intake" {
        return Err("当前已有追加需求正在梳理".into());
    }
    task.followup_requirement = requirement.to_string();
    task.followup_mode = mode.to_string();
    task.task_phase = "requirement_intake".into();
    task.status = "planning".into();
    task.pause_reason.clear();
    task.cancel_requested = false;
    task.start_requested = false;
    task.schedule_override = task.execution_mode == "scheduled";
    db::save_task(&conn, &task)?;
    record_event(
        Some(handle),
        &conn,
        task_id,
        None,
        "requirement_queued",
        &format!(
            "任务完成后追加新需求（{}）：\n{}",
            if mode == "single" {
                "单模块快改"
            } else {
                "自动判断"
            },
            prompts::truncate_chars(requirement, 4000)
        ),
    );
    log(
        Some(handle),
        &conn,
        task_id,
        "warn",
        &format!(
            "收到追加需求（{}），评审将先梳理增量：简单改动只创建 1 个子任务，不重新拆解已完成项目",
            if mode == "single" {
                "单模块快改"
            } else {
                "自动判断"
            }
        ),
    );

    let settings = db::get_settings(&conn)?;
    if running_count(handle) >= max_concurrent(&settings) {
        task.status = "pending".into();
        task.start_requested = true;
        task.pause_reason = "concurrency".into();
        db::save_task(&conn, &task)?;
        log(
            Some(handle),
            &conn,
            task_id,
            "warn",
            &format!(
                "已达到最大并发任务数（{}），追加需求进入等待队列，有空位后自动开始梳理",
                max_concurrent(&settings)
            ),
        );
        emit_update(Some(handle), task_id);
        return Ok(());
    }
    drop(conn);
    if let Err(e) = spawn_task(handle.clone(), task_id) {
        if let Ok(conn) = db::open(&db_path) {
            if let Ok(Some(mut failed)) = db::get_task(&conn, task_id) {
                failed.status = "failed".into();
                let _ = db::save_task(&conn, &failed);
            }
        }
        return Err(format!("追加需求后启动调度失败：{e}"));
    }
    Ok(())
}

/// 人工点击【运行】：立即执行 / 稍后执行 / 定时执行都从这里进入。
/// 并发上限已满时会转为等待队列，不会静默超发。
pub fn start_task(app: &AppHandle, task_id: i64) -> Result<StartOutcome, String> {
    if app
        .state::<AppState>()
        .running
        .lock()
        .map(|s| s.contains(&task_id))
        .unwrap_or(false)
    {
        return Ok(StartOutcome {
            started: true,
            message: "任务已在运行中".into(),
        });
    }
    let state = app.state::<AppState>();
    let db_path = state.db_path()?;
    let conn = db::open(&db_path)?;
    let task = db::get_task(&conn, task_id)?.ok_or_else(|| "任务不存在".to_string())?;
    if task.status == "completed" {
        return Err("任务已完成".into());
    }
    if task.status == "terminated" {
        return Err("任务已终止，无法再次运行".into());
    }
    if task.task_phase == "plan_approval" {
        return Err("方案正在等待人工审批".into());
    }
    if db::list_subtasks(&conn, task_id)?
        .iter()
        .any(|s| s.current_stage == "design_approval")
    {
        return Err("模块设计正在等待人工审批".into());
    }
    let settings = db::get_settings(&conn)?;
    if running_count(app) >= max_concurrent(&settings) {
        let mut queued = task;
        queued.status = "pending".into();
        queued.start_requested = true;
        queued.pause_reason = "concurrency".into();
        db::save_task(&conn, &queued)?;
        log(
            Some(app),
            &conn,
            task_id,
            "warn",
            &format!(
                "已达到最大并发任务数（{}），任务进入等待队列，有空位后自动启动",
                max_concurrent(&settings)
            ),
        );
        emit_update(Some(app), task_id);
        return Ok(StartOutcome {
            started: false,
            message: "已达到最大并发任务数，任务已进入等待队列".into(),
        });
    }
    let display_name = task.task_name.clone();
    mark_and_spawn(app, &conn, task, true)?;
    Ok(StartOutcome {
        started: true,
        message: format!("任务「{display_name}」已启动"),
    })
}

pub fn resume_task(app: Option<&AppHandle>, task_id: i64) -> Result<(), String> {
    let handle = app.ok_or_else(|| "缺少 UI 上下文".to_string())?;
    let db_path = handle.state::<AppState>().db_path()?;
    let conn = db::open(&db_path)?;
    let mut task = db::get_task(&conn, task_id)?.ok_or_else(|| "任务不存在".to_string())?;
    if task.status == "completed" {
        return Err("任务已完成，无需恢复".into());
    }

    let was_terminated = task.status == "terminated";
    if was_terminated {
        // 终止不再是不可逆操作：继续时保留全部模块/轮次，只清除终止标记。
        task.status = "paused".into();
        task.pause_reason.clear();
        task.cancel_requested = false;
        task.start_requested = false;
        task.schedule_override = false;
        db::save_task(&conn, &task)?;
        log(
            Some(handle),
            &conn,
            task_id,
            "info",
            "继续已终止任务：从当前持久化进度恢复，不重跑已完成模块",
        );
        emit_update(Some(handle), task_id);
    }

    let subtasks = db::list_subtasks(&conn, task_id)?;
    if task.task_phase == "plan_approval" {
        task.status = "needs_human".into();
        task.pause_reason.clear();
        db::save_task(&conn, &task)?;
        emit_update(Some(handle), task_id);
        return Ok(());
    }
    if subtasks
        .iter()
        .any(|s| s.current_stage == "design_approval")
    {
        task.status = "needs_human".into();
        task.pause_reason.clear();
        db::save_task(&conn, &task)?;
        emit_update(Some(handle), task_id);
        return Ok(());
    }
    if subtasks.iter().any(|s| s.status == "needs_human") {
        task.status = "needs_human".into();
        task.pause_reason.clear();
        db::save_task(&conn, &task)?;
        log(
            Some(handle),
            &conn,
            task_id,
            "warn",
            "任务已继续，但仍有模块等待人工介入；请在工作台处理当前模块",
        );
        emit_update(Some(handle), task_id);
        return Ok(());
    }

    drop(conn);
    let outcome = start_task(handle, task_id)?;
    if !outcome.started {
        return Err(outcome.message);
    }
    Ok(())
}

/// 人工暂停：立即停止当前 LLM/dsh，但保留恢复能力与全部数据。
pub fn pause_task(app: &AppHandle, task_id: i64) -> Result<(), String> {
    request_cancel(task_id);
    let state = app.state::<AppState>();
    let db_path = state.db_path()?;
    let conn = db::open(&db_path)?;
    let mut task = db::get_task(&conn, task_id)?.ok_or_else(|| "任务不存在".to_string())?;
    if task.status == "completed" {
        return Err("任务已完成，无法暂停".into());
    }
    if task.status == "terminated" {
        return Err("任务已终止".into());
    }
    task.pause_reason = "manual".into();
    let is_running = app
        .state::<AppState>()
        .running
        .lock()
        .map(|set| set.contains(&task_id))
        .unwrap_or(false);
    if is_running {
        task.cancel_requested = true;
        db::save_task(&conn, &task)?;
        log(
            Some(app),
            &conn,
            task_id,
            "warn",
            "收到人工暂停请求：正在停止当前 dsh/LLM 步骤…",
        );
    } else {
        task.status = "paused".into();
        task.cancel_requested = false;
        db::save_task(&conn, &task)?;
        log(
            Some(app),
            &conn,
            task_id,
            "warn",
            "任务已人工暂停，可点击「恢复」继续",
        );
    }
    emit_update(Some(app), task_id);
    Ok(())
}

/// 人工终止：终止后不再派发任何新子任务，状态保留为 terminated。
pub fn terminate_task(app: &AppHandle, task_id: i64) -> Result<(), String> {
    request_cancel(task_id);
    let state = app.state::<AppState>();
    let db_path = state.db_path()?;
    let conn = db::open(&db_path)?;
    let mut task = db::get_task(&conn, task_id)?.ok_or_else(|| "任务不存在".to_string())?;
    if task.status == "completed" {
        return Err("任务已完成，无法终止".into());
    }
    task.status = "terminated".into();
    task.pause_reason = "terminated".into();
    task.cancel_requested = true;
    task.start_requested = false;
    task.schedule_override = false;
    db::save_task(&conn, &task)?;
    log(
        Some(app),
        &conn,
        task_id,
        "warn",
        "任务已人工终止：不会再派发新的子任务；历史数据与模块快照保留，可随时点击「继续任务」恢复",
    );
    emit_update(Some(app), task_id);
    Ok(())
}

/// 由后台调度线程每 20 秒调用：处理定时窗口、并发队列和立即执行补启动。
pub fn scheduler_tick(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let Ok(db_path) = state.db_path() else {
        return;
    };
    let Ok(conn) = db::open(&db_path) else {
        return;
    };
    let Ok(settings) = db::get_settings(&conn) else {
        return;
    };
    let mut slots = max_concurrent(&settings).saturating_sub(running_count(app));
    if slots == 0 {
        return;
    }
    let Ok(tasks) = db::list_tasks(&conn) else {
        return;
    };
    for summary in tasks {
        if slots == 0 {
            break;
        }
        let task = summary.task;
        let running = app
            .state::<AppState>()
            .running
            .lock()
            .map(|set| set.contains(&task.id))
            .unwrap_or(false);
        if running {
            continue;
        }
        let scheduled_ok = task.execution_mode == "scheduled"
            && task.schedule_enabled
            && settings.scheduled_enabled
            && schedule::now_in_window(&task.schedule_start, &task.schedule_end);
        let should_start = match task.status.as_str() {
            "pending" => task.start_requested || task.execution_mode == "immediate" || scheduled_ok,
            "paused" => task.pause_reason == "waiting_window" && scheduled_ok,
            _ => false,
        };
        if !should_start {
            continue;
        }
        let mut start = task.clone();
        start.status = match start_status(&conn, &start) {
            Ok(s) => s,
            Err(_) => continue,
        };
        start.cancel_requested = false;
        start.start_requested = false;
        start.pause_reason.clear();
        start.schedule_override = false;
        if db::save_task(&conn, &start).is_err() {
            continue;
        }
        log(
            Some(app),
            &conn,
            start.id,
            "info",
            if scheduled_ok {
                "定时窗口已开启：自动继续任务"
            } else {
                "并发队列有空位：自动启动任务"
            },
        );
        emit_update(Some(app), start.id);
        if spawn_task(app.clone(), start.id).is_ok() {
            slots -= 1;
        } else if let Ok(Some(mut failed)) = db::get_task(&conn, start.id) {
            failed.status = "failed".into();
            let _ = db::save_task(&conn, &failed);
        }
    }
}
pub fn global_human_force_pass(app: Option<&AppHandle>, task_id: i64) -> Result<(), String> {
    let handle = app.ok_or_else(|| "缺少 UI 上下文".to_string())?;
    let db_path = handle.state::<AppState>().db_path()?;
    let conn = db::open(&db_path)?;
    let mut task = db::get_task(&conn, task_id)?.ok_or_else(|| "任务不存在".to_string())?;
    if task.task_phase != "global_review" {
        return Err("当前不在全局评审阶段".into());
    }
    if task.status == "completed" {
        return Err("任务已完成".into());
    }
    let round = task.current_global_round.max(1);
    task.global_review_records.push(GlobalReviewRecord {
        cycle: task.global_review_cycle.max(1),
        round,
        passed: true,
        feedback: "人工强制通过全局评审，任务收口完成".into(),
        issues: Vec::new(),
        created_at: db::now(),
    });
    task.status = "completed".into();
    db::save_task(&conn, &task)?;
    log(
        app,
        &conn,
        task_id,
        "warn",
        "🖐️ 人工强制通过全局评审，任务标记完成",
    );
    Ok(())
}

pub fn global_human_rework(
    app: Option<&AppHandle>,
    task_id: i64,
    subtask_id: i64,
    feedback: &str,
) -> Result<(), String> {
    if feedback.trim().is_empty() {
        return Err("返工意见不能为空".into());
    }
    let handle = app.ok_or_else(|| "缺少 UI 上下文".to_string())?;
    let db_path = handle.state::<AppState>().db_path()?;
    let conn = db::open(&db_path)?;
    let mut task = db::get_task(&conn, task_id)?.ok_or_else(|| "任务不存在".to_string())?;
    if task.task_phase != "global_review" {
        return Err("当前不在全局评审阶段".into());
    }
    if task.status == "completed" {
        return Err("任务已完成".into());
    }
    let mut sub = db::get_subtask(&conn, subtask_id)?.ok_or_else(|| "子任务不存在".to_string())?;
    if sub.task_id != task_id {
        return Err("子任务不属于该任务".into());
    }

    let round = task.current_global_round.max(1);
    task.global_review_records.push(GlobalReviewRecord {
        cycle: task.global_review_cycle.max(1),
        round,
        passed: false,
        feedback: "人工指定模块返工".into(),
        issues: vec![GlobalReviewIssue {
            subtask_id: Some(sub.id),
            subtask_name: sub.subtask_name.clone(),
            feedback: feedback.trim().to_string(),
        }],
        created_at: db::now(),
    });
    sub.status = "in_progress".into();
    sub.current_round = 1;
    sub.current_stage = "coding".into();
    sub.iteration_records.push(crate::models::IterationRecord {
        round: 1,
        record_type: "global_rework".into(),
        coder_agent_id: sub.assigned_coder_id,
        diff: String::new(),
        review_feedback: feedback.trim().to_string(),
        review_decision: "reject".into(),
        summary_content: String::new(),
        created_at: db::now(),
    });
    task.task_phase = "module_dev".into();
    task.status = "running".into();
    db::save_subtask(&conn, &sub)?;
    db::save_task(&conn, &task)?;
    log(
        app,
        &conn,
        task_id,
        "warn",
        &format!(
            "人工指定子任务 #{}「{}」全局返工，模块轮次重置为 1",
            sub.sort_index, sub.subtask_name
        ),
    );
    drop(conn);
    spawn_task(handle.clone(), task_id)
}

#[cfg(test)]
mod tests {
    use super::{should_replan, validate_intake};
    use serde_json::json;

    fn intake(decision: &str, names: &[&str]) -> serde_json::Value {
        json!({
            "decision": decision,
            "analysis": "测试梳理结论",
            "subtasks": names
                .iter()
                .map(|n| json!({ "name": n, "accept_criteria": "可验证的验收标准" }))
                .collect::<Vec<_>>()
        })
    }

    #[test]
    fn simple_followup_must_keep_exactly_one_subtask() {
        assert!(validate_intake(intake("simple", &["导出模块"]), &[], false).is_ok());
        assert!(validate_intake(intake("simple", &["导出模块", "样式模块"]), &[], false).is_err());
        assert!(validate_intake(intake("simple", &["导出模块"]), &[], true).is_ok());
        assert!(validate_intake(intake("complex", &["导出模块"]), &[], true).is_err());
    }

    #[test]
    fn complex_followup_is_capped_and_cannot_reuse_completed_names() {
        assert!(validate_intake(intake("complex", &["模块A", "模块B"]), &[], false).is_ok());
        assert!(validate_intake(
            intake(
                "complex",
                &["模块A", "模块B", "模块C", "模块D", "模块E", "模块F", "模块G"]
            ),
            &[],
            false
        )
        .is_err());
        assert!(validate_intake(intake("simple", &["旧模块"]), &["旧模块".into()], false).is_err());
    }

    #[test]
    fn replans_only_when_contract_and_subtasks_are_missing() {
        assert!(should_replan("planning", false, true));
        assert!(should_replan("running", false, true));
        assert!(should_replan("failed", false, true));
        assert!(!should_replan("running", true, false));
        assert!(!should_replan("running", true, true));
        assert!(!should_replan("running", false, false));
        assert!(!should_replan("completed", true, false));
    }
}
