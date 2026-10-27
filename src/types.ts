// 与 Rust 后端 serde 结构一一对应的前端类型定义。

/** 角色能力标签：需求梳理 / 方案设计 / 编码实现 / 评审验收。 */
export type Capability = "requirement" | "plan" | "code" | "review";

/** 角色类型实体（内置 reviewer / coder + 用户自定义角色）。 */
export interface RoleType {
  id: number;
  key: string;
  name: string;
  description: string;
  capabilities: Capability[];
  color: string;
  is_builtin: boolean;
  sort_order: number;
  create_time: string;
  /** 使用该角色的 Agent 数量。 */
  agent_count: number;
}

export interface RoleInput {
  id?: number;
  key: string;
  name: string;
  description: string;
  capabilities: Capability[];
  color: string;
}

/** 任务工作流中的一步（后端派生的展示结构）。 */
export interface WorkflowStep {
  order: number;
  agent_id: number;
  agent_name: string;
  role_key: string;
  role_name: string;
  role_color: string;
  capabilities: Capability[];
}
export type TaskStatus =
  | "pending"
  | "planning"
  | "running"
  | "needs_human"
  | "paused"
  | "terminated"
  | "completed"
  | "failed";
export type ExecutionMode = "immediate" | "later" | "scheduled";
export type TaskPhase = "plan_approval" | "module_dev" | "global_review" | "requirement_intake";
export type FollowupMode = "auto" | "single";
export type SubtaskStatus = "waiting" | "in_progress" | "completed" | "needs_human";
export type SubtaskStage =
  | "queued"
  | "design_generation"
  | "design_approval"
  | "coding"
  | "code_review"
  | "summary_generation"
  | "summary_review"
  | "done"
  | "needs_human";

export interface AgentConfig {
  id: number;
  name: string;
  role_type: string;
  provider: string;
  model_name: string;
  base_url: string;
  api_key: string;
  system_prompt: string;
  temperature: number;
  create_time: string;
  role_name?: string;
  role_color?: string;
  role_capabilities?: Capability[];
}

export interface AgentInput {
  id?: number;
  name: string;
  role_type: string;
  provider: string;
  model_name: string;
  base_url: string;
  api_key: string;
  system_prompt: string;
  temperature: number;
}

export interface IterationRecord {
  round: number;
  type: string; // code_review / summary_review / human_reject / global_rework
  coder_agent_id?: number | null;
  diff: string;
  review_feedback: string;
  review_decision: string; // pass / reject
  summary_content: string;
  created_at: string;
}

export interface GlobalReviewIssue {
  subtask_id?: number | null;
  subtask_name: string;
  feedback: string;
}

export interface GlobalReviewRecord {
  cycle: number;
  round: number;
  passed: boolean;
  feedback: string;
  issues: GlobalReviewIssue[];
  created_at: string;
}

/// 任务迭代（版本）：dev-1.0 / dev-1.1 / dev-2.0 …
export type IterationStatus = "pending" | "running" | "completed" | "failed" | "terminated";

export interface Iteration {
  id: number;
  task_id: number;
  /** 落定后的版本号；需求梳理完成前为空字符串（界面显示“待定”）。 */
  version: string;
  major: number;
  minor: number;
  requirement: string;
  analysis: string;
  status: IterationStatus;
  start_time: string;
  end_time: string;
  /** -1 表示尚未统计 token（当前版本只预留字段）。 */
  token_total: number;
  subtask_total: number;
  subtask_completed: number;
  /** 耗时（秒）。 */
  duration_sec: number;
  create_time: string;
}

/** 右侧预览面板的内容。 */
export interface PreviewPayload {
  kind:
    | "design"
    | "tech_stack"
    | "contract"
    | "requirement"
    | "criteria"
    | "diff"
    | "summary"
    | "review"
    | "event"
    | "empty";
  title: string;
  content: string;
  /** 来源描述，例如「dev-1.1 · #2 模块名 · 第 2 轮」。 */
  source?: string;
  time?: string;
}

export interface Subtask {
  id: number;
  task_id: number;
  subtask_name: string;
  sort_index: number;
  accept_criteria: string;
  assigned_coder_id: number | null;
  current_round: number;
  max_round: number;
  status: SubtaskStatus;
  current_stage: SubtaskStage;
  summary_content: string;
  summary_file_path: string;
  iteration_records: IterationRecord[];
  design_content: string;
  /** 所属迭代 id；0 表示未归档。 */
  iteration_id: number;
}

