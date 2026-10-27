use serde::{Deserialize, Serialize};

/// Agent 配置（数据库实体，也直接返回给前端）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub id: i64,
    pub name: String,
    pub role_type: String, // 角色 key：reviewer / coder / 自定义角色
    pub provider: String,
    pub model_name: String,
    pub base_url: String,
    pub api_key: String,
    pub system_prompt: String,
    pub temperature: f64,
    pub create_time: String,
    /// 派生字段：角色展示名 / 颜色 / 能力标签（由 role_type JOIN role_type 表得到）。
    #[serde(default)]
    pub role_name: String,
    #[serde(default)]
    pub role_color: String,
    #[serde(default)]
    pub role_capabilities: Vec<String>,
}

/// 角色类型：能力标签 + 展示信息。内置 reviewer / coder 不允许删除。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleType {
    pub id: i64,
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// 能力标签：requirement / plan / code / review。
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub is_builtin: bool,
    #[serde(default)]
    pub sort_order: i64,
    pub create_time: String,
    /// 派生字段：使用该角色的 Agent 数量。
    #[serde(default)]
    pub agent_count: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RoleInput {
    pub id: Option<i64>,
    /// 自定义角色标识；为空时后端自动生成。
    #[serde(default)]
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub color: String,
}

/// 任务工作流中的一个步骤（派生视图，不落库）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub order: i64,
    pub agent_id: i64,
    pub agent_name: String,
    pub role_key: String,
    pub role_name: String,
    pub role_color: String,
    pub capabilities: Vec<String>,
}

/// 能力标签常量。
pub const CAP_REQUIREMENT: &str = "requirement";
pub const CAP_PLAN: &str = "plan";
pub const CAP_CODE: &str = "code";
pub const CAP_REVIEW: &str = "review";

pub fn capability_label(cap: &str) -> &'static str {
    match cap {
        CAP_REQUIREMENT => "需求梳理",
        CAP_PLAN => "方案设计",
        CAP_CODE => "编码实现",
        CAP_REVIEW => "评审验收",
        _ => "未知能力",
    }
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

