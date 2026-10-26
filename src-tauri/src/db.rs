use std::path::Path;
use std::time::Duration;

use chrono::Local;
use rusqlite::{params, Connection, Row};

use crate::models::{
    AgentConfig, AgentInput, AppSettings, GlobalReviewRecord, HumanMessage, IterationRecord,
    NewTaskInput, PendingPlan, Subtask, TaskConfigInput, TaskDetail, TaskEvent, TaskLog, TaskMain,
    TaskSummary,
};

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS agent_config (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    name          TEXT NOT NULL,
    role_type     TEXT NOT NULL CHECK (role_type IN ('reviewer','coder')),
    provider      TEXT NOT NULL DEFAULT '',
    model_name    TEXT NOT NULL DEFAULT '',
    base_url      TEXT NOT NULL DEFAULT '',
    api_key       TEXT NOT NULL DEFAULT '',
    system_prompt TEXT NOT NULL DEFAULT '',
    temperature   REAL NOT NULL DEFAULT 0.3,
    create_time   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS task_main (
    id                     INTEGER PRIMARY KEY AUTOINCREMENT,
    task_name              TEXT NOT NULL,
    top_requirement        TEXT NOT NULL,
    global_accept_criteria TEXT NOT NULL,
    reviewer_agent_id      INTEGER NOT NULL,
    coder_agent_ids        TEXT NOT NULL,              -- JSON: [1,2]
    module_max_round       INTEGER NOT NULL,
    global_max_round       INTEGER NOT NULL,
    global_contract        TEXT NOT NULL DEFAULT '',
    task_phase             TEXT NOT NULL DEFAULT 'module_dev',
    status                 TEXT NOT NULL DEFAULT 'planning',
    work_dir               TEXT NOT NULL,
    current_global_round   INTEGER NOT NULL DEFAULT 0,
    global_review_records  TEXT NOT NULL DEFAULT '[]',
    baseline_tree          TEXT NOT NULL DEFAULT '',
    manual_review_enabled  INTEGER NOT NULL DEFAULT 0,
    tech_stack             TEXT NOT NULL DEFAULT '',
    pending_plan_json      TEXT NOT NULL DEFAULT '',
    cancel_requested       INTEGER NOT NULL DEFAULT 0,
    create_time            TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS subtask (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id           INTEGER NOT NULL REFERENCES task_main(id) ON DELETE CASCADE,
    subtask_name      TEXT NOT NULL,
    sort_index        INTEGER NOT NULL,
    accept_criteria   TEXT NOT NULL DEFAULT '',
    assigned_coder_id INTEGER,
    current_round     INTEGER NOT NULL DEFAULT 0,
    max_round         INTEGER NOT NULL,
    status            TEXT NOT NULL DEFAULT 'waiting',
    current_stage     TEXT NOT NULL DEFAULT 'queued',
    summary_content   TEXT NOT NULL DEFAULT '',
    summary_file_path TEXT NOT NULL DEFAULT '',
    iteration_records TEXT NOT NULL DEFAULT '[]',
    design_content    TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS task_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id     INTEGER NOT NULL REFERENCES task_main(id) ON DELETE CASCADE,
    level       TEXT NOT NULL DEFAULT 'info',
    message     TEXT NOT NULL,
    create_time TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS app_setting (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS task_event (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id     INTEGER NOT NULL REFERENCES task_main(id) ON DELETE CASCADE,
    subtask_id  INTEGER,
    kind        TEXT NOT NULL,
    content     TEXT NOT NULL,
    create_time TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS human_message (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id     INTEGER NOT NULL REFERENCES task_main(id) ON DELETE CASCADE,
    content     TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'pending',
    create_time TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_subtask_task ON subtask(task_id, sort_index);
CREATE INDEX IF NOT EXISTS idx_task_log_task ON task_log(task_id, id);
CREATE INDEX IF NOT EXISTS idx_task_event_task ON task_event(task_id, id);
CREATE INDEX IF NOT EXISTS idx_human_message_task ON human_message(task_id, id);
"#;

pub fn now() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// 初始化本地数据库，返回 db 文件路径。
pub fn init(app_data_dir: &Path) -> Result<std::path::PathBuf, String> {
    std::fs::create_dir_all(app_data_dir).map_err(|e| e.to_string())?;
    let path = app_data_dir.join("orchestrator.db");
    let conn = open(&path)?;
    conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    migrate(&conn)?;
    seed_settings(&conn)?;
    Ok(path)
}

pub fn open(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    conn.busy_timeout(Duration::from_secs(8))
        .map_err(|e| e.to_string())?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;
         PRAGMA synchronous=NORMAL;",
    )
    .map_err(|e| e.to_string())?;
    Ok(conn)
}

/// 为旧数据库补齐新增列（SQLite 的 ADD COLUMN 不支持 IF NOT EXISTS）。
fn ensure_column(conn: &Connection, table: &str, column: &str, ddl: &str) -> Result<(), String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| e.to_string())?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    if !names.iter().any(|n| n == column) {
        conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {ddl}"), [])
            .map_err(|e| format!("数据库迁移失败（{table}.{column}）: {e}"))?;
    }
    Ok(())
}

fn migrate(conn: &Connection) -> Result<(), String> {
    ensure_column(
        conn,
        "task_main",
        "manual_review_enabled",
        "manual_review_enabled INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(
        conn,
        "task_main",
        "tech_stack",
        "tech_stack TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        conn,
        "task_main",
        "pending_plan_json",
        "pending_plan_json TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        conn,
        "task_main",
        "cancel_requested",
        "cancel_requested INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(
        conn,
        "subtask",
        "design_content",
        "design_content TEXT NOT NULL DEFAULT ''",
    )?;
    Ok(())
}

fn seed_settings(conn: &Connection) -> Result<(), String> {
    let defaults = AppSettings::default();
    let pairs = [
        ("default_work_dir", defaults.default_work_dir.as_str()),
        ("llm_timeout_sec", defaults.llm_timeout_sec.as_str()),
        ("dsh_timeout_sec", defaults.dsh_timeout_sec.as_str()),
        ("dsh_bin", defaults.dsh_bin.as_str()),
        ("dsh_profile", defaults.dsh_profile.as_str()),
    ];
    for (k, v) in pairs {
        conn.execute(
            "INSERT OR IGNORE INTO app_setting(key, value) VALUES (?1, ?2)",
            params![k, v],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ------------------------------------------------------------------
// JSON helper
// ------------------------------------------------------------------
fn parse_json<T: serde::de::DeserializeOwned + Default>(raw: &str) -> T {
    serde_json::from_str(raw).unwrap_or_default()
}

// ------------------------------------------------------------------
// Agent
// ------------------------------------------------------------------
fn row_to_agent(row: &Row<'_>) -> rusqlite::Result<AgentConfig> {
    Ok(AgentConfig {
        id: row.get("id")?,
        name: row.get("name")?,
        role_type: row.get("role_type")?,
        provider: row.get("provider")?,
        model_name: row.get("model_name")?,
        base_url: row.get("base_url")?,
        api_key: row.get("api_key")?,
        system_prompt: row.get("system_prompt")?,
        temperature: row.get("temperature")?,
        create_time: row.get("create_time")?,
    })
}

pub fn list_agents(conn: &Connection) -> Result<Vec<AgentConfig>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id,name,role_type,provider,model_name,base_url,api_key,system_prompt,temperature,create_time
             FROM agent_config ORDER BY CASE role_type WHEN 'reviewer' THEN 0 ELSE 1 END, id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], row_to_agent)
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

pub fn get_agent(conn: &Connection, id: i64) -> Result<Option<AgentConfig>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id,name,role_type,provider,model_name,base_url,api_key,system_prompt,temperature,create_time
             FROM agent_config WHERE id = ?1",
        )
        .map_err(|e| e.to_string())?;
    let mut rows = stmt.query(params![id]).map_err(|e| e.to_string())?;
    match rows.next().map_err(|e| e.to_string())? {
        Some(row) => Ok(Some(row_to_agent(row).map_err(|e| e.to_string())?)),
        None => Ok(None),
    }
}

pub fn save_agent(conn: &Connection, input: &AgentInput) -> Result<AgentConfig, String> {
    if input.name.trim().is_empty() {
        return Err("Agent 名称不能为空".into());
    }
    if input.role_type != "reviewer" && input.role_type != "coder" {
        return Err("角色类型必须是 reviewer 或 coder".into());
    }
    if input.model_name.trim().is_empty() {
        return Err("model 名称不能为空".into());
    }
    if input.base_url.trim().is_empty() {
        return Err("baseUrl 不能为空".into());
    }
    let temperature = input.temperature.clamp(0.0, 2.0);
    if let Some(id) = input.id {
        conn.execute(
            "UPDATE agent_config SET name=?1, role_type=?2, provider=?3, model_name=?4,
                 base_url=?5, api_key=?6, system_prompt=?7, temperature=?8 WHERE id=?9",
            params![
                input.name.trim(),
                input.role_type,
                input.provider,
                input.model_name.trim(),
                input.base_url.trim(),
                input.api_key,
                input.system_prompt,
                temperature,
                id
            ],
        )
        .map_err(|e| e.to_string())?;
        get_agent(conn, id)?.ok_or_else(|| "Agent 不存在".to_string())
    } else {
        let create_time = now();
        conn.execute(
            "INSERT INTO agent_config(name,role_type,provider,model_name,base_url,api_key,system_prompt,temperature,create_time)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                input.name.trim(),
                input.role_type,
                input.provider,
                input.model_name.trim(),
                input.base_url.trim(),
                input.api_key,
                input.system_prompt,
                temperature,
                create_time
            ],
        )
        .map_err(|e| e.to_string())?;
        get_agent(conn, conn.last_insert_rowid())?.ok_or_else(|| "Agent 创建失败".to_string())
    }
}

pub fn delete_agent(conn: &Connection, id: i64) -> Result<(), String> {
    // 被任务引用时禁止删除，避免历史任务显示/执行断裂。
    let mut stmt = conn
        .prepare("SELECT reviewer_agent_id, coder_agent_ids FROM task_main")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            let reviewer: i64 = row.get(0)?;
            let coders: String = row.get(1)?;
            Ok((reviewer, coders))
        })
        .map_err(|e| e.to_string())?;
    for row in rows {
        let (reviewer, coders_raw) = row.map_err(|e| e.to_string())?;
        let coders: Vec<i64> = parse_json(&coders_raw);
        if reviewer == id || coders.contains(&id) {
            return Err("该 Agent 已被任务引用，无法删除".into());
        }
    }
    conn.execute("DELETE FROM agent_config WHERE id=?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ------------------------------------------------------------------
// Task
// ------------------------------------------------------------------
fn row_to_task(row: &Row<'_>) -> rusqlite::Result<TaskMain> {
    let coders_raw: String = row.get("coder_agent_ids")?;
    let records_raw: String = row.get("global_review_records")?;
    Ok(TaskMain {
        id: row.get("id")?,
        task_name: row.get("task_name")?,
        top_requirement: row.get("top_requirement")?,
        global_accept_criteria: row.get("global_accept_criteria")?,
        reviewer_agent_id: row.get("reviewer_agent_id")?,
        coder_agent_ids: parse_json(&coders_raw),
        module_max_round: row.get("module_max_round")?,
        global_max_round: row.get("global_max_round")?,
        global_contract: row.get("global_contract")?,
        task_phase: row.get("task_phase")?,
        status: row.get("status")?,
        work_dir: row.get("work_dir")?,
        current_global_round: row.get("current_global_round")?,
        global_review_records: parse_json(&records_raw),
        create_time: row.get("create_time")?,
        baseline_tree: row.get("baseline_tree").unwrap_or_default(),
        manual_review_enabled: row.get::<_, i64>("manual_review_enabled").unwrap_or(0) != 0,
        tech_stack: row.get("tech_stack").unwrap_or_default(),
        pending_plan_json: row.get("pending_plan_json").unwrap_or_default(),
        cancel_requested: row.get::<_, i64>("cancel_requested").unwrap_or(0) != 0,
    })
}

const TASK_COLS: &str = "id,task_name,top_requirement,global_accept_criteria,reviewer_agent_id,
    coder_agent_ids,module_max_round,global_max_round,global_contract,task_phase,status,work_dir,
    current_global_round,global_review_records,baseline_tree,manual_review_enabled,tech_stack,
    pending_plan_json,cancel_requested,create_time";

pub fn list_tasks(conn: &Connection) -> Result<Vec<TaskSummary>, String> {
    let agents = list_agents(conn)?;
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {TASK_COLS} FROM task_main ORDER BY id DESC"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], row_to_task).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let task = row.map_err(|e| e.to_string())?;
        let reviewer_name = agents
            .iter()
            .find(|a| a.id == task.reviewer_agent_id)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| "（已删除）".into());
        let coder_names = task
            .coder_agent_ids
            .iter()
            .map(|id| {
                agents
                    .iter()
                    .find(|a| a.id == *id)
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| format!("（已删除#{id}）"))
            })
            .collect::<Vec<_>>();
        let (subtask_count, completed_count) = subtask_counts(conn, task.id)?;
        out.push(TaskSummary {
            task,
            reviewer_name,
            coder_names,
            subtask_count,
            completed_count,
        });
    }
    Ok(out)
}

