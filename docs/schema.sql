-- AgentCrew SQLite 建表 SQL
-- 与 src-tauri/src/db.rs 的 SCHEMA 保持一致
-- 旧数据库会在启动时通过 ALTER TABLE 自动补齐新增列

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