/// 任务迭代（版本）：dev-1.0 / dev-1.1 / dev-2.0 …
///
/// 首次创建任务即产生 dev-1.0；之后每追加一次需求新增一个迭代，
/// 默认小版本 +1；若该次追加拆出的子任务超过 5 个，则大版本 +1、小版本归零。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Iteration {
    pub id: i64,
    pub task_id: i64,
    /// 落定后的版本号（如 dev-1.1）；需求梳理完成前为空字符串（UI 显示为待定）。
    pub version: String,
    pub major: i64,
    pub minor: i64,
    /// 该迭代对应的增量需求原文（dev-1.0 为顶层需求）。
    pub requirement: String,
    /// 评审对该次需求的梳理结论。
    pub analysis: String,
    /// pending / running / completed / failed / terminated
    pub status: String,
    pub start_time: String,
    pub end_time: String,
    /// -1 表示该迭代尚未统计 token（预留字段，界面显示“未统计”）。
    pub token_total: i64,
    /// 派生字段：该迭代下的子任务总数 / 已完成数。
    pub subtask_total: i64,
    pub subtask_completed: i64,
    /// 派生字段：耗时（秒）；未开始时为 0。
    pub duration_sec: i64,
    pub create_time: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GlobalReviewIssue {
    pub subtask_id: Option<i64>,
    pub subtask_name: String,
    pub feedback: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GlobalReviewRecord {
    /// 第几次任务迭代（首次完成=1；每次追加需求后 +1），用于区分多轮全局评审。
    #[serde(default = "default_review_cycle")]
    pub cycle: i64,
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
    /// 任务级线性工作流：按顺序排列的 Agent id。
    #[serde(default)]
    pub workflow_agent_ids: Vec<i64>,
    pub module_max_round: i64,
    pub global_max_round: i64,
    pub global_contract: String,
    pub task_phase: String, // module_dev / global_review
    pub status: String, // pending / planning / running / needs_human / paused / terminated / completed / failed
    pub work_dir: String,
    pub current_global_round: i64,
    /// 当前任务迭代轮次：首次任务=1，每次完成后追加需求再 +1。
    #[serde(default = "default_review_cycle")]
    pub global_review_cycle: i64,
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
    /// 执行模式：immediate 立即执行 / later 稍后执行 / scheduled 定时执行。
    #[serde(default = "default_execution_mode")]
    pub execution_mode: String,
    /// 定时执行窗口（本地时间 HH:MM）。
    #[serde(default = "default_schedule_start")]
    pub schedule_start: String,
    #[serde(default = "default_schedule_end")]
    pub schedule_end: String,
    #[serde(default = "default_true")]
    pub schedule_enabled: bool,
    /// 人工手动运行定时任务时忽略窗口；自动调度会清零。
    #[serde(default)]
    pub schedule_override: bool,
    /// 已请求启动但受并发上限限制，等待调度器启动。
    #[serde(default)]
    pub start_requested: bool,
    /// 暂停原因：manual / waiting_window / concurrency / terminated。
    #[serde(default)]
    pub pause_reason: String,
    /// 已完成任务追加的新需求；评审梳理期间暂存，落成子任务后清空。
    #[serde(default)]
    pub followup_requirement: String,
    /// 追加需求模式：auto 自动判断 / single 强制单模块快改。
    #[serde(default = "default_followup_mode")]
    pub followup_mode: String,
}

pub fn default_review_cycle() -> i64 {
    1
}

pub fn default_followup_mode() -> String {
    "auto".to_string()
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
    /// 所属迭代 id；0 表示尚未归档（旧数据迁移后会被归入 dev-1.0）。
    #[serde(default)]
    pub iteration_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskLog {
    pub id: i64,
    pub task_id: i64,
    #[serde(default)]
    pub iteration_id: i64,
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
    #[serde(default)]
    pub iteration_id: i64,
    pub subtask_id: Option<i64>,
    pub kind: String,
    pub content: String,
    pub create_time: String,
    /// 产生该事件的 Agent 展示名与角色名（系统事件为空）。
    #[serde(default)]
    pub agent_name: String,
    #[serde(default)]
    pub role_name: String,
}

/// 人工随时输入的消息，pending 状态会在下一个 Agent 步骤注入。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: i64,
    pub task_id: i64,
    pub role: String, // user / assistant
    pub content: String,
    pub create_time: String,
}

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
    /// 任务工作流（按顺序展开的 Agent + 角色信息）。
    #[serde(default)]
    pub workflow: Vec<WorkflowStep>,
    /// 全部角色类型，供前端选择/展示。
    #[serde(default)]
    pub roles: Vec<RoleType>,
    #[serde(default)]
    pub execution_events: Vec<TaskEvent>,
    #[serde(default)]
    pub human_messages: Vec<HumanMessage>,
    #[serde(default)]
    pub pending_plan: Option<PendingPlan>,
    #[serde(default)]
    pub chat_messages: Vec<ChatMessage>,
    /// 该任务的全部迭代，按版本正序（旧的在前）。
    #[serde(default)]
    pub iterations: Vec<Iteration>,
}

/// 单个迭代的详情视图：只包含该迭代范围内的子任务与日志。
#[derive(Debug, Clone, Serialize)]
pub struct IterationDetail {
    pub task: TaskMain,
    pub iteration: Iteration,
    /// 该任务全部迭代（按版本正序），供左侧迭代树切换。
    pub iterations: Vec<Iteration>,
    pub subtasks: Vec<Subtask>,
    pub logs: Vec<TaskLog>,
    pub reviewer: Option<AgentConfig>,
    pub coders: Vec<AgentConfig>,
    #[serde(default)]
    pub workflow: Vec<WorkflowStep>,
    #[serde(default)]
    pub roles: Vec<RoleType>,
    pub execution_events: Vec<TaskEvent>,
    pub human_messages: Vec<HumanMessage>,
}

