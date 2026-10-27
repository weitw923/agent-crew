use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

use chrono::{Local, NaiveDateTime};
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::models::{
    AgentConfig, AgentInput, AppSettings, ChatMessage, GlobalReviewRecord, HumanMessage, Iteration,
    IterationRecord, NewTaskInput, PendingPlan, RoleInput, RoleType, Subtask, TaskConfigInput,
    TaskDetail, TaskEvent, TaskLog, TaskMain, TaskSummary, WorkflowStep,
};

/// 运行日志是否持久化。关闭后事件仍会实时推送前端，但不写入数据库。
static LOG_PERSIST: AtomicBool = AtomicBool::new(true);
static SYNTHETIC_ID: AtomicI64 = AtomicI64::new(-1);

pub fn set_log_persistence(enabled: bool) {
    LOG_PERSIST.store(enabled, Ordering::SeqCst);
}

pub fn log_persistence() -> bool {
    LOG_PERSIST.load(Ordering::SeqCst)
}

fn synthetic_id() -> i64 {
    SYNTHETIC_ID.fetch_sub(1, Ordering::SeqCst)
}

fn valid_clock(raw: &str) -> bool {
    let mut parts = raw.trim().split(':');
    let (Some(h), Some(m), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let (Ok(h), Ok(m)) = (h.parse::<u32>(), m.parse::<u32>()) else {
        return false;
    };
    h < 24 && m < 60
}

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS role_type (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    key          TEXT NOT NULL UNIQUE,
    name         TEXT NOT NULL,
    description  TEXT NOT NULL DEFAULT '',
    capabilities TEXT NOT NULL DEFAULT '[]',
    color        TEXT NOT NULL DEFAULT '',
    is_builtin   INTEGER NOT NULL DEFAULT 0,
    sort_order   INTEGER NOT NULL DEFAULT 0,
    create_time  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS agent_config (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    name          TEXT NOT NULL,
    role_type     TEXT NOT NULL,
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
    workflow_agent_ids     TEXT NOT NULL DEFAULT '',   -- JSON: [3,1,2]，按顺序的线性工作流
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
    execution_mode         TEXT NOT NULL DEFAULT 'later',
    schedule_start         TEXT NOT NULL DEFAULT '01:00',
    schedule_end           TEXT NOT NULL DEFAULT '06:00',
    schedule_enabled       INTEGER NOT NULL DEFAULT 1,
    schedule_override      INTEGER NOT NULL DEFAULT 0,
    start_requested        INTEGER NOT NULL DEFAULT 0,
    pause_reason           TEXT NOT NULL DEFAULT '',
    followup_requirement   TEXT NOT NULL DEFAULT '',
    followup_mode          TEXT NOT NULL DEFAULT 'auto',
    global_review_cycle    INTEGER NOT NULL DEFAULT 1,
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
    design_content    TEXT NOT NULL DEFAULT '',
    iteration_id      INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS task_log (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id      INTEGER NOT NULL REFERENCES task_main(id) ON DELETE CASCADE,
    iteration_id INTEGER NOT NULL DEFAULT 0,
    level        TEXT NOT NULL DEFAULT 'info',
    message      TEXT NOT NULL,
    create_time  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS iteration (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id      INTEGER NOT NULL REFERENCES task_main(id) ON DELETE CASCADE,
    version      TEXT NOT NULL DEFAULT '',
    major        INTEGER NOT NULL DEFAULT 0,
    minor        INTEGER NOT NULL DEFAULT 0,
    requirement  TEXT NOT NULL DEFAULT '',
    analysis     TEXT NOT NULL DEFAULT '',
    status       TEXT NOT NULL DEFAULT 'pending',
    start_time   TEXT NOT NULL DEFAULT '',
    end_time     TEXT NOT NULL DEFAULT '',
    token_total  INTEGER NOT NULL DEFAULT -1,
    create_time  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS app_setting (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS task_event (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id      INTEGER NOT NULL REFERENCES task_main(id) ON DELETE CASCADE,
    iteration_id INTEGER NOT NULL DEFAULT 0,
    subtask_id   INTEGER,
    kind         TEXT NOT NULL,
    content      TEXT NOT NULL,
    create_time  TEXT NOT NULL,
    agent_name   TEXT NOT NULL DEFAULT '',
    role_name    TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS human_message (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id     INTEGER NOT NULL REFERENCES task_main(id) ON DELETE CASCADE,
    content     TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'pending',
    create_time TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS chat_message (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id     INTEGER NOT NULL REFERENCES task_main(id) ON DELETE CASCADE,
    role        TEXT NOT NULL,
    content     TEXT NOT NULL,
    create_time TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_subtask_task ON subtask(task_id, sort_index);
CREATE INDEX IF NOT EXISTS idx_task_log_task ON task_log(task_id, id);
CREATE INDEX IF NOT EXISTS idx_task_event_task ON task_event(task_id, id);
CREATE INDEX IF NOT EXISTS idx_iteration_task ON iteration(task_id, id);
CREATE INDEX IF NOT EXISTS idx_human_message_task ON human_message(task_id, id);
CREATE INDEX IF NOT EXISTS idx_chat_message_task ON chat_message(task_id, id);
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

/// 内置角色定义：(key, name, description, capabilities, color, sort_order)。
pub const BUILTIN_ROLES: [(&str, &str, &str, &[&str], &str, i64); 2] = [
    (
        "reviewer",
        "评审者",
        "负责需求梳理、拆解方案、模块设计与评审验收。",
        &["requirement", "plan", "review"],
        "#9b7bff",
        0,
    ),
    (
        "coder",
        "编码者",
        "负责按方案实现代码，通过 dsh 在工作目录中写文件。",
        &["code"],
        "#5aa9ff",
        1,
    ),
];

/// V2.5：agent_config.role_type 旧表带 CHECK 约束，重建以支持任意角色 key。
fn rebuild_agent_config_if_needed(conn: &Connection) -> Result<(), String> {
    let sql: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='agent_config'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some(sql) = sql else {
        return Ok(()); // 全新库由 SCHEMA 直接创建
    };
    if !sql.contains("CHECK (role_type IN") {
        return Ok(());
    }
    conn.execute_batch(
        "BEGIN;
         ALTER TABLE agent_config RENAME TO agent_config_v24;
         CREATE TABLE agent_config (
             id            INTEGER PRIMARY KEY AUTOINCREMENT,
             name          TEXT NOT NULL,
             role_type     TEXT NOT NULL,
             provider      TEXT NOT NULL DEFAULT '',
             model_name    TEXT NOT NULL DEFAULT '',
             base_url      TEXT NOT NULL DEFAULT '',
             api_key       TEXT NOT NULL DEFAULT '',
             system_prompt TEXT NOT NULL DEFAULT '',
             temperature   REAL NOT NULL DEFAULT 0.3,
             create_time   TEXT NOT NULL
         );
         INSERT INTO agent_config(id,name,role_type,provider,model_name,base_url,api_key,system_prompt,temperature,create_time)
             SELECT id,name,role_type,provider,model_name,base_url,api_key,system_prompt,temperature,create_time
             FROM agent_config_v24;
         DROP TABLE agent_config_v24;
         COMMIT;",
    )
    .map_err(|e| format!("数据库迁移失败（agent_config 角色约束）: {e}"))?;
    Ok(())
}

/// 写入内置角色，并为历史 Agent 中出现的自定义角色 key 补登记。
fn seed_roles(conn: &Connection) -> Result<(), String> {
    for (key, name, description, caps, color, sort_order) in BUILTIN_ROLES {
        let caps_json =
            serde_json::to_string(caps).map_err(|e| format!("角色能力序列化失败: {e}"))?;
        conn.execute(
            "INSERT OR IGNORE INTO role_type(key,name,description,capabilities,color,is_builtin,sort_order,create_time)
             VALUES (?1,?2,?3,?4,?5,1,?6,?7)",
            params![key, name, description, caps_json, color, sort_order, now()],
        )
        .map_err(|e| e.to_string())?;
    }
    // 预置一个可删除的「产品经理」角色（仅需求梳理能力），开箱即可创建产品经理 Agent。
    conn.execute(
        "INSERT OR IGNORE INTO role_type(key,name,description,capabilities,color,is_builtin,sort_order,create_time)
         VALUES ('product_manager','产品经理','负责需求澄清与需求文档输出，把需求交给评审者拆解。','[\"requirement\"]','#ffc857',0,2,?1)",
        params![now()],
    )
    .map_err(|e| e.to_string())?;
    // 历史上若出现不在角色表中的 role_type，补登记，避免 Agent 变成“未知角色”。
    let unknown: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT a.role_type FROM agent_config a
                 LEFT JOIN role_type r ON r.key = a.role_type WHERE r.key IS NULL",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?
    };
    for key in unknown {
        let (name, caps): (&str, Vec<&str>) = match key.as_str() {
            "reviewer" => ("评审者", vec!["requirement", "plan", "review"]),
            "coder" => ("编码者", vec!["code"]),
            _ => (key.as_str(), vec![]),
        };
        let caps_json =
            serde_json::to_string(&caps).map_err(|e| format!("角色能力序列化失败: {e}"))?;
        conn.execute(
            "INSERT OR IGNORE INTO role_type(key,name,description,capabilities,color,is_builtin,sort_order,create_time)
             VALUES (?1,?2,'',?3,'#8b97aa',0,99,?4)",
            params![key, name, caps_json, now()],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 为旧任务补齐 workflow_agent_ids（reviewer + coders 去重保序）。
fn backfill_task_workflows(conn: &Connection) -> Result<(), String> {
    let mut stmt = conn
        .prepare("SELECT id, reviewer_agent_id, coder_agent_ids, workflow_agent_ids FROM task_main")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut updates: Vec<(i64, String)> = Vec::new();
    for row in rows {
        let (id, reviewer, coders_raw, workflow_raw) = row.map_err(|e| e.to_string())?;
        let existing: Vec<i64> = parse_json(&workflow_raw);
        if !existing.is_empty() {
            continue;
        }
        let coders: Vec<i64> = parse_json(&coders_raw);
        let mut ids: Vec<i64> = Vec::new();
        if reviewer > 0 {
            ids.push(reviewer);
        }
        for coder in coders {
            if coder > 0 && !ids.contains(&coder) {
                ids.push(coder);
            }
        }
        if ids.is_empty() {
            continue;
        }
        let json = serde_json::to_string(&ids).map_err(|e| format!("工作流序列化失败: {e}"))?;
        updates.push((id, json));
    }
    for (id, json) in updates {
        conn.execute(
            "UPDATE task_main SET workflow_agent_ids=?1 WHERE id=?2",
            params![json, id],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn migrate(conn: &Connection) -> Result<(), String> {
    // ---- V2.5：角色类型 & 线性工作流 ----
    rebuild_agent_config_if_needed(conn)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS role_type (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            key          TEXT NOT NULL UNIQUE,
            name         TEXT NOT NULL,
            description  TEXT NOT NULL DEFAULT '',
            capabilities TEXT NOT NULL DEFAULT '[]',
            color        TEXT NOT NULL DEFAULT '',
            is_builtin   INTEGER NOT NULL DEFAULT 0,
            sort_order   INTEGER NOT NULL DEFAULT 0,
            create_time  TEXT NOT NULL
        );",
    )
    .map_err(|e| e.to_string())?;
    seed_roles(conn)?;
    ensure_column(
        conn,
        "task_main",
        "workflow_agent_ids",
        "workflow_agent_ids TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        conn,
        "task_event",
        "agent_name",
        "agent_name TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        conn,
        "task_event",
        "role_name",
        "role_name TEXT NOT NULL DEFAULT ''",
    )?;
    backfill_task_workflows(conn)?;

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
    for (column, ddl) in [
        (
            "execution_mode",
            "execution_mode TEXT NOT NULL DEFAULT 'immediate'",
        ),
        (
            "schedule_start",
            "schedule_start TEXT NOT NULL DEFAULT '01:00'",
        ),
        ("schedule_end", "schedule_end TEXT NOT NULL DEFAULT '06:00'"),
        (
            "schedule_enabled",
            "schedule_enabled INTEGER NOT NULL DEFAULT 1",
        ),
        (
            "schedule_override",
            "schedule_override INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "start_requested",
            "start_requested INTEGER NOT NULL DEFAULT 0",
        ),
        ("pause_reason", "pause_reason TEXT NOT NULL DEFAULT ''"),
        (
            "followup_requirement",
            "followup_requirement TEXT NOT NULL DEFAULT ''",
        ),
        (
            "followup_mode",
            "followup_mode TEXT NOT NULL DEFAULT 'auto'",
        ),
        (
            "global_review_cycle",
            "global_review_cycle INTEGER NOT NULL DEFAULT 1",
        ),
    ] {
        ensure_column(conn, "task_main", column, ddl)?;
    }
    // ---- V2.3：任务迭代（版本） ----
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS iteration (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id      INTEGER NOT NULL REFERENCES task_main(id) ON DELETE CASCADE,
            version      TEXT NOT NULL DEFAULT '',
            major        INTEGER NOT NULL DEFAULT 0,
            minor        INTEGER NOT NULL DEFAULT 0,
            requirement  TEXT NOT NULL DEFAULT '',
            analysis     TEXT NOT NULL DEFAULT '',
            status       TEXT NOT NULL DEFAULT 'pending',
            start_time   TEXT NOT NULL DEFAULT '',
            end_time     TEXT NOT NULL DEFAULT '',
            token_total  INTEGER NOT NULL DEFAULT -1,
            create_time  TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_iteration_task ON iteration(task_id, id);",
    )
    .map_err(|e| e.to_string())?;
    ensure_column(
        conn,
        "subtask",
        "iteration_id",
        "iteration_id INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(
        conn,
        "task_log",
        "iteration_id",
        "iteration_id INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(
        conn,
        "task_event",
        "iteration_id",
        "iteration_id INTEGER NOT NULL DEFAULT 0",
    )?;
    backfill_iterations(conn)?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS chat_message (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id     INTEGER NOT NULL REFERENCES task_main(id) ON DELETE CASCADE,
            role        TEXT NOT NULL,
            content     TEXT NOT NULL,
            create_time TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_chat_message_task ON chat_message(task_id, id);",
    )
    .map_err(|e| e.to_string())?;
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
        ("storage_root", defaults.storage_root.as_str()),
        (
            "log_persistence",
            if defaults.log_persistence { "1" } else { "0" },
        ),
        (
            "default_execution_mode",
            defaults.default_execution_mode.as_str(),
        ),
        (
            "default_module_max_round",
            defaults.default_module_max_round.as_str(),
        ),
        (
            "default_global_max_round",
            defaults.default_global_max_round.as_str(),
        ),
        (
            "max_concurrent_tasks",
            defaults.max_concurrent_tasks.as_str(),
        ),
        (
            "scheduled_enabled",
            if defaults.scheduled_enabled { "1" } else { "0" },
        ),
        (
            "llm_rate_limit_per_min",
            defaults.llm_rate_limit_per_min.as_str(),
        ),
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
const AGENT_COLS: &str = "a.id,a.name,a.role_type,a.provider,a.model_name,a.base_url,a.api_key,
    a.system_prompt,a.temperature,a.create_time,
    COALESCE(r.name,'') AS role_name, COALESCE(r.color,'') AS role_color,
    COALESCE(r.capabilities,'[]') AS role_capabilities";

fn row_to_agent(row: &Row<'_>) -> rusqlite::Result<AgentConfig> {
    let caps_raw: String = row.get("role_capabilities").unwrap_or_default();
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
        role_name: row.get("role_name").unwrap_or_default(),
        role_color: row.get("role_color").unwrap_or_default(),
        role_capabilities: parse_json(&caps_raw),
    })
}

pub fn list_agents(conn: &Connection) -> Result<Vec<AgentConfig>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {AGENT_COLS} FROM agent_config a
             LEFT JOIN role_type r ON r.key = a.role_type
             ORDER BY COALESCE(r.sort_order, 99), a.id"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], row_to_agent)
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

pub fn get_agent(conn: &Connection, id: i64) -> Result<Option<AgentConfig>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {AGENT_COLS} FROM agent_config a
             LEFT JOIN role_type r ON r.key = a.role_type WHERE a.id = ?1"
        ))
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
    if input.role_type.trim().is_empty() {
        return Err("请选择角色类型".into());
    }
    if get_role_by_key(conn, input.role_type.trim())?.is_none() {
        return Err(format!("角色「{}」不存在", input.role_type));
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
        .prepare(
            "SELECT reviewer_agent_id, coder_agent_ids, workflow_agent_ids FROM task_main",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            let reviewer: i64 = row.get(0)?;
            let coders: String = row.get(1)?;
            let workflow: String = row.get(2).unwrap_or_default();
            Ok((reviewer, coders, workflow))
        })
        .map_err(|e| e.to_string())?;
    for row in rows {
        let (reviewer, coders_raw, workflow_raw) = row.map_err(|e| e.to_string())?;
        let coders: Vec<i64> = parse_json(&coders_raw);
        let workflow: Vec<i64> = parse_json(&workflow_raw);
        if reviewer == id || coders.contains(&id) || workflow.contains(&id) {
            return Err("该 Agent 已被任务引用，无法删除".into());
        }
    }
    conn.execute("DELETE FROM agent_config WHERE id=?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ------------------------------------------------------------------
// 角色类型（V2.5）
// ------------------------------------------------------------------
const ROLE_COLS: &str = "r.id,r.key,r.name,r.description,r.capabilities,r.color,
    r.is_builtin,r.sort_order,r.create_time";

fn row_to_role(row: &Row<'_>) -> rusqlite::Result<RoleType> {
    let caps_raw: String = row.get("capabilities").unwrap_or_default();
    Ok(RoleType {
        id: row.get("id")?,
        key: row.get("key")?,
        name: row.get("name")?,
        description: row.get("description").unwrap_or_default(),
        capabilities: parse_json(&caps_raw),
        color: row.get("color").unwrap_or_default(),
        is_builtin: row.get::<_, i64>("is_builtin").unwrap_or(0) != 0,
        sort_order: row.get::<_, i64>("sort_order").unwrap_or(0),
        create_time: row.get("create_time")?,
        agent_count: 0,
    })
}

pub fn list_roles(conn: &Connection) -> Result<Vec<RoleType>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {ROLE_COLS}, (SELECT COUNT(*) FROM agent_config a WHERE a.role_type=r.key) AS agent_count
             FROM role_type r ORDER BY r.is_builtin DESC, r.sort_order, r.id"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            let mut role = row_to_role(row)?;
            role.agent_count = row.get("agent_count").unwrap_or(0);
            Ok(role)
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

pub fn get_role(conn: &Connection, id: i64) -> Result<Option<RoleType>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {ROLE_COLS}, (SELECT COUNT(*) FROM agent_config a WHERE a.role_type=r.key) AS agent_count
             FROM role_type r WHERE r.id=?1"
        ))
        .map_err(|e| e.to_string())?;
    let mut rows = stmt.query(params![id]).map_err(|e| e.to_string())?;
    match rows.next().map_err(|e| e.to_string())? {
        Some(row) => {
            let mut role = row_to_role(row).map_err(|e| e.to_string())?;
            role.agent_count = row.get("agent_count").unwrap_or(0);
            Ok(Some(role))
        }
        None => Ok(None),
    }
}

pub fn get_role_by_key(conn: &Connection, key: &str) -> Result<Option<RoleType>, String> {
    let id: Option<i64> = conn
        .query_row(
            "SELECT id FROM role_type WHERE key=?1",
            params![key.trim()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    match id {
        Some(id) => get_role(conn, id),
        None => Ok(None),
    }
}

fn role_key_from_name(name: &str, seq: i64) -> String {
    let mut key: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c
            } else {
                '_'
            }
        })
        .collect();
    key = key.trim_matches('_').to_string();
    if key.is_empty() || !key.chars().next().unwrap().is_ascii_alphabetic() {
        key = format!("role_{seq}");
    }
    key
}

fn validate_capabilities(caps: &[String]) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for cap in caps {
        let cap = cap.trim();
        if !matches!(
            cap,
            crate::models::CAP_REQUIREMENT
                | crate::models::CAP_PLAN
                | crate::models::CAP_CODE
                | crate::models::CAP_REVIEW
        ) {
            return Err(format!("未知能力标签：{cap}"));
        }
        if !out.iter().any(|c| c == cap) {
            out.push(cap.to_string());
        }
    }
    if out.is_empty() {
        return Err("角色至少需要 1 个能力标签".into());
    }
    Ok(out)
}

pub fn save_role(conn: &Connection, input: &RoleInput) -> Result<RoleType, String> {
    if input.name.trim().is_empty() {
        return Err("角色名称不能为空".into());
    }
    let capabilities = validate_capabilities(&input.capabilities)?;
    let caps_json =
        serde_json::to_string(&capabilities).map_err(|e| format!("角色能力序列化失败: {e}"))?;
    let color = input.color.trim();
    if let Some(id) = input.id {
        let current = get_role(conn, id)?.ok_or_else(|| "角色不存在".to_string())?;
        if current.is_builtin {
            // 内置角色只允许补充说明，能力与标识不可改。
            conn.execute(
                "UPDATE role_type SET description=?1 WHERE id=?2",
                params![input.description.trim(), id],
            )
            .map_err(|e| e.to_string())?;
        } else {
            conn.execute(
                "UPDATE role_type SET name=?1, description=?2, capabilities=?3, color=?4 WHERE id=?5",
                params![
                    input.name.trim(),
                    input.description.trim(),
                    caps_json,
                    color,
                    id
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        get_role(conn, id)?.ok_or_else(|| "角色不存在".to_string())
    } else {
        let key = if input.key.trim().is_empty() {
            let seq: i64 = conn
                .query_row("SELECT COALESCE(MAX(id),0)+1 FROM role_type", [], |row| row.get(0))
                .map_err(|e| e.to_string())?;
            role_key_from_name(&input.name, seq)
        } else {
            input.key.trim().to_lowercase()
        };
        if key.len() < 2 || key.len() > 32 {
            return Err("角色标识长度必须是 2-32 个字符".into());
        }
        if !key
            .chars()
            .enumerate()
            .all(|(i, c)| c.is_ascii_lowercase() || c.is_ascii_digit() || (c == '_' && i > 0))
        {
            return Err("角色标识只能包含小写字母、数字和下划线，且以字母开头".into());
        }
        if get_role_by_key(conn, &key)?.is_some() {
            return Err(format!("角色标识「{key}」已存在"));
        }
        let sort_order: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(sort_order),0)+1 FROM role_type",
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO role_type(key,name,description,capabilities,color,is_builtin,sort_order,create_time)
             VALUES (?1,?2,?3,?4,?5,0,?6,?7)",
            params![
                key,
                input.name.trim(),
                input.description.trim(),
                caps_json,
                color,
                sort_order,
                now()
            ],
        )
        .map_err(|e| e.to_string())?;
        get_role(conn, conn.last_insert_rowid())?.ok_or_else(|| "角色创建失败".to_string())
    }
}

pub fn delete_role(conn: &Connection, id: i64) -> Result<(), String> {
    let role = get_role(conn, id)?.ok_or_else(|| "角色不存在".to_string())?;
    if role.is_builtin {
        return Err(format!("「{}」是内置角色，不能删除", role.name));
    }
    if role.agent_count > 0 {
        return Err(format!(
            "仍有 {} 个 Agent 使用「{}」，请先改为其它角色",
            role.agent_count, role.name
        ));
    }
    conn.execute("DELETE FROM role_type WHERE id=?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 角色能力标签（role 表缺失时按内置角色兜底）。
pub fn role_capabilities(conn: &Connection, key: &str) -> Vec<String> {
    if let Ok(Some(role)) = get_role_by_key(conn, key) {
        return role.capabilities;
    }
    match key {
        "reviewer" => vec![
            crate::models::CAP_REQUIREMENT.into(),
            crate::models::CAP_PLAN.into(),
            crate::models::CAP_REVIEW.into(),
        ],
        "coder" => vec![crate::models::CAP_CODE.into()],
        _ => Vec::new(),
    }
}

/// 任务实际生效的工作流 Agent id 列表（旧任务回退到 reviewer + coders）。
pub fn task_workflow_ids(task: &TaskMain) -> Vec<i64> {
    if !task.workflow_agent_ids.is_empty() {
        return task.workflow_agent_ids.clone();
    }
    let mut ids: Vec<i64> = Vec::new();
    if task.reviewer_agent_id > 0 {
        ids.push(task.reviewer_agent_id);
    }
    for id in &task.coder_agent_ids {
        if !ids.contains(id) {
            ids.push(*id);
        }
    }
    ids
}

/// 工作流展开为带角色信息的步骤视图。
pub fn workflow_steps(conn: &Connection, task: &TaskMain) -> Vec<WorkflowStep> {
    let mut out = Vec::new();
    for (idx, agent_id) in task_workflow_ids(task).into_iter().enumerate() {
        let Ok(Some(agent)) = get_agent(conn, agent_id) else {
            continue;
        };
        out.push(WorkflowStep {
            order: (idx + 1) as i64,
            agent_id: agent.id,
            agent_name: agent.name.clone(),
            role_key: agent.role_type.clone(),
            role_name: if agent.role_name.is_empty() {
                agent.role_type.clone()
            } else {
                agent.role_name.clone()
            },
            role_color: agent.role_color.clone(),
            capabilities: agent.role_capabilities.clone(),
        });
    }
    out
}

/// 校验工作流：2-6 步、不重复、Agent 存在、必须覆盖 plan / code / review 三种能力。
pub fn validate_workflow(conn: &Connection, ids: &[i64]) -> Result<(), String> {
    if ids.len() < 2 {
        return Err("工作流至少需要 2 个 Agent（例如 评审者 → 编码者）".into());
    }
    if ids.len() > 6 {
        return Err("工作流最多支持 6 个 Agent".into());
    }
    let mut seen: Vec<i64> = Vec::new();
    let mut caps: Vec<String> = Vec::new();
    for id in ids {
        if seen.contains(id) {
            return Err(format!("Agent #{id} 在工作流中重复出现"));
        }
        seen.push(*id);
        let agent = get_agent(conn, *id)?
            .ok_or_else(|| format!("工作流中的 Agent #{id} 不存在"))?;
        for cap in &agent.role_capabilities {
            if !caps.contains(cap) {
                caps.push(cap.clone());
            }
        }
    }
    for cap in [
        crate::models::CAP_PLAN,
        crate::models::CAP_CODE,
        crate::models::CAP_REVIEW,
    ] {
        if !caps.iter().any(|c| c == cap) {
            return Err(format!(
                "工作流缺少「{}」能力的 Agent",
                crate::models::capability_label(cap)
            ));
        }
    }
    Ok(())
}

/// 按能力在工作流中解析第一个匹配的 Agent。
pub fn resolve_task_agent(
    conn: &Connection,
    task: &TaskMain,
    capability: &str,
) -> Result<AgentConfig, String> {
    for id in task_workflow_ids(task) {
        if let Some(agent) = get_agent(conn, id)? {
            if agent.role_capabilities.iter().any(|c| c == capability) {
                return Ok(agent);
            }
        }
    }
    Err(format!(
        "任务工作流中没有具备「{}」能力的 Agent",
        crate::models::capability_label(capability)
    ))
}

/// 按能力解析工作流中所有匹配的 Agent（编码阶段用于轮换分配）。
pub fn resolve_task_agents(
    conn: &Connection,
    task: &TaskMain,
    capability: &str,
) -> Result<Vec<AgentConfig>, String> {
    let mut out = Vec::new();
    for id in task_workflow_ids(task) {
        if let Some(agent) = get_agent(conn, id)? {
            if agent.role_capabilities.iter().any(|c| c == capability) {
                out.push(agent);
            }
        }
    }
    if out.is_empty() {
        return Err(format!(
            "任务工作流中没有具备「{}」能力的 Agent",
            crate::models::capability_label(capability)
        ));
    }
    Ok(out)
}

// ------------------------------------------------------------------
// Task
// ------------------------------------------------------------------
fn row_to_task(row: &Row<'_>) -> rusqlite::Result<TaskMain> {
    let coders_raw: String = row.get("coder_agent_ids")?;
    let workflow_raw: String = row.get("workflow_agent_ids").unwrap_or_default();
    let records_raw: String = row.get("global_review_records")?;
    Ok(TaskMain {
        id: row.get("id")?,
        task_name: row.get("task_name")?,
        top_requirement: row.get("top_requirement")?,
        global_accept_criteria: row.get("global_accept_criteria")?,
        reviewer_agent_id: row.get("reviewer_agent_id")?,
        coder_agent_ids: parse_json(&coders_raw),
        workflow_agent_ids: parse_json(&workflow_raw),
        module_max_round: row.get("module_max_round")?,
        global_max_round: row.get("global_max_round")?,
        global_contract: row.get("global_contract")?,
        task_phase: row.get("task_phase")?,
        status: row.get("status")?,
        work_dir: row.get("work_dir")?,
        current_global_round: row.get("current_global_round")?,
        global_review_cycle: row.get::<_, i64>("global_review_cycle").unwrap_or(1).max(1),
        global_review_records: parse_json(&records_raw),
        create_time: row.get("create_time")?,
        baseline_tree: row.get("baseline_tree").unwrap_or_default(),
        manual_review_enabled: row.get::<_, i64>("manual_review_enabled").unwrap_or(0) != 0,
        tech_stack: row.get("tech_stack").unwrap_or_default(),
        pending_plan_json: row.get("pending_plan_json").unwrap_or_default(),
        cancel_requested: row.get::<_, i64>("cancel_requested").unwrap_or(0) != 0,
        execution_mode: row
            .get::<_, String>("execution_mode")
            .unwrap_or_else(|_| "later".into()),
        schedule_start: row
            .get::<_, String>("schedule_start")
            .unwrap_or_else(|_| "01:00".into()),
        schedule_end: row
            .get::<_, String>("schedule_end")
            .unwrap_or_else(|_| "06:00".into()),
        schedule_enabled: row.get::<_, i64>("schedule_enabled").unwrap_or(0) != 0,
        schedule_override: row.get::<_, i64>("schedule_override").unwrap_or(0) != 0,
        start_requested: row.get::<_, i64>("start_requested").unwrap_or(0) != 0,
        pause_reason: row.get("pause_reason").unwrap_or_default(),
        followup_requirement: row.get("followup_requirement").unwrap_or_default(),
        followup_mode: row
            .get::<_, String>("followup_mode")
            .unwrap_or_else(|_| "auto".into()),
    })
}

const TASK_COLS: &str = "id,task_name,top_requirement,global_accept_criteria,reviewer_agent_id,
    coder_agent_ids,workflow_agent_ids,module_max_round,global_max_round,global_contract,task_phase,status,work_dir,
    current_global_round,global_review_records,baseline_tree,manual_review_enabled,tech_stack,
    pending_plan_json,cancel_requested,execution_mode,schedule_start,schedule_end,schedule_enabled,
    schedule_override,start_requested,pause_reason,followup_requirement,followup_mode,
    global_review_cycle,create_time";

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
        let workflow_ids = task_workflow_ids(&task);
        let workflow: Vec<WorkflowStep> = workflow_ids
            .iter()
            .enumerate()
            .filter_map(|(idx, id)| {
                agents.iter().find(|a| a.id == *id).map(|a| WorkflowStep {
                    order: (idx + 1) as i64,
                    agent_id: a.id,
                    agent_name: a.name.clone(),
                    role_key: a.role_type.clone(),
                    role_name: if a.role_name.is_empty() {
                        a.role_type.clone()
                    } else {
                        a.role_name.clone()
                    },
                    role_color: a.role_color.clone(),
                    capabilities: a.role_capabilities.clone(),
                })
            })
            .collect();
        let reviewer_name = workflow
            .iter()
            .find(|w| w.capabilities.iter().any(|c| c == "plan"))
            .or_else(|| {
                workflow
                    .iter()
                    .find(|w| w.capabilities.iter().any(|c| c == "review"))
            })
            .or_else(|| workflow.first())
            .map(|w| w.agent_name.clone())
            .unwrap_or_else(|| "（已删除）".into());
        let coder_names = workflow
            .iter()
            .filter(|w| w.capabilities.iter().any(|c| c == "code"))
            .map(|w| w.agent_name.clone())
            .collect::<Vec<_>>();
        let (subtask_count, completed_count) = subtask_counts(conn, task.id)?;
        let version = conn
            .query_row(
                "SELECT version FROM iteration WHERE task_id=?1 AND version<>''
                 ORDER BY major DESC, minor DESC, id DESC LIMIT 1",
                params![task.id],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        out.push(TaskSummary {
            task,
            reviewer_name,
            coder_names,
            workflow,
            subtask_count,
            completed_count,
            version,
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

/// 依据工作流推导旧的 reviewer_agent_id / coder_agent_ids（回滚兼容）。
fn derive_legacy_bindings(
    conn: &Connection,
    workflow: &[i64],
) -> Result<(i64, Vec<i64>), String> {
    let mut reviewer = 0i64;
    let mut coders: Vec<i64> = Vec::new();
    let mut review_fallback = 0i64;
    for id in workflow {
        if let Some(agent) = get_agent(conn, *id)? {
            let caps = &agent.role_capabilities;
            if reviewer == 0 && caps.iter().any(|c| c == crate::models::CAP_PLAN) {
                reviewer = *id;
            }
            if review_fallback == 0 && caps.iter().any(|c| c == crate::models::CAP_REVIEW) {
                review_fallback = *id;
            }
            if caps.iter().any(|c| c == crate::models::CAP_CODE) {
                coders.push(*id);
            }
        }
    }
    if reviewer == 0 {
        reviewer = review_fallback;
    }
    if reviewer == 0 {
        reviewer = *workflow.first().ok_or_else(|| "工作流不能为空".to_string())?;
    }
    Ok((reviewer, coders))
}

/// 持久化任务工作流，并同步旧的 reviewer / coder 字段。
pub fn save_task_workflow(
    conn: &Connection,
    task_id: i64,
    workflow: &[i64],
) -> Result<(), String> {
    validate_workflow(conn, workflow)?;
    let (reviewer, coders) = derive_legacy_bindings(conn, workflow)?;
    let workflow_json = serde_json::to_string(workflow)
        .map_err(|e| format!("工作流序列化失败: {e}"))?;
    let coders_json = serde_json::to_string(&coders)
        .map_err(|e| format!("coder 列表序列化失败: {e}"))?;
    conn.execute(
        "UPDATE task_main SET workflow_agent_ids=?1, reviewer_agent_id=?2, coder_agent_ids=?3 WHERE id=?4",
        params![workflow_json, reviewer, coders_json, task_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
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
    // 新前端只传 workflow_agent_ids；旧调用方仍可只传 reviewer + coders。
    let workflow: Vec<i64> = if !input.workflow_agent_ids.is_empty() {
        input.workflow_agent_ids.clone()
    } else {
        let mut ids: Vec<i64> = Vec::new();
        if input.reviewer_agent_id > 0 {
            ids.push(input.reviewer_agent_id);
        }
        for id in &input.coder_agent_ids {
            if !ids.contains(id) {
                ids.push(*id);
            }
        }
        ids
    };
    validate_workflow(conn, &workflow)?;
    let (reviewer_id, coder_ids) = derive_legacy_bindings(conn, &workflow)?;
    if input.module_max_round < 1 || input.global_max_round < 1 {
        return Err("轮次上限必须大于等于 1".into());
    }
    let mode = input.execution_mode.trim();
    if !matches!(mode, "immediate" | "later" | "scheduled") {
        return Err("执行模式必须是 immediate / later / scheduled".into());
    }
    if mode == "scheduled" {
        if !valid_clock(&input.schedule_start) || !valid_clock(&input.schedule_end) {
            return Err("定时窗口时间格式必须是 HH:MM".into());
        }
        if input.schedule_start.trim() == input.schedule_end.trim() {
            return Err("定时窗口开始时间和结束时间不能相同".into());
        }
    }
    let coders_json = serde_json::to_string(&coder_ids)
        .map_err(|e| format!("coder 列表序列化失败: {e}"))?;
    let workflow_json = serde_json::to_string(&workflow)
        .map_err(|e| format!("工作流序列化失败: {e}"))?;
    let schedule_enabled = mode == "scheduled" && input.schedule_enabled;
    conn.execute(
        "INSERT INTO task_main(task_name,top_requirement,global_accept_criteria,reviewer_agent_id,
            coder_agent_ids,workflow_agent_ids,module_max_round,global_max_round,global_contract,
            task_phase,status,work_dir,current_global_round,global_review_records,manual_review_enabled,
            execution_mode,schedule_start,schedule_end,schedule_enabled,schedule_override,
            start_requested,pause_reason,followup_requirement,followup_mode,global_review_cycle,
            create_time)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'','module_dev','pending',?9,0,'[]',?10,
            ?11,?12,?13,?14,0,?15,'','','auto',1,?16)",
        params![
            input.task_name.trim(),
            input.top_requirement.trim(),
            input.global_accept_criteria.trim(),
            reviewer_id,
            coders_json,
            workflow_json,
            input.module_max_round,
            input.global_max_round,
            input.work_dir.trim(),
            if input.manual_review_enabled { 1 } else { 0 },
            mode,
            input.schedule_start.trim(),
            input.schedule_end.trim(),
            if schedule_enabled { 1 } else { 0 },
            if mode == "immediate" { 1 } else { 0 },
            now()
        ],
    )
    .map_err(|e| e.to_string())?;
    let task_id = conn.last_insert_rowid();
    // 首次创建任务即登记 dev-1.0，后续追加需求在此基础上递增版本号。
    let iteration_id = create_iteration(conn, task_id, input.top_requirement.trim())?;
    settle_iteration(conn, iteration_id, "dev-1.0", 1, 0)?;
    Ok(task_id)
}

/// 只持久化调度过程中会变化的字段。
pub fn save_task(conn: &Connection, task: &TaskMain) -> Result<(), String> {
    let records = serde_json::to_string(&task.global_review_records)
        .map_err(|e| format!("全局评审记录序列化失败: {e}"))?;
    conn.execute(
        "UPDATE task_main SET global_contract=?1, task_phase=?2, status=?3,
            current_global_round=?4, global_review_records=?5, baseline_tree=?6,
            tech_stack=?7, pending_plan_json=?8, execution_mode=?9, schedule_start=?10,
            schedule_end=?11, schedule_enabled=?12, schedule_override=?13,
            start_requested=?14, pause_reason=?15, followup_requirement=?16,
            followup_mode=?17, global_review_cycle=?18 WHERE id=?19",
        params![
            task.global_contract,
            task.task_phase,
            task.status,
            task.current_global_round,
            records,
            task.baseline_tree,
            task.tech_stack,
            task.pending_plan_json,
            task.execution_mode,
            task.schedule_start,
            task.schedule_end,
            if task.schedule_enabled { 1 } else { 0 },
            if task.schedule_override { 1 } else { 0 },
            if task.start_requested { 1 } else { 0 },
            task.pause_reason,
            task.followup_requirement,
            task.followup_mode,
            task.global_review_cycle.max(1),
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
// Iteration（任务迭代 / 版本）
// ------------------------------------------------------------------
/// 单次追加需求拆出的子任务数超过该阈值时，版本号升大版本。
pub const COMPLEX_SUBTASK_THRESHOLD: i64 = 5;

fn parse_local(s: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(s.trim(), "%Y-%m-%d %H:%M:%S").ok()
}

/// 迭代耗时（秒）：已有开始时间即返回；未结束时按当前时间计算。
pub fn duration_sec(start_time: &str, end_time: &str) -> i64 {
    if start_time.trim().is_empty() {
        return 0;
    }
    let Some(start) = parse_local(start_time) else {
        return 0;
    };
    let end = if end_time.trim().is_empty() {
        Local::now().naive_local()
    } else {
        match parse_local(end_time) {
            Some(v) => v,
            None => return 0,
        }
    };
    (end - start).num_seconds().max(0)
}

fn row_to_iteration(row: &Row<'_>) -> rusqlite::Result<Iteration> {
    let start_time: String = row.get("start_time")?;
    let end_time: String = row.get("end_time")?;
    Ok(Iteration {
        id: row.get("id")?,
        task_id: row.get("task_id")?,
        version: row.get("version")?,
        major: row.get("major")?,
        minor: row.get("minor")?,
        requirement: row.get("requirement")?,
        analysis: row.get("analysis")?,
        status: row.get("status")?,
        duration_sec: duration_sec(&start_time, &end_time),
        start_time,
        end_time,
        token_total: row.get("token_total")?,
        subtask_total: 0,
        subtask_completed: 0,
        create_time: row.get("create_time")?,
    })
}

const ITERATION_COLS: &str = "id,task_id,version,major,minor,requirement,analysis,status,
    start_time,end_time,token_total,create_time";

/// 新建一个「待定」迭代（需求梳理完成前 version 为空，之后由 settle 落定）。
pub fn create_iteration(
    conn: &Connection,
    task_id: i64,
    requirement: &str,
) -> Result<i64, String> {
    let create_time = now();
    conn.execute(
        "INSERT INTO iteration(task_id,version,major,minor,requirement,analysis,status,
            start_time,end_time,token_total,create_time)
         VALUES (?1,'',0,0,?2,'','pending','','',0,?3)",
        params![task_id, requirement.trim(), create_time],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

/// 把一次 LLM 调用的 token 用量累计到当前迭代。
/// `token_total = -1` 表示该迭代从未统计过（旧版本数据），首次累加会从实际值开始。
pub fn add_iteration_tokens(conn: &Connection, task_id: i64, tokens: i64) -> Result<(), String> {
    if tokens <= 0 {
        return Ok(());
    }
    let id = current_iteration_id(conn, task_id);
    if id == 0 {
        return Ok(());
    }
    conn.execute(
        "UPDATE iteration SET token_total = CASE WHEN token_total < 0 THEN ?1 ELSE token_total + ?1 END
         WHERE id=?2",
        params![tokens, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 落定版本号。
pub fn settle_iteration(
    conn: &Connection,
    iteration_id: i64,
    version: &str,
    major: i64,
    minor: i64,
) -> Result<(), String> {
    conn.execute(
        "UPDATE iteration SET version=?1, major=?2, minor=?3 WHERE id=?4",
        params![version, major, minor, iteration_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_iteration_analysis(conn: &Connection, iteration_id: i64, analysis: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE iteration SET analysis=?1 WHERE id=?2",
        params![analysis.trim(), iteration_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 计算下一个版本号：默认小版本 +1，子任务数超过阈值时大版本 +1 且小版本归零。
pub fn next_iteration_version(
    conn: &Connection,
    task_id: i64,
    subtask_count: i64,
) -> Result<(String, i64, i64), String> {
    let last: Option<(i64, i64)> = conn
        .query_row(
            "SELECT major,minor FROM iteration
             WHERE task_id=?1 AND version<>''
             ORDER BY major DESC, minor DESC, id DESC LIMIT 1",
            params![task_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let (major, minor) = match last {
        None => (1, 0),
        Some((major, minor)) => {
            if subtask_count > COMPLEX_SUBTASK_THRESHOLD {
                (major + 1, 0)
            } else {
                (major, minor + 1)
            }
        }
    };
    Ok((format!("dev-{major}.{minor}"), major, minor))
}

pub fn list_iterations(conn: &Connection, task_id: i64) -> Result<Vec<Iteration>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {ITERATION_COLS},
                (SELECT COUNT(*) FROM subtask s WHERE s.iteration_id=iteration.id) AS subtask_total,
                (SELECT COUNT(*) FROM subtask s WHERE s.iteration_id=iteration.id AND s.status='completed') AS subtask_completed
             FROM iteration WHERE task_id=?1 ORDER BY major, minor, id"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![task_id], |row| {
            let mut it = row_to_iteration(row)?;
            it.subtask_total = row.get("subtask_total")?;
            it.subtask_completed = row.get("subtask_completed")?;
            Ok(it)
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

pub fn get_iteration(conn: &Connection, iteration_id: i64) -> Result<Option<Iteration>, String> {
    let all: Option<Iteration> = conn
        .query_row(
            &format!("SELECT {ITERATION_COLS} FROM iteration WHERE id=?1"),
            params![iteration_id],
            row_to_iteration,
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(all)
}

/// 当前迭代 = 最新创建的那个（含尚未落定版本号的待定迭代）。
pub fn current_iteration_id(conn: &Connection, task_id: i64) -> i64 {
    conn.query_row(
        "SELECT id FROM iteration WHERE task_id=?1 ORDER BY id DESC LIMIT 1",
        params![task_id],
        |r| r.get::<_, i64>(0),
    )
    .optional()
    .ok()
    .flatten()
    .unwrap_or(0)
}

/// 标记当前迭代开始计时（重复调用不覆盖已有开始时间）。
pub fn mark_iteration_started(conn: &Connection, task_id: i64) -> Result<(), String> {
    let id = current_iteration_id(conn, task_id);
    if id == 0 {
        return Ok(());
    }
    conn.execute(
        "UPDATE iteration SET start_time=CASE WHEN start_time='' THEN ?1 ELSE start_time END,
            status=CASE WHEN status IN ('completed','terminated','failed') THEN status ELSE 'running' END
         WHERE id=?2",
        params![now(), id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 重新打开当前迭代（例如继续已终止任务时）。
pub fn reopen_current_iteration(conn: &Connection, task_id: i64) -> Result<(), String> {
    let id = current_iteration_id(conn, task_id);
    if id == 0 {
        return Ok(());
    }
    conn.execute(
        "UPDATE iteration SET status='running', end_time='' WHERE id=?1",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 标记当前迭代结束（写入结束时间与最终状态）。
pub fn mark_iteration_finished(conn: &Connection, task_id: i64, status: &str) -> Result<(), String> {
    let id = current_iteration_id(conn, task_id);
    if id == 0 {
        return Ok(());
    }
    conn.execute(
        "UPDATE iteration SET end_time=CASE WHEN end_time='' THEN ?1 ELSE end_time END, status=?2
         WHERE id=?3",
        params![now(), status, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 把尚未归档的子任务/日志/事件挂到指定迭代。
pub fn assign_iteration_records(
    conn: &Connection,
    task_id: i64,
    iteration_id: i64,
) -> Result<(), String> {
    conn.execute(
        "UPDATE subtask SET iteration_id=?1 WHERE task_id=?2 AND iteration_id=0",
        params![iteration_id, task_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE task_log SET iteration_id=?1 WHERE task_id=?2 AND iteration_id=0",
        params![iteration_id, task_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE task_event SET iteration_id=?1 WHERE task_id=?2 AND iteration_id=0",
        params![iteration_id, task_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 旧库升级：为没有迭代数据的任务补建 dev-1.0，并把既有子任务/日志/事件归档进去。
fn backfill_iterations(conn: &Connection) -> Result<(), String> {
    let tasks: Vec<(i64, String, String, String)> = {
        let mut stmt = conn
            .prepare("SELECT id,top_requirement,status,create_time FROM task_main")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?
    };
    for (task_id, requirement, status, create_time) in tasks {
        let existing: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM iteration WHERE task_id=?1",
                params![task_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if existing > 0 {
            // 已有迭代：仍把游离记录挂回第一个迭代，保证不出现无归属数据。
            let first: i64 = conn
                .query_row(
                    "SELECT id FROM iteration WHERE task_id=?1 ORDER BY id LIMIT 1",
                    params![task_id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            assign_iteration_records(conn, task_id, first)?;
            continue;
        }
        let iter_status = match status.as_str() {
            "completed" => "completed",
            "terminated" => "terminated",
            "failed" => "failed",
            "pending" | "planning" => "pending",
            _ => "running",
        };
        let end_time = if iter_status == "completed" {
            legacy_end_time(conn, task_id).unwrap_or_else(|| create_time.clone())
        } else {
            String::new()
        };
        conn.execute(
            "INSERT INTO iteration(task_id,version,major,minor,requirement,analysis,status,
                start_time,end_time,token_total,create_time)
             VALUES (?1,'dev-1.0',1,0,?2,'',?3,?4,?5,-1,?6)",
            params![
                task_id,
                requirement,
                iter_status,
                create_time,
                end_time,
                create_time
            ],
        )
        .map_err(|e| e.to_string())?;
        let iteration_id = conn.last_insert_rowid();
        assign_iteration_records(conn, task_id, iteration_id)?;
    }
    Ok(())
}

fn legacy_end_time(conn: &Connection, task_id: i64) -> Option<String> {
    let from_events: Option<String> = conn
        .query_row(
            "SELECT MAX(create_time) FROM task_event WHERE task_id=?1",
            params![task_id],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten();
    from_events.or_else(|| {
        conn.query_row(
            "SELECT MAX(create_time) FROM task_log WHERE task_id=?1",
            params![task_id],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten()
    })
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
        iteration_id: row.get("iteration_id").unwrap_or_default(),
    })
}

const SUBTASK_COLS: &str = "id,task_id,subtask_name,sort_index,accept_criteria,assigned_coder_id,
    current_round,max_round,status,current_stage,summary_content,summary_file_path,iteration_records,
    design_content,iteration_id";

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
    insert_subtasks_from(conn, task_id, items, coder_id, max_round, 1)
}

fn insert_subtasks_from(
    conn: &mut Connection,
    task_id: i64,
    items: &[(String, String)],
    coder_id: i64,
    max_round: i64,
    start_index: i64,
) -> Result<Vec<Subtask>, String> {
    let iteration_id = current_iteration_id(conn, task_id);
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for (i, (name, criteria)) in items.iter().enumerate() {
        tx.execute(
            "INSERT INTO subtask(task_id,subtask_name,sort_index,accept_criteria,assigned_coder_id,
                current_round,max_round,status,current_stage,summary_content,summary_file_path,iteration_records,design_content,iteration_id)
             VALUES (?1,?2,?3,?4,?5,0,?6,'waiting','queued','','','[]','',?7)",
            params![
                task_id,
                name.trim(),
                start_index + i as i64,
                criteria,
                coder_id,
                max_round,
                iteration_id
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
        "UPDATE subtask SET assigned_coder_id=?1, current_round=?2, max_round=?3, status=?4,
            current_stage=?5, summary_content=?6, summary_file_path=?7, iteration_records=?8,
            design_content=?9, iteration_id=?10 WHERE id=?11",
        params![
            subtask.assigned_coder_id,
            subtask.current_round,
            subtask.max_round,
            subtask.status,
            subtask.current_stage,
            subtask.summary_content,
            subtask.summary_file_path,
            records,
            subtask.design_content,
            subtask.iteration_id,
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
        iteration_id: row.get("iteration_id").unwrap_or_default(),
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
    if !log_persistence() {
        return Ok(TaskLog {
            id: synthetic_id(),
            task_id,
            iteration_id: current_iteration_id(conn, task_id),
            level: level.to_string(),
            message: message.to_string(),
            create_time,
        });
    }
    let iteration_id = current_iteration_id(conn, task_id);
    conn.execute(
        "INSERT INTO task_log(task_id,iteration_id,level,message,create_time)
         VALUES (?1,?2,?3,?4,?5)",
        params![task_id, iteration_id, level, message, create_time],
    )
    .map_err(|e| e.to_string())?;
    Ok(TaskLog {
        id: conn.last_insert_rowid(),
        task_id,
        iteration_id,
        level: level.to_string(),
        message: message.to_string(),
        create_time,
    })
}

pub fn list_logs(conn: &Connection, task_id: i64) -> Result<Vec<TaskLog>, String> {
    // 只取最近 800 条，避免超长任务把页面拖慢。
    let mut stmt = conn
        .prepare(
            "SELECT id,task_id,iteration_id,level,message,create_time FROM (
                SELECT id,task_id,iteration_id,level,message,create_time FROM task_log
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
    let workflow = workflow_steps(conn, &task);
    let reviewer = workflow
        .iter()
        .find(|w| w.capabilities.iter().any(|c| c == "plan"))
        .or_else(|| workflow.iter().find(|w| w.capabilities.iter().any(|c| c == "review")))
        .and_then(|w| get_agent(conn, w.agent_id).ok().flatten());
    let mut coders = Vec::new();
    for step in workflow
        .iter()
        .filter(|w| w.capabilities.iter().any(|c| c == "code"))
    {
        if let Some(a) = get_agent(conn, step.agent_id)? {
            coders.push(a);
        }
    }
    let roles = list_roles(conn)?;
    let execution_events = list_events(conn, task_id, 2000)?;
    let human_messages = list_human_messages(conn, task_id)?;
    let pending_plan = parse_json::<PendingPlan>(&task.pending_plan_json);
    let pending_plan = if task.pending_plan_json.trim().is_empty() {
        None
    } else {
        Some(pending_plan)
    };
    let chat_messages = list_chat_messages(conn, task_id, 300)?;
    let iterations = list_iterations(conn, task_id)?;
    Ok(TaskDetail {
        task,
        subtasks,
        logs,
        reviewer,
        coders,
        workflow,
        roles,
        execution_events,
        human_messages,
        pending_plan,
        chat_messages,
        iterations,
    })
}

/// 单个迭代的详情：只看该迭代范围内的子任务 / 事件 / 日志。
pub fn get_iteration_detail(
    conn: &Connection,
    task_id: i64,
    iteration_id: i64,
) -> Result<crate::models::IterationDetail, String> {
    let task = get_task(conn, task_id)?.ok_or_else(|| "任务不存在".to_string())?;
    let iteration = get_iteration(conn, iteration_id)?
        .filter(|i| i.task_id == task_id)
        .ok_or_else(|| "迭代不存在或不属于该任务".to_string())?;
    let workflow = workflow_steps(conn, &task);
    let reviewer = workflow
        .iter()
        .find(|w| w.capabilities.iter().any(|c| c == "plan"))
        .or_else(|| workflow.iter().find(|w| w.capabilities.iter().any(|c| c == "review")))
        .and_then(|w| get_agent(conn, w.agent_id).ok().flatten());
    let mut coders = Vec::new();
    for step in workflow
        .iter()
        .filter(|w| w.capabilities.iter().any(|c| c == "code"))
    {
        if let Some(a) = get_agent(conn, step.agent_id)? {
            coders.push(a);
        }
    }
    let roles = list_roles(conn)?;
    let iterations = list_iterations(conn, task_id)?;
    let subtasks = list_iteration_subtasks(conn, task_id, iteration_id)?;
    let logs = list_iteration_logs(conn, task_id, iteration_id)?;
    let execution_events = list_iteration_events(conn, task_id, iteration_id)?;
    let human_messages = list_human_messages(conn, task_id)?;
    Ok(crate::models::IterationDetail {
        task,
        iteration,
        iterations,
        subtasks,
        logs,
        reviewer,
        coders,
        workflow,
        roles,
        execution_events,
        human_messages,
    })
}

pub fn list_iteration_subtasks(
    conn: &Connection,
    task_id: i64,
    iteration_id: i64,
) -> Result<Vec<Subtask>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {SUBTASK_COLS} FROM subtask WHERE task_id=?1 AND iteration_id=?2 ORDER BY sort_index"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![task_id, iteration_id], row_to_subtask)
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

pub fn list_iteration_logs(
    conn: &Connection,
    task_id: i64,
    iteration_id: i64,
) -> Result<Vec<TaskLog>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id,task_id,iteration_id,level,message,create_time FROM task_log
             WHERE task_id=?1 AND iteration_id=?2 ORDER BY id ASC LIMIT 2000",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![task_id, iteration_id], row_to_log)
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

pub fn list_iteration_events(
    conn: &Connection,
    task_id: i64,
    iteration_id: i64,
) -> Result<Vec<TaskEvent>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id,task_id,iteration_id,subtask_id,kind,content,create_time,
                agent_name,role_name FROM task_event
             WHERE task_id=?1 AND iteration_id=?2 ORDER BY id ASC LIMIT 5000",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![task_id, iteration_id], row_to_event)
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
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
            "storage_root" => settings.storage_root = v,
            "log_persistence" => settings.log_persistence = v != "0",
            "default_execution_mode" => settings.default_execution_mode = v,
            "default_module_max_round" => settings.default_module_max_round = v,
            "default_global_max_round" => settings.default_global_max_round = v,
            "max_concurrent_tasks" => settings.max_concurrent_tasks = v,
            "scheduled_enabled" => settings.scheduled_enabled = v != "0",
            "llm_rate_limit_per_min" => settings.llm_rate_limit_per_min = v,
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
        ("storage_root", settings.storage_root.as_str()),
        (
            "log_persistence",
            if settings.log_persistence { "1" } else { "0" },
        ),
        (
            "default_execution_mode",
            settings.default_execution_mode.as_str(),
        ),
        (
            "default_module_max_round",
            settings.default_module_max_round.as_str(),
        ),
        (
            "default_global_max_round",
            settings.default_global_max_round.as_str(),
        ),
        (
            "max_concurrent_tasks",
            settings.max_concurrent_tasks.as_str(),
        ),
        (
            "scheduled_enabled",
            if settings.scheduled_enabled { "1" } else { "0" },
        ),
        (
            "llm_rate_limit_per_min",
            settings.llm_rate_limit_per_min.as_str(),
        ),
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
        .prepare(
            "SELECT id,status,execution_mode FROM task_main
             WHERE status IN ('running','planning')",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2).unwrap_or_else(|_| "later".into()),
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut items = Vec::new();
    for row in rows {
        items.push(row.map_err(|e| e.to_string())?);
    }
    for (id, status, mode) in &items {
        if mode == "scheduled" {
            conn.execute(
                "UPDATE task_main SET status='paused', pause_reason='waiting_window',
                    schedule_override=0 WHERE id=?1",
                params![id],
            )
            .map_err(|e| e.to_string())?;
            add_log(
                conn,
                *id,
                "warn",
                "应用重启：定时任务已暂停，等待下一个时间窗口自动继续",
            )?;
        } else {
            conn.execute(
                "UPDATE task_main SET status='failed', pause_reason='' WHERE id=?1",
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
    let current = get_task(conn, task_id)?.ok_or_else(|| "任务不存在".to_string())?;
    if let Some(workflow) = &cfg.workflow_agent_ids {
        if current.status != "pending" {
            return Err("任务已启动，工作流不可再修改".into());
        }
        save_task_workflow(conn, task_id, workflow)?;
    }
    if cfg.module_max_round < 1 || cfg.global_max_round < 1 {
        return Err("轮次上限必须大于等于 1".into());
    }
    if !matches!(
        cfg.execution_mode.trim(),
        "immediate" | "later" | "scheduled"
    ) {
        return Err("执行模式必须是 immediate / later / scheduled".into());
    }
    if !valid_clock(&cfg.schedule_start) || !valid_clock(&cfg.schedule_end) {
        return Err("定时窗口时间格式必须是 HH:MM".into());
    }
    if cfg.schedule_start.trim() == cfg.schedule_end.trim() {
        return Err("定时窗口开始时间和结束时间不能相同".into());
    }
    conn.execute(
        "UPDATE task_main SET module_max_round=?1, global_max_round=?2, manual_review_enabled=?3,
            execution_mode=?4, schedule_start=?5, schedule_end=?6, schedule_enabled=?7,
            schedule_override=0 WHERE id=?8",
        params![
            cfg.module_max_round,
            cfg.global_max_round,
            if cfg.manual_review_enabled { 1 } else { 0 },
            cfg.execution_mode.trim(),
            cfg.schedule_start.trim(),
            cfg.schedule_end.trim(),
            if cfg.execution_mode == "scheduled" && cfg.schedule_enabled {
                1
            } else {
                0
            },
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
        iteration_id: row.get("iteration_id").unwrap_or_default(),
        subtask_id: row.get("subtask_id")?,
        kind: row.get("kind")?,
        content: row.get("content")?,
        create_time: row.get("create_time")?,
        agent_name: row.get("agent_name").unwrap_or_default(),
        role_name: row.get("role_name").unwrap_or_default(),
    })
}

/// 构造只推送给前端、不落库的实时事件（日志持久化关闭时使用）。
pub fn live_event(
    task_id: i64,
    iteration_id: i64,
    subtask_id: Option<i64>,
    kind: &str,
    content: &str,
) -> TaskEvent {
    TaskEvent {
        id: synthetic_id(),
        task_id,
        iteration_id,
        subtask_id,
        kind: kind.to_string(),
        content: content.to_string(),
        create_time: now(),
        agent_name: String::new(),
        role_name: String::new(),
    }
}

pub fn add_event(
    conn: &Connection,
    task_id: i64,
    subtask_id: Option<i64>,
    kind: &str,
    content: &str,
) -> Result<TaskEvent, String> {
    add_event_with_agent(conn, task_id, subtask_id, kind, content, "", "")
}

/// 带 Agent 归属的事件写入：时间线据此显示“谁在说话”。
pub fn add_event_with_agent(
    conn: &Connection,
    task_id: i64,
    subtask_id: Option<i64>,
    kind: &str,
    content: &str,
    agent_name: &str,
    role_name: &str,
) -> Result<TaskEvent, String> {
    let create_time = now();
    if !log_persistence() {
        return Ok(TaskEvent {
            id: synthetic_id(),
            task_id,
            iteration_id: current_iteration_id(conn, task_id),
            subtask_id,
            kind: kind.to_string(),
            content: content.to_string(),
            create_time,
            agent_name: agent_name.to_string(),
            role_name: role_name.to_string(),
        });
    }
    let iteration_id = current_iteration_id(conn, task_id);
    conn.execute(
        "INSERT INTO task_event(task_id,iteration_id,subtask_id,kind,content,create_time,agent_name,role_name)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            task_id,
            iteration_id,
            subtask_id,
            kind,
            content,
            create_time,
            agent_name,
            role_name
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(TaskEvent {
        id: conn.last_insert_rowid(),
        task_id,
        iteration_id,
        subtask_id,
        kind: kind.to_string(),
        content: content.to_string(),
        create_time,
        agent_name: agent_name.to_string(),
        role_name: role_name.to_string(),
    })
}

pub fn list_events(conn: &Connection, task_id: i64, limit: i64) -> Result<Vec<TaskEvent>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id,task_id,iteration_id,subtask_id,kind,content,create_time,agent_name,role_name FROM (
                SELECT id,task_id,iteration_id,subtask_id,kind,content,create_time,agent_name,role_name FROM task_event
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

/// 全量事件读取（仅用户主动打开「原始事件」时调用）。
pub fn list_all_events(conn: &Connection, task_id: i64) -> Result<Vec<TaskEvent>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id,task_id,iteration_id,subtask_id,kind,content,create_time,agent_name,role_name FROM task_event
             WHERE task_id=?1 ORDER BY id ASC LIMIT 50000",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![task_id], row_to_event)
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

pub fn add_chat_message(
    conn: &Connection,
    task_id: i64,
    role: &str,
    content: &str,
) -> Result<ChatMessage, String> {
    let create_time = now();
    conn.execute(
        "INSERT INTO chat_message(task_id,role,content,create_time) VALUES (?1,?2,?3,?4)",
        params![task_id, role, content, create_time],
    )
    .map_err(|e| e.to_string())?;
    Ok(ChatMessage {
        id: conn.last_insert_rowid(),
        task_id,
        role: role.to_string(),
        content: content.to_string(),
        create_time,
    })
}

pub fn list_chat_messages(
    conn: &Connection,
    task_id: i64,
    limit: i64,
) -> Result<Vec<ChatMessage>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id,task_id,role,content,create_time FROM (
                SELECT id,task_id,role,content,create_time FROM chat_message
                WHERE task_id=?1 ORDER BY id DESC LIMIT ?2
             ) ORDER BY id ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![task_id, limit], |row| {
            Ok(ChatMessage {
                id: row.get("id")?,
                task_id: row.get("task_id")?,
                role: row.get("role")?,
                content: row.get("content")?,
                create_time: row.get("create_time")?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

pub fn clear_chat_messages(conn: &Connection, task_id: i64) -> Result<(), String> {
    conn.execute(
        "DELETE FROM chat_message WHERE task_id=?1",
        params![task_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 运行中追加/修改顶层需求与验收标准。
pub fn update_requirements(
    conn: &Connection,
    task_id: i64,
    top_requirement: &str,
    accept_criteria: &str,
) -> Result<TaskMain, String> {
    if top_requirement.trim().is_empty() {
        return Err("顶层需求不能为空".into());
    }
    if accept_criteria.trim().is_empty() {
        return Err("项目验收标准不能为空".into());
    }
    conn.execute(
        "UPDATE task_main SET top_requirement=?1, global_accept_criteria=?2 WHERE id=?3",
        params![top_requirement.trim(), accept_criteria.trim(), task_id],
    )
    .map_err(|e| e.to_string())?;
    // 仅有一个迭代（尚未追加过需求）时，同步 dev-1.0 的需求快照。
    let settled: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM iteration WHERE task_id=?1 AND version<>''",
            params![task_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if settled <= 1 {
        conn.execute(
            "UPDATE iteration SET requirement=?1 WHERE task_id=?2 AND version='dev-1.0'",
            params![top_requirement.trim(), task_id],
        )
        .map_err(|e| e.to_string())?;
    }
    get_task(conn, task_id)?.ok_or_else(|| "任务不存在".to_string())
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
    let first_coder = resolve_task_agents(conn, task, crate::models::CAP_CODE)?
        .first()
        .map(|a| a.id)
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

/// 评审完成对追加需求的梳理后，一次性写入新需求、追加子任务并重置本次迭代的全局评审轮次。
pub fn apply_followup_requirement(
    conn: &mut Connection,
    task: &mut TaskMain,
    requirement: &str,
    analysis: &str,
    items: &[(String, String)],
) -> Result<Vec<Subtask>, String> {
    if items.is_empty() {
        return Err("追加需求没有生成任何子任务".into());
    }
    let first_coder = resolve_task_agents(conn, task, crate::models::CAP_CODE)?
        .first()
        .map(|a| a.id)
        .ok_or_else(|| "任务未绑定 Coder".to_string())?;
    let appended = format!(
        "{}\n\n---\n## 追加需求（{}）\n{}\n\n### 评审梳理\n{}",
        task.top_requirement.trim(),
        now(),
        requirement.trim(),
        analysis.trim()
    );
    let next_cycle = task.global_review_cycle.max(1) + 1;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "UPDATE task_main SET top_requirement=?1, followup_requirement='', followup_mode='auto',
            task_phase='module_dev', status='running', current_global_round=0,
            global_review_cycle=?2, pause_reason='' WHERE id=?3",
        params![appended, next_cycle, task.id],
    )
    .map_err(|e| e.to_string())?;
    let start: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(sort_index), 0) + 1 FROM subtask WHERE task_id=?1",
            params![task.id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let iteration_id = current_iteration_id(&tx, task.id);
    for (i, (name, criteria)) in items.iter().enumerate() {
        tx.execute(
            "INSERT INTO subtask(task_id,subtask_name,sort_index,accept_criteria,assigned_coder_id,
                current_round,max_round,status,current_stage,summary_content,summary_file_path,iteration_records,design_content,iteration_id)
             VALUES (?1,?2,?3,?4,?5,0,?6,'waiting','queued','','','[]','',?7)",
            params![
                task.id,
                name.trim(),
                start + i as i64,
                criteria,
                first_coder,
                task.module_max_round,
                iteration_id
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    task.top_requirement = appended;
    task.followup_requirement.clear();
    task.followup_mode = "auto".into();
    task.task_phase = "module_dev".into();
    task.status = "running".into();
    task.current_global_round = 0;
    task.global_review_cycle = next_cycle;
    task.pause_reason.clear();
    list_subtasks(conn, task.id)
}

pub fn set_paused(conn: &Connection, task_id: i64, message: &str) -> Result<(), String> {
    if let Some(mut task) = get_task(conn, task_id)? {
        if task.status != "terminated" {
            task.status = "paused".into();
            if task.pause_reason.trim().is_empty() {
                task.pause_reason = "manual".into();
            }
        }
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
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 日志持久化开关是进程级全局变量：涉及事件写入的用例串行执行，避免互相干扰。
    static LOG_PERSIST_LOCK: Mutex<()> = Mutex::new(());

    /// 测试库：建表 + 角色种子 + 默认 reviewer/coder（id 1/2）。
    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        seed_roles(&conn).unwrap();
        for (name, role) in [("R", "reviewer"), ("C", "coder")] {
            conn.execute(
                "INSERT INTO agent_config(name,role_type,provider,model_name,base_url,api_key,system_prompt,temperature,create_time)
                 VALUES (?1,?2,'openai-compatible','mock','http://127.0.0.1:1/v1','k','',0.2,'2026-01-01 00:00:00')",
                params![name, role],
            )
            .unwrap();
        }
        conn
    }

    /// 关闭日志持久化时事件只返回给前端，不写入数据库。
    #[test]
    fn log_persistence_off_skips_event_storage() {
        let _guard = LOG_PERSIST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let conn = test_db();
        conn.execute(
            "INSERT INTO task_main(task_name,top_requirement,global_accept_criteria,reviewer_agent_id,
                coder_agent_ids,module_max_round,global_max_round,work_dir,create_time)
             VALUES ('t','r','a',1,'[1]',1,1,'/tmp','2026-01-01 00:00:00')",
            [],
        )
        .unwrap();
        set_log_persistence(false);
        let ev = add_event(&conn, 1, None, "agent_message:coder", "hello").unwrap();
        assert!(ev.id < 0);
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM task_event", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
        set_log_persistence(true);
        let ev = add_event(&conn, 1, None, "agent_message:coder", "hello").unwrap();
        assert!(ev.id > 0);
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM task_event", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

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
            assert_eq!(task.execution_mode, "immediate");
            assert!(task.schedule_enabled);
            // 新表存在
            let n: i64 = conn
                .query_row("SELECT COUNT(*) FROM task_event", [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 0);
            let n: i64 = conn
                .query_row("SELECT COUNT(*) FROM chat_message", [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 0);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn new_task_input() -> NewTaskInput {
        NewTaskInput {
            task_name: "迭代任务".into(),
            top_requirement: "做一个日历".into(),
            global_accept_criteria: "能新增日程".into(),
            reviewer_agent_id: 1,
            coder_agent_ids: vec![2],
            workflow_agent_ids: vec![],
            module_max_round: 3,
            global_max_round: 2,
            work_dir: "/tmp/agentcrew-iter".into(),
            manual_review_enabled: false,
            execution_mode: "later".into(),
            schedule_start: "01:00".into(),
            schedule_end: "06:00".into(),
            schedule_enabled: true,
        }
    }

    /// 新建任务即登记 dev-1.0，且首个迭代处于 pending 状态。
    #[test]
    fn create_task_registers_dev_1_0_iteration() {
        let conn = test_db();
        let task_id = create_task(&conn, &new_task_input()).unwrap();
        let iterations = list_iterations(&conn, task_id).unwrap();
        assert_eq!(iterations.len(), 1);
        assert_eq!(iterations[0].version, "dev-1.0");
        assert_eq!(iterations[0].status, "pending");
        assert_eq!(iterations[0].requirement, "做一个日历");
        let summary_version = list_tasks(&conn).unwrap()[0].version.clone();
        assert_eq!(summary_version, "dev-1.0");
    }

    /// 版本号规则：默认小版本 +1；子任务超过 5 个时升大版本、小版本归零。
    #[test]
    fn iteration_version_rule_bumps_minor_or_major() {
        let conn = test_db();
        let task_id = create_task(&conn, &new_task_input()).unwrap();

        // 第一个迭代已落定 dev-1.0，下一次默认 dev-1.1。
        let (version, major, minor) = next_iteration_version(&conn, task_id, 1).unwrap();
        assert_eq!((version.as_str(), major, minor), ("dev-1.1", 1, 1));

        // 落定 dev-1.1 后，5 个子任务仍然是小版本。
        let iter2 = create_iteration(&conn, task_id, "加导出".into()).unwrap();
        settle_iteration(&conn, iter2, "dev-1.1", 1, 1).unwrap();
        let (version, _, _) = next_iteration_version(&conn, task_id, 5).unwrap();
        assert_eq!(version, "dev-1.2");

        // 6 个子任务 -> 大版本 +1。
        let (version, major, minor) = next_iteration_version(&conn, task_id, 6).unwrap();
        assert_eq!((version.as_str(), major, minor), ("dev-2.0", 2, 0));

        let iter3 = create_iteration(&conn, task_id, "大改造".into()).unwrap();
        settle_iteration(&conn, iter3, "dev-2.0", 2, 0).unwrap();
        let (version, _, _) = next_iteration_version(&conn, task_id, 1).unwrap();
        assert_eq!(version, "dev-2.1");
    }

    /// 子任务 / 日志 / 事件写入时自动归属当前迭代，详情可按迭代隔离。
    #[test]
    fn records_are_scoped_to_current_iteration() {
        let _guard = LOG_PERSIST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_log_persistence(true);
        let conn = test_db();
        let task_id = create_task(&conn, &new_task_input()).unwrap();
        let first = current_iteration_id(&conn, task_id);
        let ev = add_event(&conn, task_id, None, "agent_message:reviewer", "第一版").unwrap();
        assert_eq!(ev.iteration_id, first);

        let second = create_iteration(&conn, task_id, "追加需求".into()).unwrap();
        settle_iteration(&conn, second, "dev-1.1", 1, 1).unwrap();
        let ev2 = add_event(&conn, task_id, None, "agent_message:coder", "第二版").unwrap();
        assert_eq!(ev2.iteration_id, second);

        let detail = get_iteration_detail(&conn, task_id, first).unwrap();
        assert_eq!(detail.execution_events.len(), 1);
        assert_eq!(detail.execution_events[0].content, "第一版");
        assert_eq!(detail.iterations.len(), 2);
        let detail2 = get_iteration_detail(&conn, task_id, second).unwrap();
        assert_eq!(detail2.execution_events[0].content, "第二版");
    }

    /// 迭代开始/结束/重开的时间与状态流转。
    #[test]
    fn iteration_lifecycle_tracks_time_and_status() {
        let conn = test_db();
        let task_id = create_task(&conn, &new_task_input()).unwrap();
        mark_iteration_started(&conn, task_id).unwrap();
        let it = list_iterations(&conn, task_id).unwrap().remove(0);
        assert_eq!(it.status, "running");
        assert!(!it.start_time.is_empty());

        mark_iteration_finished(&conn, task_id, "terminated").unwrap();
        let it = list_iterations(&conn, task_id).unwrap().remove(0);
        assert_eq!(it.status, "terminated");
        assert!(!it.end_time.is_empty());
        assert!(it.duration_sec >= 0);

        reopen_current_iteration(&conn, task_id).unwrap();
        let it = list_iterations(&conn, task_id).unwrap().remove(0);
        assert_eq!(it.status, "running");
        assert!(it.end_time.is_empty());
    }

    /// token 累计：新迭代从 0 开始累加；旧迭代（-1）首次收到用量时从实际值开始。
    #[test]
    fn iteration_tokens_accumulate() {
        let conn = test_db();
        let task_id = create_task(&conn, &new_task_input()).unwrap();
        let it = list_iterations(&conn, task_id).unwrap().remove(0);
        assert_eq!(it.token_total, 0, "new iteration starts tracked at 0");

        add_iteration_tokens(&conn, task_id, 120).unwrap();
        add_iteration_tokens(&conn, task_id, 80).unwrap();
        let it = list_iterations(&conn, task_id).unwrap().remove(0);
        assert_eq!(it.token_total, 200);

        // 模拟旧版本迭代（未统计 = -1）
        conn.execute(
            "UPDATE iteration SET token_total=-1 WHERE id=?1",
            params![it.id],
        )
        .unwrap();
        add_iteration_tokens(&conn, task_id, 50).unwrap();
        let it = list_iterations(&conn, task_id).unwrap().remove(0);
        assert_eq!(it.token_total, 50, "legacy -1 switches to real number");

        // 新增迭代各自独立累计
        let second = create_iteration(&conn, task_id, "追加需求".into()).unwrap();
        settle_iteration(&conn, second, "dev-1.1", 1, 1).unwrap();
        add_iteration_tokens(&conn, task_id, 30).unwrap();
        let iterations = list_iterations(&conn, task_id).unwrap();
        assert_eq!(iterations[0].token_total, 50);
        assert_eq!(iterations[1].token_total, 30);
    }

    /// 迁移必须把旧任务的子任务/日志归档进 dev-1.0，且重复执行无副作用。
    #[test]
    fn migration_backfills_legacy_task_into_dev_1_0() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("orch-iter-{}-{}", std::process::id(), nanos));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("orchestrator.db");
        {
            let conn = Connection::open(&path).unwrap();
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
                VALUES ('旧任务','旧需求','旧验收',1,'[1]',3,2,'/tmp','2026-01-01 00:00:00');
                INSERT INTO subtask(task_id,subtask_name,sort_index,max_round)
                VALUES (1,'旧模块',1,3);
                "#,
            )
            .unwrap();
            conn.execute_batch(SCHEMA).unwrap();
            migrate(&conn).unwrap();
            // 幂等：再执行一次不应重复插入迭代。
            migrate(&conn).unwrap();
            let iterations = list_iterations(&conn, 1).unwrap();
            assert_eq!(iterations.len(), 1);
            assert_eq!(iterations[0].version, "dev-1.0");
            assert_eq!(iterations[0].requirement, "旧需求");
            assert_eq!(iterations[0].subtask_total, 1);
            let task = get_task(&conn, 1).unwrap().unwrap();
            assert_eq!(task.workflow_agent_ids, vec![1], "旧任务应回填工作流");
            let subtask = list_subtasks(&conn, 1).unwrap().remove(0);
            assert_eq!(subtask.iteration_id, iterations[0].id);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------------- V2.5：角色类型 / 线性工作流 ----------------

    fn make_agent(conn: &Connection, name: &str, role: &str) -> AgentConfig {
        save_agent(
            conn,
            &AgentInput {
                id: None,
                name: name.into(),
                role_type: role.into(),
                provider: "openai-compatible".into(),
                model_name: "mock".into(),
                base_url: "http://127.0.0.1:1/v1".into(),
                api_key: "k".into(),
                system_prompt: String::new(),
                temperature: 0.2,
            },
        )
        .unwrap()
    }

    /// 内置角色必须存在且不可删除；自定义角色可增删改。
    #[test]
    fn roles_seeded_and_builtin_protected() {
        let conn = test_db();
        let roles = list_roles(&conn).unwrap();
        let reviewer = roles.iter().find(|r| r.key == "reviewer").unwrap();
        assert!(reviewer.is_builtin);
        assert!(reviewer.capabilities.iter().any(|c| c == "plan"));
        let coder = roles.iter().find(|r| r.key == "coder").unwrap();
        assert!(coder.capabilities.iter().any(|c| c == "code"));
        // 预置产品经理角色（可删除）
        assert!(roles.iter().any(|r| r.key == "product_manager"));

        let err = delete_role(&conn, reviewer.id).unwrap_err();
        assert!(err.contains("内置角色"));

        let pm = save_role(
            &conn,
            &RoleInput {
                id: None,
                key: "product_owner".into(),
                name: "产品负责人".into(),
                description: "负责需求".into(),
                capabilities: vec!["requirement".into()],
                color: "#123456".into(),
            },
        )
        .unwrap();
        assert_eq!(pm.key, "product_owner");
        make_agent(&conn, "PO", "product_owner");
        // 有 Agent 引用时不允许删除
        let err = delete_role(&conn, pm.id).unwrap_err();
        assert!(err.contains("仍有 1 个 Agent"));
        // 内置角色能力不可被修改
        let updated = save_role(
            &conn,
            &RoleInput {
                id: Some(reviewer.id),
                key: String::new(),
                name: "改名".into(),
                description: "内置说明".into(),
                capabilities: vec!["code".into()],
                color: String::new(),
            },
        )
        .unwrap();
        assert_eq!(updated.capabilities, reviewer.capabilities);
        assert_eq!(updated.description, "内置说明");
    }

    /// 工作流必须覆盖方案/编码/评审三种能力，且不允许重复 Agent。
    #[test]
    fn workflow_validation_requires_capabilities() {
        let conn = test_db();
        let reviewer = get_agent(&conn, 1).unwrap().unwrap();
        let coder = get_agent(&conn, 2).unwrap().unwrap();
        assert!(validate_workflow(&conn, &[reviewer.id, coder.id]).is_ok());
        assert!(validate_workflow(&conn, &[reviewer.id]).is_err());
        assert!(validate_workflow(&conn, &[reviewer.id, reviewer.id, coder.id])
            .unwrap_err()
            .contains("重复"));
        let req_only = save_role(
            &conn,
            &RoleInput {
                id: None,
                key: "pm_only".into(),
                name: "产品经理".into(),
                description: String::new(),
                capabilities: vec!["requirement".into()],
                color: String::new(),
            },
        )
        .unwrap();
        let pm = make_agent(&conn, "PM", &req_only.key);
        // 只有需求能力的 Agent 不能替代方案/评审能力
        let err = validate_workflow(&conn, &[pm.id, coder.id]).unwrap_err();
        assert!(err.contains("方案设计"));
        // 加入评审者后合法
        assert!(validate_workflow(&conn, &[pm.id, reviewer.id, coder.id]).is_ok());
    }

    /// 按能力解析时严格按工作流顺序找第一个匹配的 Agent。
    #[test]
    fn resolve_agents_follow_workflow_order() {
        let conn = test_db();
        let reviewer = get_agent(&conn, 1).unwrap().unwrap();
        let coder = get_agent(&conn, 2).unwrap().unwrap();
        let pm = save_role(
            &conn,
            &RoleInput {
                id: None,
                key: "pm".into(),
                name: "产品经理".into(),
                description: String::new(),
                capabilities: vec!["requirement".into()],
                color: String::new(),
            },
        )
        .unwrap();
        let pm_agent = make_agent(&conn, "PM", &pm.key);
        let task = TaskMain {
            id: 1,
            task_name: "t".into(),
            top_requirement: "r".into(),
            global_accept_criteria: "a".into(),
            reviewer_agent_id: reviewer.id,
            coder_agent_ids: vec![coder.id],
            workflow_agent_ids: vec![pm_agent.id, reviewer.id, coder.id],
            module_max_round: 3,
            global_max_round: 2,
            global_contract: String::new(),
            task_phase: "module_dev".into(),
            status: "pending".into(),
            work_dir: "/tmp".into(),
            current_global_round: 0,
            global_review_cycle: 1,
            global_review_records: Vec::new(),
            create_time: String::new(),
            baseline_tree: String::new(),
            manual_review_enabled: false,
            tech_stack: String::new(),
            pending_plan_json: String::new(),
            cancel_requested: false,
            execution_mode: "later".into(),
            schedule_start: "01:00".into(),
            schedule_end: "06:00".into(),
            schedule_enabled: false,
            schedule_override: false,
            start_requested: false,
            pause_reason: String::new(),
            followup_requirement: String::new(),
            followup_mode: "auto".into(),
        };
        assert_eq!(
            resolve_task_agent(&conn, &task, crate::models::CAP_REQUIREMENT)
                .unwrap()
                .id,
            pm_agent.id
        );
        assert_eq!(
            resolve_task_agent(&conn, &task, crate::models::CAP_PLAN)
                .unwrap()
                .id,
            reviewer.id
        );
        assert_eq!(
            resolve_task_agent(&conn, &task, crate::models::CAP_REVIEW)
                .unwrap()
                .id,
            reviewer.id
        );
        assert_eq!(
            resolve_task_agent(&conn, &task, crate::models::CAP_CODE)
                .unwrap()
                .id,
            coder.id
        );
        let steps = workflow_steps(&conn, &task);
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].role_name, "产品经理");
    }

    /// 旧库 agent_config 的 reviewer/coder CHECK 约束必须能无损放开。
    #[test]
    fn migration_drops_agent_role_check_constraint() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE agent_config (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                role_type TEXT NOT NULL CHECK (role_type IN ('reviewer','coder')),
                provider TEXT NOT NULL DEFAULT '',
                model_name TEXT NOT NULL DEFAULT '',
                base_url TEXT NOT NULL DEFAULT '',
                api_key TEXT NOT NULL DEFAULT '',
                system_prompt TEXT NOT NULL DEFAULT '',
                temperature REAL NOT NULL DEFAULT 0.3,
                create_time TEXT NOT NULL
            );
            INSERT INTO agent_config(name,role_type,model_name,base_url,create_time)
            VALUES ('老评审','reviewer','m','u','2026-01-01 00:00:00');
            INSERT INTO agent_config(name,role_type,model_name,base_url,create_time)
            VALUES ('老编码','coder','m','u','2026-01-01 00:00:00');",
        )
        .unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        migrate(&conn).unwrap();
        let agents = list_agents(&conn).unwrap();
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].role_name, "评审者");
        // 迁移后可以保存自定义角色（旧 CHECK 已移除）
        let pm = save_role(
            &conn,
            &RoleInput {
                id: None,
                key: "product_manager2".into(),
                name: "产品经理2".into(),
                description: String::new(),
                capabilities: vec!["requirement".into()],
                color: String::new(),
            },
        )
        .unwrap();
        save_agent(
            &conn,
            &AgentInput {
                id: None,
                name: "PM2".into(),
                role_type: pm.key,
                provider: "openai-compatible".into(),
                model_name: "m".into(),
                base_url: "u".into(),
                api_key: "k".into(),
                system_prompt: String::new(),
                temperature: 0.2,
            },
        )
        .unwrap();
        assert!(validate_workflow(&conn, &[agents[0].id, agents[1].id]).is_ok());
    }

    /// 任务只能在没有启动前修改工作流。
    #[test]
    fn workflow_editable_only_before_start() {
        let conn = test_db();
        let task_id = create_task(&conn, &new_task_input()).unwrap();
        let cfg = TaskConfigInput {
            module_max_round: 3,
            global_max_round: 2,
            manual_review_enabled: false,
            workflow_agent_ids: Some(vec![2, 1]),
            execution_mode: "later".into(),
            schedule_start: "01:00".into(),
            schedule_end: "06:00".into(),
            schedule_enabled: false,
        };
        let updated = update_task_config(&conn, task_id, &cfg).unwrap();
        assert_eq!(updated.workflow_agent_ids, vec![2, 1]);
        // 变成运行中后禁止修改
        conn.execute(
            "UPDATE task_main SET status='running' WHERE id=?1",
            params![task_id],
        )
        .unwrap();
        let err = update_task_config(&conn, task_id, &cfg).unwrap_err();
        assert!(err.contains("不可再修改"));
    }
}
