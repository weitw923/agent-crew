-- AgentCrew V2 SQLite 建表 SQL（与 Rust 内 SCHEMA 一致）

-- V2.5：角色类型（能力标签决定任务工作流中的阶段映射）
CREATE TABLE IF NOT EXISTS role_type (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    key          TEXT NOT NULL UNIQUE,
    name         TEXT NOT NULL,
    description  TEXT NOT NULL DEFAULT '',
    capabilities TEXT NOT NULL DEFAULT '[]',   -- JSON: ["requirement","plan","code","review"]
    color        TEXT NOT NULL DEFAULT '',
    is_builtin   INTEGER NOT NULL DEFAULT 0,
    sort_order   INTEGER NOT NULL DEFAULT 0,
    create_time  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS agent_config (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    name          TEXT NOT NULL,
    role_type     TEXT NOT NULL,               -- role_type.key
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
    coder_agent_ids        TEXT NOT NULL,              -- JSON: [1,2]（旧字段，兼容保留）
    workflow_agent_ids     TEXT NOT NULL DEFAULT '',   -- JSON: [3,1,2] 任务级线性工作流
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
    iteration_id      INTEGER NOT NULL DEFAULT 0   -- 所属迭代
);

CREATE TABLE IF NOT EXISTS iteration (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id      INTEGER NOT NULL REFERENCES task_main(id) ON DELETE CASCADE,
    version      TEXT NOT NULL DEFAULT '',      -- dev-1.0 / dev-1.1；需求梳理完成前为空（待定）
    major        INTEGER NOT NULL DEFAULT 0,
    minor        INTEGER NOT NULL DEFAULT 0,
    requirement  TEXT NOT NULL DEFAULT '',      -- 本次迭代对应的增量需求
    analysis     TEXT NOT NULL DEFAULT '',      -- 评审对该次需求的梳理结论
    status       TEXT NOT NULL DEFAULT 'pending', -- pending/running/completed/failed/terminated
    start_time   TEXT NOT NULL DEFAULT '',
    end_time     TEXT NOT NULL DEFAULT '',
    token_total  INTEGER NOT NULL DEFAULT -1,   -- 累计 token；-1 = 统计上线前，不可回溯
    create_time  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS task_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id     INTEGER NOT NULL REFERENCES task_main(id) ON DELETE CASCADE,
    level       TEXT NOT NULL DEFAULT 'info',
    iteration_id INTEGER NOT NULL DEFAULT 0,   -- 归属迭代
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
    iteration_id INTEGER NOT NULL DEFAULT 0,   -- 归属迭代
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