pub fn subtask_counts(conn: &Connection, task_id: i64) -> Result<(i64, i64), String> {
    let total: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM subtask WHERE task_id=?1",
            params![task_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let done: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM subtask WHERE task_id=?1 AND status='completed'",
            params![task_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok((total, done))
}

pub fn get_task(conn: &Connection, id: i64) -> Result<Option<TaskMain>, String> {
    let mut stmt = conn
        .prepare(&format!("SELECT {TASK_COLS} FROM task_main WHERE id=?1"))
        .map_err(|e| e.to_string())?;
    let mut rows = stmt.query(params![id]).map_err(|e| e.to_string())?;
    match rows.next().map_err(|e| e.to_string())? {
        Some(row) => Ok(Some(row_to_task(row).map_err(|e| e.to_string())?)),
        None => Ok(None),
    }
}

pub fn create_task(conn: &Connection, input: &NewTaskInput) -> Result<i64, String> {
    if input.task_name.trim().is_empty() {
        return Err("任务名称不能为空".into());
    }
    if input.top_requirement.trim().is_empty() {
        return Err("顶层需求不能为空".into());
    }
    if input.global_accept_criteria.trim().is_empty() {
        return Err("项目验收标准不能为空".into());
    }
    if input.coder_agent_ids.is_empty() {
        return Err("至少选择一个 Coder".into());
    }
    if input.module_max_round < 1 || input.global_max_round < 1 {
        return Err("轮次上限必须大于等于 1".into());
    }
    let coders_json = serde_json::to_string(&input.coder_agent_ids)
        .map_err(|e| format!("coder 列表序列化失败: {e}"))?;
    conn.execute(
        "INSERT INTO task_main(task_name,top_requirement,global_accept_criteria,reviewer_agent_id,
            coder_agent_ids,module_max_round,global_max_round,global_contract,task_phase,status,
            work_dir,current_global_round,global_review_records,manual_review_enabled,create_time)
         VALUES (?1,?2,?3,?4,?5,?6,?7,'','module_dev','planning',?8,0,'[]',?9,?10)",
        params![
            input.task_name.trim(),
            input.top_requirement.trim(),
            input.global_accept_criteria.trim(),
            input.reviewer_agent_id,
            coders_json,
            input.module_max_round,
            input.global_max_round,
            input.work_dir.trim(),
            if input.manual_review_enabled { 1 } else { 0 },
            now()
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

/// 只持久化调度过程中会变化的字段。
pub fn save_task(conn: &Connection, task: &TaskMain) -> Result<(), String> {
    let records = serde_json::to_string(&task.global_review_records)
        .map_err(|e| format!("全局评审记录序列化失败: {e}"))?;
    conn.execute(
        "UPDATE task_main SET global_contract=?1, task_phase=?2, status=?3,
            current_global_round=?4, global_review_records=?5, baseline_tree=?6,
            tech_stack=?7, pending_plan_json=?8 WHERE id=?9",
        params![
            task.global_contract,
            task.task_phase,
            task.status,
            task.current_global_round,
            records,
            task.baseline_tree,
            task.tech_stack,
            task.pending_plan_json,
            task.id
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_task(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM task_main WHERE id=?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ------------------------------------------------------------------
// Subtask
// ------------------------------------------------------------------
fn row_to_subtask(row: &Row<'_>) -> rusqlite::Result<Subtask> {
    let records_raw: String = row.get("iteration_records")?;
    Ok(Subtask {
        id: row.get("id")?,
        task_id: row.get("task_id")?,
        subtask_name: row.get("subtask_name")?,
        sort_index: row.get("sort_index")?,
        accept_criteria: row.get("accept_criteria")?,
        assigned_coder_id: row.get("assigned_coder_id")?,
        current_round: row.get("current_round")?,
        max_round: row.get("max_round")?,
        status: row.get("status")?,
        current_stage: row.get("current_stage")?,
        summary_content: row.get("summary_content")?,
        summary_file_path: row.get("summary_file_path")?,
        iteration_records: parse_json(&records_raw),
        design_content: row.get("design_content").unwrap_or_default(),
    })
}

const SUBTASK_COLS: &str = "id,task_id,subtask_name,sort_index,accept_criteria,assigned_coder_id,
    current_round,max_round,status,current_stage,summary_content,summary_file_path,iteration_records,
    design_content";

pub fn list_subtasks(conn: &Connection, task_id: i64) -> Result<Vec<Subtask>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {SUBTASK_COLS} FROM subtask WHERE task_id=?1 ORDER BY sort_index"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![task_id], row_to_subtask)
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

pub fn get_subtask(conn: &Connection, id: i64) -> Result<Option<Subtask>, String> {
    let mut stmt = conn
        .prepare(&format!("SELECT {SUBTASK_COLS} FROM subtask WHERE id=?1"))
        .map_err(|e| e.to_string())?;
    let mut rows = stmt.query(params![id]).map_err(|e| e.to_string())?;
    match rows.next().map_err(|e| e.to_string())? {
        Some(row) => Ok(Some(row_to_subtask(row).map_err(|e| e.to_string())?)),
        None => Ok(None),
    }
}

pub fn insert_subtasks(
    conn: &mut Connection,
    task_id: i64,
    items: &[(String, String)],
    coder_id: i64,
    max_round: i64,
) -> Result<Vec<Subtask>, String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for (i, (name, criteria)) in items.iter().enumerate() {
        tx.execute(
            "INSERT INTO subtask(task_id,subtask_name,sort_index,accept_criteria,assigned_coder_id,
                current_round,max_round,status,current_stage,summary_content,summary_file_path,iteration_records,design_content)
             VALUES (?1,?2,?3,?4,?5,0,?6,'waiting','queued','','','[]','')",
            params![
                task_id,
                name.trim(),
                (i + 1) as i64,
                criteria,
                coder_id,
                max_round
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    list_subtasks(conn, task_id)
}

pub fn save_subtask(conn: &Connection, subtask: &Subtask) -> Result<(), String> {
    let records = serde_json::to_string(&subtask.iteration_records)
        .map_err(|e| format!("迭代记录序列化失败: {e}"))?;
    conn.execute(
        "UPDATE subtask SET assigned_coder_id=?1, current_round=?2, status=?3, current_stage=?4,
            summary_content=?5, summary_file_path=?6, iteration_records=?7, design_content=?8
            WHERE id=?9",
        params![
            subtask.assigned_coder_id,
            subtask.current_round,
            subtask.status,
            subtask.current_stage,
            subtask.summary_content,
            subtask.summary_file_path,
            records,
            subtask.design_content,
            subtask.id
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ------------------------------------------------------------------
// Logs
// ------------------------------------------------------------------
fn row_to_log(row: &Row<'_>) -> rusqlite::Result<TaskLog> {
    Ok(TaskLog {
        id: row.get("id")?,
        task_id: row.get("task_id")?,
        level: row.get("level")?,
        message: row.get("message")?,
        create_time: row.get("create_time")?,
    })
}

pub fn add_log(
    conn: &Connection,
    task_id: i64,
    level: &str,
    message: &str,
) -> Result<TaskLog, String> {
    let create_time = now();
    conn.execute(
        "INSERT INTO task_log(task_id,level,message,create_time) VALUES (?1,?2,?3,?4)",
        params![task_id, level, message, create_time],
    )
    .map_err(|e| e.to_string())?;
    Ok(TaskLog {
        id: conn.last_insert_rowid(),
        task_id,
        level: level.to_string(),
        message: message.to_string(),
        create_time,
    })
}

pub fn list_logs(conn: &Connection, task_id: i64) -> Result<Vec<TaskLog>, String> {
    // 只取最近 800 条，避免超长任务把页面拖慢。
    let mut stmt = conn
        .prepare(
            "SELECT id,task_id,level,message,create_time FROM (
                SELECT id,task_id,level,message,create_time FROM task_log
                WHERE task_id=?1 ORDER BY id DESC LIMIT 800
             ) ORDER BY id ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![task_id], row_to_log)
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

// ------------------------------------------------------------------
// Task detail
// ------------------------------------------------------------------
pub fn get_task_detail(conn: &Connection, task_id: i64) -> Result<TaskDetail, String> {
    let task = get_task(conn, task_id)?.ok_or_else(|| "任务不存在".to_string())?;
    let subtasks = list_subtasks(conn, task_id)?;
    let logs = list_logs(conn, task_id)?;
    let reviewer = get_agent(conn, task.reviewer_agent_id)?;
    let mut coders = Vec::new();
    for id in &task.coder_agent_ids {
        if let Some(a) = get_agent(conn, *id)? {
            coders.push(a);
        }
    }
    let execution_events = list_events(conn, task_id, 2000)?;
    let human_messages = list_human_messages(conn, task_id)?;
    let pending_plan = parse_json::<PendingPlan>(&task.pending_plan_json);
    let pending_plan = if task.pending_plan_json.trim().is_empty() {
        None
    } else {
        Some(pending_plan)
    };
    Ok(TaskDetail {
        task,
        subtasks,
        logs,
        reviewer,
        coders,
        execution_events,
        human_messages,
        pending_plan,
    })
}

// ------------------------------------------------------------------
// Settings
// ------------------------------------------------------------------
pub fn get_settings(conn: &Connection) -> Result<AppSettings, String> {
    let mut settings = AppSettings::default();
    let mut stmt = conn
        .prepare("SELECT key,value FROM app_setting")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;
    for row in rows {
        let (k, v) = row.map_err(|e| e.to_string())?;
        match k.as_str() {
            "default_work_dir" => settings.default_work_dir = v,
            "llm_timeout_sec" => settings.llm_timeout_sec = v,
            "dsh_timeout_sec" => settings.dsh_timeout_sec = v,
            "dsh_bin" => settings.dsh_bin = v,
            "dsh_profile" => settings.dsh_profile = v,
            _ => {}
        }
    }
    Ok(settings)
}

pub fn save_settings(conn: &Connection, settings: &AppSettings) -> Result<(), String> {
    let pairs = [
        ("default_work_dir", settings.default_work_dir.as_str()),
        ("llm_timeout_sec", settings.llm_timeout_sec.as_str()),
        ("dsh_timeout_sec", settings.dsh_timeout_sec.as_str()),
        ("dsh_bin", settings.dsh_bin.as_str()),
        ("dsh_profile", settings.dsh_profile.as_str()),
    ];
    for (k, v) in pairs {
        conn.execute(
            "INSERT INTO app_setting(key,value) VALUES (?1,?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![k, v],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ------------------------------------------------------------------
// 辅助
// ------------------------------------------------------------------
/// 构造送入评审的上下文：只包含已完成模块的摘要，绝不包含历史对话。
pub fn completed_summaries(subtasks: &[Subtask]) -> String {
    let mut out = String::new();
    for s in subtasks.iter().filter(|s| s.status == "completed") {
        out.push_str(&format!(
            "\n===== 已完成模块 #{} {} =====\n{}\n",
            s.sort_index, s.subtask_name, s.summary_content
        ));
    }
    if out.is_empty() {
        "（暂无已完成模块摘要）".to_string()
    } else {
        out
    }
}

// 明确导出，避免 dead_code 警告干扰。
pub fn _touch_types() {
    let _: Option<IterationRecord> = None;
    let _: Option<GlobalReviewRecord> = None;
}

/// 应用重启后，之前处于 running/planning 的任务已没有调度线程，标记为 failed 供人工恢复。
pub fn recover_interrupted(conn: &Connection) -> Result<usize, String> {
    let _ = conn.execute("UPDATE task_main SET cancel_requested=0", []);
    let mut stmt = conn
        .prepare("SELECT id,status FROM task_main WHERE status IN ('running','planning')")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;
    let mut items = Vec::new();
    for row in rows {
        items.push(row.map_err(|e| e.to_string())?);
    }
    for (id, status) in &items {
        conn.execute(
            "UPDATE task_main SET status='failed' WHERE id=?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
        add_log(
            conn,
            *id,
            "warn",
            &format!(
                "应用重启：上次运行的调度已中断（原状态 {status}），可在工作台点击「恢复调度」"
            ),
        )?;
    }
    Ok(items.len())
}

// ------------------------------------------------------------------
// 运行中配置 / 人工审批 / 执行事件 / 人工消息
// ------------------------------------------------------------------
pub fn update_task_config(
    conn: &Connection,
    task_id: i64,
    cfg: &TaskConfigInput,
) -> Result<TaskMain, String> {
    if cfg.module_max_round < 1 || cfg.global_max_round < 1 {
        return Err("轮次上限必须大于等于 1".into());
    }
    conn.execute(
        "UPDATE task_main SET module_max_round=?1, global_max_round=?2, manual_review_enabled=?3 WHERE id=?4",
        params![
            cfg.module_max_round,
            cfg.global_max_round,
            if cfg.manual_review_enabled { 1 } else { 0 },
            task_id
        ],
    )
    .map_err(|e| e.to_string())?;
    // 运行中修改模块轮次：同步到所有子任务，下一轮重新加载时立即生效。
    conn.execute(
        "UPDATE subtask SET max_round=?1 WHERE task_id=?2",
        params![cfg.module_max_round, task_id],
    )
    .map_err(|e| e.to_string())?;
    get_task(conn, task_id)?.ok_or_else(|| "任务不存在".to_string())
}

pub fn set_cancel_requested(
    conn: &Connection,
    task_id: i64,
    requested: bool,
) -> Result<(), String> {
    conn.execute(
        "UPDATE task_main SET cancel_requested=?1 WHERE id=?2",
        params![if requested { 1 } else { 0 }, task_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn row_to_event(row: &Row<'_>) -> rusqlite::Result<TaskEvent> {
    Ok(TaskEvent {
        id: row.get("id")?,
        task_id: row.get("task_id")?,
        subtask_id: row.get("subtask_id")?,
        kind: row.get("kind")?,
        content: row.get("content")?,
        create_time: row.get("create_time")?,
    })
}

pub fn add_event(
    conn: &Connection,
    task_id: i64,
    subtask_id: Option<i64>,
    kind: &str,
    content: &str,
) -> Result<TaskEvent, String> {
    let create_time = now();
    conn.execute(
        "INSERT INTO task_event(task_id,subtask_id,kind,content,create_time)
         VALUES (?1,?2,?3,?4,?5)",
        params![task_id, subtask_id, kind, content, create_time],
    )
    .map_err(|e| e.to_string())?;
    Ok(TaskEvent {
        id: conn.last_insert_rowid(),
        task_id,
        subtask_id,
        kind: kind.to_string(),
        content: content.to_string(),
        create_time,
    })
}

pub fn list_events(conn: &Connection, task_id: i64, limit: i64) -> Result<Vec<TaskEvent>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id,task_id,subtask_id,kind,content,create_time FROM (
                SELECT id,task_id,subtask_id,kind,content,create_time FROM task_event
                WHERE task_id=?1 ORDER BY id DESC LIMIT ?2
             ) ORDER BY id ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![task_id, limit], row_to_event)
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

pub fn add_human_message(
    conn: &Connection,
    task_id: i64,
    content: &str,
) -> Result<HumanMessage, String> {
    let create_time = now();
    conn.execute(
        "INSERT INTO human_message(task_id,content,status,create_time) VALUES (?1,?2,'pending',?3)",
        params![task_id, content, create_time],
    )
    .map_err(|e| e.to_string())?;
    Ok(HumanMessage {
        id: conn.last_insert_rowid(),
        task_id,
        content: content.to_string(),
        status: "pending".into(),
        create_time,
    })
}

pub fn list_human_messages(conn: &Connection, task_id: i64) -> Result<Vec<HumanMessage>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id,task_id,content,status,create_time FROM human_message
             WHERE task_id=?1 ORDER BY id ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![task_id], |row| {
            Ok(HumanMessage {
                id: row.get("id")?,
                task_id: row.get("task_id")?,
                content: row.get("content")?,
                status: row.get("status")?,
                create_time: row.get("create_time")?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

/// 取出所有 pending 人工消息并标记 consumed；返回内容用于注入 prompt。
pub fn consume_human_messages(conn: &Connection, task_id: i64) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id,content FROM human_message WHERE task_id=?1 AND status='pending' ORDER BY id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![task_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;
    let mut items = Vec::new();
    for row in rows {
        items.push(row.map_err(|e| e.to_string())?);
    }
    for (id, _) in &items {
        conn.execute(
            "UPDATE human_message SET status='consumed' WHERE id=?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(items.into_iter().map(|(_, content)| content).collect())
}

/// 审批通过后，把 pending_plan_json 正式落成 subtask。
pub fn apply_pending_plan(
    conn: &mut Connection,
    task: &mut TaskMain,
    plan: &PendingPlan,
) -> Result<(), String> {
    if plan.subtasks.is_empty() {
        return Err("待审批方案没有子任务".into());
    }
    let first_coder = task
        .coder_agent_ids
        .first()
        .copied()
        .ok_or_else(|| "任务未绑定 Coder".to_string())?;
    let items: Vec<(String, String)> = plan
        .subtasks
        .iter()
        .map(|i| {
            (
                i.name.trim().to_string(),
                i.accept_criteria.trim().to_string(),
            )
        })
        .collect();
    insert_subtasks(conn, task.id, &items, first_coder, task.module_max_round)?;
    task.global_contract = plan.global_contract.trim().to_string();
    task.tech_stack = plan.tech_stack.trim().to_string();
    task.pending_plan_json.clear();
    task.task_phase = "module_dev".into();
    task.status = "running".into();
    save_task(conn, task)?;
    Ok(())
}

pub fn set_paused(conn: &Connection, task_id: i64, message: &str) -> Result<(), String> {
    if let Some(mut task) = get_task(conn, task_id)? {
        task.status = "paused".into();
        task.cancel_requested = false;
        save_task(conn, &task)?;
        set_cancel_requested(conn, task_id, false)?;
    }
    add_log(conn, task_id, "warn", message)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 关键：旧版本数据库必须能无损升级，不能丢 Agent/任务数据。
    #[test]
    fn migrates_old_database_without_losing_rows() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("orch-migrate-{}-{}", std::process::id(), nanos));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("orchestrator.db");
        {
            let conn = Connection::open(&path).unwrap();
            // 模拟旧版本表结构（没有新列，也没有新表）。
            conn.execute_batch(
                r#"
                CREATE TABLE task_main (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    task_name TEXT NOT NULL,
                    top_requirement TEXT NOT NULL,
                    global_accept_criteria TEXT NOT NULL,
                    reviewer_agent_id INTEGER NOT NULL,
                    coder_agent_ids TEXT NOT NULL,
                    module_max_round INTEGER NOT NULL,
                    global_max_round INTEGER NOT NULL,
                    global_contract TEXT NOT NULL DEFAULT '',
                    task_phase TEXT NOT NULL DEFAULT 'module_dev',
                    status TEXT NOT NULL DEFAULT 'planning',
                    work_dir TEXT NOT NULL,
                    current_global_round INTEGER NOT NULL DEFAULT 0,
                    global_review_records TEXT NOT NULL DEFAULT '[]',
                    baseline_tree TEXT NOT NULL DEFAULT '',
                    create_time TEXT NOT NULL
                );
                CREATE TABLE subtask (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    task_id INTEGER NOT NULL,
                    subtask_name TEXT NOT NULL,
                    sort_index INTEGER NOT NULL,
                    accept_criteria TEXT NOT NULL DEFAULT '',
                    assigned_coder_id INTEGER,
                    current_round INTEGER NOT NULL DEFAULT 0,
                    max_round INTEGER NOT NULL,
                    status TEXT NOT NULL DEFAULT 'waiting',
                    current_stage TEXT NOT NULL DEFAULT 'queued',
                    summary_content TEXT NOT NULL DEFAULT '',
                    summary_file_path TEXT NOT NULL DEFAULT '',
                    iteration_records TEXT NOT NULL DEFAULT '[]'
                );
                INSERT INTO task_main(task_name,top_requirement,global_accept_criteria,reviewer_agent_id,
                    coder_agent_ids,module_max_round,global_max_round,work_dir,create_time)
                VALUES ('旧任务','需求','验收',1,'[1]',3,2,'/tmp','2026-01-01 00:00:00');
                "#,
            )
            .unwrap();
            conn.execute_batch(SCHEMA).unwrap();
            migrate(&conn).unwrap();
            let task = get_task(&conn, 1).unwrap().unwrap();
            assert_eq!(task.task_name, "旧任务");
            assert!(!task.manual_review_enabled);
            assert_eq!(task.tech_stack, "");
            // 新表存在
            let n: i64 = conn
                .query_row("SELECT COUNT(*) FROM task_event", [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 0);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
