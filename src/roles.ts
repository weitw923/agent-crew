// 角色 / 能力标签的前端展示工具（V2.5）。纯展示，不参与业务判断。
import type { AgentConfig, Capability, RoleType, WorkflowStep } from "./types";

export const CAPABILITIES: Capability[] = ["requirement", "plan", "code", "review"];

export const CAPABILITY_LABELS: Record<Capability, string> = {
  requirement: "需求梳理",
  plan: "方案设计",
  code: "编码实现",
  review: "评审验收"
};

export const CAPABILITY_HINTS: Record<Capability, string> = {
  requirement: "负责梳理需求 / 拆分增量，把需求交给方案 Agent",
  plan: "负责拆解方案、技术栈、全局契约与模块设计",
  code: "负责写代码（通过 dsh 在工作目录执行）",
  review: "负责代码评审、摘要校验与全局验收"
};

/** 角色颜色预设（主题无关，徽章使用文字色 + 半透明底）。 */
export const ROLE_COLOR_PRESETS = [
  "#9b7bff",
  "#5aa9ff",
  "#ffc857",
  "#3ddc97",
  "#ff8f6b",
  "#ff6b9a",
  "#8b97aa",
  "#6b8cff"
];

export function roleName(role: Pick<RoleType, "name" | "key"> | null | undefined): string {
  return role?.name?.trim() || role?.key || "未知角色";
}

export function capabilityText(caps: Capability[] | undefined): string {
  if (!caps?.length) return "无能力标签";
  return caps.map((c) => CAPABILITY_LABELS[c] ?? c).join(" · ");
}

/** 角色徽章内联样式：颜色来自角色配置，浅色/深色主题都只调透明度。 */
export function roleBadgeStyle(color: string | undefined): Record<string, string> {
  if (!color) return {};
  return {
    color,
    borderColor: `${color}66`,
    background: `${color}1a`
  };
}

/** 工作流简图文本：产品经理「小P」→ 评审者「老周」→ 编码者「阿码」 */
export function workflowSummary(steps: WorkflowStep[] | undefined): string {
  if (!steps?.length) return "未配置工作流";
  return steps.map((s) => `${s.role_name || s.role_key}「${s.agent_name}」`).join(" → ");
}

/** 工作流已覆盖的能力（按出现顺序去重）。 */
export function workflowCapabilities(agents: AgentConfig[]): Capability[] {
  const caps: Capability[] = [];
  for (const agent of agents) {
    for (const cap of agent.role_capabilities ?? []) {
      if (!caps.includes(cap)) caps.push(cap);
    }
  }
  return caps;
}

/** 工作流缺失的必需能力（方案 / 编码 / 评审）。 */
export function missingWorkflowCapabilities(agents: AgentConfig[]): Capability[] {
  const caps = workflowCapabilities(agents);
  return (["plan", "code", "review"] as Capability[]).filter((cap) => !caps.includes(cap));
}

/** 工作流校验：返回中文错误文案，通过时返回空串。 */
export function validateWorkflowAgents(agents: AgentConfig[]): string {
  if (agents.length < 2) return "工作流至少需要 2 个 Agent（例如 评审者 → 编码者）";
  if (agents.length > 6) return "工作流最多支持 6 个 Agent";
  const missing = missingWorkflowCapabilities(agents);
  if (missing.length) {
    return `工作流缺少「${missing.map((c) => CAPABILITY_LABELS[c]).join("、")}」能力的 Agent`;
  }
  return "";
}

/** 默认工作流：需求梳理角色 → 方案角色 → 编码角色（去重）。 */
export function defaultWorkflowAgents(list: AgentConfig[]): AgentConfig[] {
  const picks: AgentConfig[] = [];
  const push = (agent?: AgentConfig) => {
    if (agent && !picks.some((a) => a.id === agent.id)) picks.push(agent);
  };
  const has = (agent: AgentConfig, cap: Capability) =>
    (agent.role_capabilities ?? []).includes(cap);
  push(
    list.find(
      (a) => has(a, "requirement") && !has(a, "plan") && !has(a, "code") && !has(a, "review")
    )
  );
  push(list.find((a) => has(a, "plan")));
  push(list.find((a) => has(a, "code")));
  push(list.find((a) => has(a, "review")));
  return picks;
}

/** 按能力推导旧的 reviewer / coder 字段（与服务端逻辑一致，兼容回滚）。 */
export function deriveLegacyBindings(agents: AgentConfig[]): {
  reviewer_agent_id: number;
  coder_agent_ids: number[];
} {
  const has = (agent: AgentConfig, cap: Capability) =>
    (agent.role_capabilities ?? []).includes(cap);
  const reviewer =
    agents.find((a) => has(a, "plan")) ??
    agents.find((a) => has(a, "review")) ??
    agents[0];
  return {
    reviewer_agent_id: reviewer?.id ?? 0,
    coder_agent_ids: agents.filter((a) => has(a, "code")).map((a) => a.id)
  };
}