export interface TaskMain {
  id: number;
  task_name: string;
  top_requirement: string;
  global_accept_criteria: string;
  reviewer_agent_id: number;
  coder_agent_ids: number[];
  /** 任务级线性工作流（按顺序的 Agent id）。 */
  workflow_agent_ids?: number[];
  module_max_round: number;
  global_max_round: number;
  global_contract: string;
  task_phase: TaskPhase;
  status: TaskStatus;
  work_dir: string;
  current_global_round: number;
  global_review_cycle: number;
  global_review_records: GlobalReviewRecord[];
  create_time: string;
  manual_review_enabled: boolean;
  tech_stack: string;
  pending_plan_json: string;
  cancel_requested: boolean;
  execution_mode: ExecutionMode;
  schedule_start: string;
  schedule_end: string;
  schedule_enabled: boolean;
  schedule_override: boolean;
  start_requested: boolean;
  pause_reason: string;
  followup_requirement: string;
  followup_mode: FollowupMode;
}

export interface TaskLog {
  id: number;
  task_id: number;
  iteration_id: number;
  level: string; // info / warn / error
  message: string;
  create_time: string;
}

export interface TaskEvent {
  id: number;
  task_id: number;
  iteration_id: number;
  subtask_id: number | null;
  kind: string;
  content: string;
  create_time: string;
  /** 产生该事件的 Agent 与角色名（系统事件为空）。 */
  agent_name?: string;
  role_name?: string;
}

export interface ChatMessage {
  id: number;
  task_id: number;
  role: "user" | "assistant";
  content: string;
  create_time: string;
}

export interface HumanMessage {
  id: number;
  task_id: number;
  content: string;
  status: "pending" | "consumed";
  create_time: string;
}

export interface PendingPlanItem {
  name: string;
  accept_criteria: string;
}

export interface PendingPlan {
  tech_stack: string;
  global_contract: string;
  subtasks: PendingPlanItem[];
}

export interface TaskDetail {
  task: TaskMain;
  subtasks: Subtask[];
  logs: TaskLog[];
  reviewer: AgentConfig | null;
  coders: AgentConfig[];
  workflow?: WorkflowStep[];
  roles?: RoleType[];
  execution_events: TaskEvent[];
  human_messages: HumanMessage[];
  pending_plan: PendingPlan | null;
  chat_messages: ChatMessage[];
  /** 该任务的全部迭代（按版本正序）。 */
  iterations: Iteration[];
}

/** 单个迭代的详情视图。 */
export interface IterationDetail {
  task: TaskMain;
  iteration: Iteration;
  iterations: Iteration[];
  subtasks: Subtask[];
  logs: TaskLog[];
  reviewer: AgentConfig | null;
  coders: AgentConfig[];
  workflow?: WorkflowStep[];
  roles?: RoleType[];
  execution_events: TaskEvent[];
  human_messages: HumanMessage[];
}

export interface TaskSummary extends TaskMain {
  reviewer_name: string;
  coder_names: string[];
  workflow: WorkflowStep[];
  subtask_count: number;
  completed_count: number;
  /** 当前（最新）迭代版本号。 */
  version: string;
}

export interface NewTaskInput {
  task_name: string;
  top_requirement: string;
  global_accept_criteria: string;
  reviewer_agent_id: number;
  coder_agent_ids: number[];
  workflow_agent_ids: number[];
  module_max_round: number;
  global_max_round: number;
  work_dir: string;
  manual_review_enabled: boolean;
  execution_mode: ExecutionMode;
  schedule_start: string;
  schedule_end: string;
  schedule_enabled: boolean;
}

export interface RuntimeConfigInput {
  module_max_round: number;
  global_max_round: number;
  manual_review_enabled: boolean;
  /** 仅任务未启动时允许修改。 */
  workflow_agent_ids?: number[];
  execution_mode: ExecutionMode;
  schedule_start: string;
  schedule_end: string;
  schedule_enabled: boolean;
}

export interface AppSettings {
  default_work_dir: string;
  llm_timeout_sec: string;
  dsh_timeout_sec: string;
  dsh_bin: string;
  dsh_profile: string;
  storage_root: string;
  log_persistence: boolean;
  default_execution_mode: ExecutionMode;
  default_module_max_round: string;
  default_global_max_round: string;
  max_concurrent_tasks: string;
  scheduled_enabled: boolean;
  llm_rate_limit_per_min: string;
}

export interface StartOutcome {
  started: boolean;
  message: string;
}
