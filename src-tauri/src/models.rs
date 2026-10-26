use serde::{Deserialize, Serialize};

/// Agent 配置（数据库实体，也直接返回给前端）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub id: i64,
    pub name: String,
    pub role_type: String, // reviewer / coder
    pub provider: String,
    pub model_name: String,
    pub base_url: String,
    pub api_key: String,
    pub system_prompt: String,
    pub temperature: f64,
    pub create_time: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentInput {
    pub id: Option<i64>,
    pub name: String,
    pub role_type: String,
    pub provider: String,
    pub model_name: String,
    pub base_url: String,
    pub api_key: String,
    pub system_prompt: String,
    pub temperature: f64,
}

/// 单个子任务的一轮归档记录。默认仅用于页面审计，不会送入 LLM 上下文。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IterationRecord {
    pub round: i64,
    #[serde(rename = "type")]
    pub record_type: String, // code_review / summary_review / human_reject / global_rework
    pub coder_agent_id: Option<i64>,
    pub diff: String,
    pub review_feedback: String,
    pub review_decision: String, // pass / reject
    pub summary_content: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GlobalReviewIssue {
    pub subtask_id: Option<i64>,
    pub subtask_name: String,
    pub feedback: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GlobalReviewRecord {
    pub round: i64,
    pub passed: bool,
    pub feedback: String,
    pub issues: Vec<GlobalReviewIssue>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskMain {
    pub id: i64,
    pub task_name: String,
    pub top_requirement: String,
    pub global_accept_criteria: String,
    pub reviewer_agent_id: i64,
    pub coder_agent_ids: Vec<i64>,
    pub module_max_round: i64,
    pub global_max_round: i64,
    pub global_contract: String,
    pub task_phase: String, // module_dev / global_review
    pub status: String,     // planning / running / needs_human / completed / failed
    pub work_dir: String,
    pub current_global_round: i64,
    pub global_review_records: Vec<GlobalReviewRecord>,
    pub create_time: String,
    /// 任务启动时的 git tree 快照，用于计算整个项目的累计 diff（额外内部字段）。
    #[serde(default)]
    pub baseline_tree: String,
    /// 人工审核模式：打开后整体方案要审批，每个模块设计文档也要逐一审批。
    #[serde(default)]
    pub manual_review_enabled: bool,
    /// 技术总监选定的技术栈说明。
    #[serde(default)]
    pub tech_stack: String,
    /// 等待人工审批的方案 JSON（拆解结果，尚未创建 subtask 记录）。
    #[serde(default)]
    pub pending_plan_json: String,
    /// 人工终止请求标记；恢复调度时清零。
    #[serde(default)]
    pub cancel_requested: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subtask {
    pub id: i64,
    pub task_id: i64,
    pub subtask_name: String,
    pub sort_index: i64,
    pub accept_criteria: String,
    pub assigned_coder_id: Option<i64>,
    pub current_round: i64,
    pub max_round: i64,
    pub status: String,        // waiting / in_progress / completed / needs_human
    pub current_stage: String, // queued / coding / code_review / summary_generation / summary_review / done / needs_human
    pub summary_content: String,
    pub summary_file_path: String,
    pub iteration_records: Vec<IterationRecord>,
    /// 技术总监为该模块生成的详细设计文档（Coder 开工前生成）。
    #[serde(default)]
    pub design_content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskLog {
    pub id: i64,
    pub task_id: i64,
    pub level: String,
    pub message: String,
    pub create_time: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PendingPlanItem {
    pub name: String,
    #[serde(default, alias = "acceptance_criteria")]
    pub accept_criteria: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PendingPlan {
    #[serde(default)]
    pub tech_stack: String,
    #[serde(default)]
    pub global_contract: String,
    #[serde(default)]
    pub subtasks: Vec<PendingPlanItem>,
}

/// 执行过程中的全量输出事件。与高层 task_log 分开，用于页面实时执行流。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskEvent {
    pub id: i64,
    pub task_id: i64,
    pub subtask_id: Option<i64>,
    pub kind: String,
    pub content: String,
    pub create_time: String,
}

/// 人工随时输入的消息，pending 状态会在下一个 Agent 步骤注入。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HumanMessage {
    pub id: i64,
    pub task_id: i64,
    pub content: String,
    pub status: String, // pending / consumed
    pub create_time: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskDetail {
    pub task: TaskMain,
    pub subtasks: Vec<Subtask>,
    pub logs: Vec<TaskLog>,
    pub reviewer: Option<AgentConfig>,
    pub coders: Vec<AgentConfig>,
    #[serde(default)]
    pub execution_events: Vec<TaskEvent>,
    #[serde(default)]
    pub human_messages: Vec<HumanMessage>,
    #[serde(default)]
    pub pending_plan: Option<PendingPlan>,
}

/// 列表页使用的扁平结构：TaskMain 字段 + 展示用派生字段。
#[derive(Debug, Clone, Serialize)]
pub struct TaskSummary {
    #[serde(flatten)]
    pub task: TaskMain,
    pub reviewer_name: String,
    pub coder_names: Vec<String>,
    pub subtask_count: i64,
    pub completed_count: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewTaskInput {
    pub task_name: String,
    pub top_requirement: String,
    pub global_accept_criteria: String,
    pub reviewer_agent_id: i64,
    pub coder_agent_ids: Vec<i64>,
    pub module_max_round: i64,
    pub global_max_round: i64,
    pub work_dir: String,
    #[serde(default)]
    pub manual_review_enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskConfigInput {
    pub module_max_round: i64,
    pub global_max_round: i64,
    pub manual_review_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub default_work_dir: String,
    pub llm_timeout_sec: String,
    pub dsh_timeout_sec: String,
    pub dsh_bin: String,
    pub dsh_profile: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        Self {
            default_work_dir: format!("{home}/Projects"),
            llm_timeout_sec: "180".to_string(),
            dsh_timeout_sec: "1800".to_string(),
            dsh_bin: "dsh".to_string(),
            dsh_profile: "headless".to_string(),
        }
    }
}
