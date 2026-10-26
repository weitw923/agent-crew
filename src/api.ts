import { invoke } from "@tauri-apps/api/core";
import type {
  AgentConfig,
  AgentInput,
  AppSettings,
  NewTaskInput,
  RuntimeConfigInput,
  TaskDetail,
  TaskMain,
  TaskSummary
} from "./types";

// ---------------- Agent 配置 ----------------
export const listAgents = () => invoke<AgentConfig[]>("list_agents");
export const saveAgent = (input: AgentInput) => invoke<AgentConfig>("save_agent", { input });
export const deleteAgent = (id: number) => invoke<void>("delete_agent", { id });
export const exportAgents = (path: string) => invoke<number>("export_agents", { path });
export const importAgents = (path: string) => invoke<number>("import_agents", { path });

// ---------------- 任务 ----------------
export const listTasks = () => invoke<TaskSummary[]>("list_tasks");
export const createTask = (input: NewTaskInput) => invoke<number>("create_task", { input });
export const startTask = (taskId: number) => invoke<void>("start_task", { taskId });
export const resumeTask = (taskId: number) => invoke<void>("resume_task", { taskId });
export const getTaskDetail = (taskId: number) => invoke<TaskDetail>("get_task_detail", { taskId });
export const deleteTask = (taskId: number) => invoke<void>("delete_task", { taskId });
export const humanForcePass = (taskId: number, subtaskId: number) =>
  invoke<void>("human_force_pass", { taskId, subtaskId });
export const humanReject = (taskId: number, subtaskId: number, feedback: string) =>
  invoke<void>("human_reject", { taskId, subtaskId, feedback });
export const globalHumanForcePass = (taskId: number) =>
  invoke<void>("global_human_force_pass", { taskId });
export const globalHumanRework = (taskId: number, subtaskId: number, feedback: string) =>
  invoke<void>("global_human_rework", { taskId, subtaskId, feedback });

// 运行中配置 / 人工审批 / 终止 / 人工消息
export const updateTaskConfig = (taskId: number, input: RuntimeConfigInput) =>
  invoke<TaskMain>("update_task_config", { taskId, input });
export const approvePlan = (taskId: number) => invoke<void>("approve_plan", { taskId });
export const rejectPlan = (taskId: number, feedback: string) =>
  invoke<void>("reject_plan", { taskId, feedback });
export const approveDesign = (taskId: number, subtaskId: number) =>
  invoke<void>("approve_design", { taskId, subtaskId });
export const rejectDesign = (taskId: number, subtaskId: number, feedback: string) =>
  invoke<void>("reject_design", { taskId, subtaskId, feedback });
export const sendHumanMessage = (taskId: number, content: string) =>
  invoke<void>("send_human_message", { taskId, content });
export const cancelTask = (taskId: number) => invoke<void>("cancel_task", { taskId });

// ---------------- 设置 ----------------
export const getSettings = () => invoke<AppSettings>("get_settings");
export const saveSettings = (settings: AppSettings) =>
  invoke<AppSettings>("save_settings", { settings });
