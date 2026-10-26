// 与 Rust 后端 serde 结构一一对应的前端类型定义。

export type RoleType = "reviewer" | "coder";
export type TaskStatus =
  | "planning"
  | "running"
  | "needs_human"
  | "paused"
  | "completed"
  | "failed";
export type TaskPhase = "plan_approval" | "module_dev" | "global_review";
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
  role_type: RoleType;
  provider: string;
  model_name: string;
  base_url: string;
  api_key: string;
  system_prompt: string;
  temperature: number;
  create_time: string;
}

export interface AgentInput {
  id?: number;
  name: string;
  role_type: RoleType;
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
  round: number;
  passed: boolean;
  feedback: string;
  issues: GlobalReviewIssue[];
  created_at: string;
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
}

export interface TaskMain {
  id: number;
  task_name: string;
  top_requirement: string;
  global_accept_criteria: string;
  reviewer_agent_id: number;
  coder_agent_ids: number[];
  module_max_round: number;
  global_max_round: number;
  global_contract: string;
  task_phase: TaskPhase;
  status: TaskStatus;
  work_dir: string;
  current_global_round: number;
  global_review_records: GlobalReviewRecord[];
  create_time: string;
  manual_review_enabled: boolean;
  tech_stack: string;
  pending_plan_json: string;
  cancel_requested: boolean;
}

export interface TaskLog {
  id: number;
  task_id: number;
  level: string; // info / warn / error
  message: string;
  create_time: string;
}

export interface TaskEvent {
  id: number;
  task_id: number;
  subtask_id: number | null;
  kind: string;
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
  execution_events: TaskEvent[];
  human_messages: HumanMessage[];
  pending_plan: PendingPlan | null;
}

export interface TaskSummary extends TaskMain {
  reviewer_name: string;
  coder_names: string[];
  subtask_count: number;
  completed_count: number;
}

export interface NewTaskInput {
  task_name: string;
  top_requirement: string;
  global_accept_criteria: string;
  reviewer_agent_id: number;
  coder_agent_ids: number[];
  module_max_round: number;
  global_max_round: number;
  work_dir: string;
  manual_review_enabled: boolean;
}

export interface RuntimeConfigInput {
  module_max_round: number;
  global_max_round: number;
  manual_review_enabled: boolean;
}

export interface AppSettings {
  default_work_dir: string;
  llm_timeout_sec: string;
  dsh_timeout_sec: string;
  dsh_bin: string;
  dsh_profile: string;
}