/// 列表页使用的扁平结构：TaskMain 字段 + 展示用派生字段。
#[derive(Debug, Clone, Serialize)]
pub struct TaskSummary {
    #[serde(flatten)]
    pub task: TaskMain,
    pub reviewer_name: String,
    pub coder_names: Vec<String>,
    /// 工作流简图数据（角色徽章 + Agent 名）。
    #[serde(default)]
    pub workflow: Vec<WorkflowStep>,
    pub subtask_count: i64,
    pub completed_count: i64,
    /// 当前（最新）迭代版本号，如 dev-1.1。
    pub version: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewTaskInput {
    pub task_name: String,
    pub top_requirement: String,
    pub global_accept_criteria: String,
    /// 旧字段：工作流缺失时按旧逻辑兜底（兼容旧前端 / 旧数据）。
    #[serde(default)]
    pub reviewer_agent_id: i64,
    #[serde(default)]
    pub coder_agent_ids: Vec<i64>,
    /// 任务级线性工作流（按顺序），新前端只传该字段。
    #[serde(default)]
    pub workflow_agent_ids: Vec<i64>,
    pub module_max_round: i64,
    pub global_max_round: i64,
    pub work_dir: String,
    #[serde(default)]
    pub manual_review_enabled: bool,
    #[serde(default = "default_execution_mode")]
    pub execution_mode: String,
    #[serde(default = "default_schedule_start")]
    pub schedule_start: String,
    #[serde(default = "default_schedule_end")]
    pub schedule_end: String,
    #[serde(default = "default_true")]
    pub schedule_enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskConfigInput {
    pub module_max_round: i64,
    pub global_max_round: i64,
    pub manual_review_enabled: bool,
    /// 仅任务未启动（pending）时允许修改工作流；None 表示不修改。
    #[serde(default)]
    pub workflow_agent_ids: Option<Vec<i64>>,
    #[serde(default = "default_execution_mode")]
    pub execution_mode: String,
    #[serde(default = "default_schedule_start")]
    pub schedule_start: String,
    #[serde(default = "default_schedule_end")]
    pub schedule_end: String,
    #[serde(default = "default_true")]
    pub schedule_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub default_work_dir: String,
    pub llm_timeout_sec: String,
    pub dsh_timeout_sec: String,
    pub dsh_bin: String,
    pub dsh_profile: String,
    /// 任务数据、数据库、日志与模块快照的存储根目录。
    #[serde(default)]
    pub storage_root: String,
    /// 是否把运行日志/执行事件持久化到数据库；关闭时只在页面实时展示。
    #[serde(default = "default_true")]
    pub log_persistence: bool,
    /// 新建任务默认执行模式。
    #[serde(default = "default_execution_mode")]
    pub default_execution_mode: String,
    #[serde(default = "default_module_round")]
    pub default_module_max_round: String,
    #[serde(default = "default_global_round")]
    pub default_global_max_round: String,
    /// 同时运行的任务数上限。
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_tasks: String,
    /// 全局定时任务开关。
    #[serde(default = "default_true")]
    pub scheduled_enabled: bool,
    /// LLM 每分钟请求上限；0 表示不限。
    #[serde(default = "default_rate_limit")]
    pub llm_rate_limit_per_min: String,
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
            storage_root: String::new(),
            log_persistence: true,
            default_execution_mode: "later".to_string(),
            default_module_max_round: "3".to_string(),
            default_global_max_round: "2".to_string(),
            max_concurrent_tasks: "1".to_string(),
            scheduled_enabled: true,
            llm_rate_limit_per_min: "0".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StartOutcome {
    pub started: bool,
    pub message: String,
}

pub fn default_execution_mode() -> String {
    "later".to_string()
}
pub fn default_schedule_start() -> String {
    "01:00".to_string()
}
pub fn default_schedule_end() -> String {
    "06:00".to_string()
}
pub fn default_true() -> bool {
    true
}
fn default_module_round() -> String {
    "3".to_string()
}
fn default_global_round() -> String {
    "2".to_string()
}
fn default_max_concurrent() -> String {
    "1".to_string()
}
fn default_rate_limit() -> String {
    "0".to_string()
}
