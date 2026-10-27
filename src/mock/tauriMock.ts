// 浏览器内 Tauri invoke / event mock：只为 UI 视觉回归提供稳定的假数据。
import type {
  AgentConfig,
  Iteration,
  IterationDetail,
  TaskDetail,
  TaskEvent,
  TaskLog,
  TaskSummary
} from "../types";

const now = "2026-10-28 04:00:00";

const reviewer: AgentConfig = {
  id: 1,
  name: "技术总监",
  role_type: "reviewer",
  provider: "openai",
  model_name: "gpt-5-codex",
  base_url: "",
  api_key: "",
  system_prompt: "",
  temperature: 0.3,
  create_time: now
};
const coder: AgentConfig = {
  id: 2,
  name: "开发工程师",
  role_type: "coder",
  provider: "openai",
  model_name: "gpt-5-codex",
  base_url: "",
  api_key: "",
  system_prompt: "",
  temperature: 0.2,
  create_time: now
};

const iterations: Iteration[] = [
  {
    id: 11,
    task_id: 1,
    version: "dev-1.0",
    major: 1,
    minor: 0,
    requirement: "做一个日历应用：支持新建日程、按天查看、拖拽调整时间。",
    analysis: "拆成 3 个模块：日历骨架、新建日程、拖拽交互。",
    status: "completed",
    start_time: "2026-10-27 09:00:00",
    end_time: "2026-10-27 10:12:00",
    token_total: 48260,
    subtask_total: 3,
    subtask_completed: 3,
    duration_sec: 4320,
    create_time: "2026-10-27 08:59:00"
  },
  {
    id: 12,
    task_id: 1,
    version: "dev-1.1",
    major: 1,
    minor: 1,
    requirement: "给日历页增加“导出当日计划为 Markdown”的按钮，并在空数据时给出提示。",
    analysis: "简单改动，只需要一个模块。",
    status: "running",
    start_time: "2026-10-28 03:40:00",
    end_time: "",
    token_total: 3184,
    subtask_total: 1,
    subtask_completed: 0,
    duration_sec: 1380,
    create_time: "2026-10-28 03:39:00"
  }
];

const subtasks = [
  {
    id: 101,
    task_id: 1,
    subtask_name: "日历视图骨架与数据模型",
    sort_index: 1,
    accept_criteria: "月/周视图可切换；日程数据结构包含标题、开始时间、结束时间。",
    assigned_coder_id: 2,
    current_round: 2,
    max_round: 3,
    status: "completed" as const,
    current_stage: "done" as const,
    summary_content:
      "# 模块名称\n日历视图骨架与数据模型\n\n## 技术栈\nVue 3 + TypeScript + 原生 CSS Grid\n\n## 接口与数据结构\n`Schedule { id, title, start, end, color }`\n\n## 验收自测\n- 月/周视图切换通过\n- 空数据占位图正常",
    summary_file_path: "/tmp/mock/summary.md",
    iteration_records: [
      {
        round: 1,
        type: "code_review",
        coder_agent_id: 2,
        diff: "diff --git a/src/Calendar.vue ...",
        review_feedback: "日期格子没有处理跨月日期，请补充 prevMonth/nextMonth 的灰显逻辑。",
        review_decision: "reject",
        summary_content: "",
        created_at: "2026-10-27 09:24:00"
      },
      {
        round: 2,
        type: "code_review",
        coder_agent_id: 2,
        diff: "diff --git a/src/Calendar.vue ...",
        review_feedback: "跨月日期已灰显，结构与契约一致，通过。",
        review_decision: "pass",
        summary_content: "",
        created_at: "2026-10-27 09:41:00"
      }
    ],
    design_content:
      "# 模块设计方案：日历视图骨架\n\n## 目标\n搭建可切换的月/周视图，并定义日程数据结构。\n\n## 技术选型\n- Vue 3 `<script setup>` + TypeScript\n- CSS Grid 布局，不引入第三方日历库\n\n## 数据结构\n```ts\ninterface Schedule { id: string; title: string; start: string; end: string; color?: string }\n```\n\n## 边界与约束\n- 所有时间统一 ISO 字符串\n- 跨月日期需要灰显且不可点击\n",
    iteration_id: 11
  },
  {
    id: 102,
    task_id: 1,
    subtask_name: "新建日程表单与校验",
    sort_index: 2,
    accept_criteria: "标题必填；结束时间必须晚于开始时间；提交后列表即时刷新。",
    assigned_coder_id: 2,
    current_round: 1,
    max_round: 3,
    status: "completed" as const,
    current_stage: "done" as const,
    summary_content:
      "# 模块名称\n新建日程表单与校验\n\n## 技术栈\nVue 3 + 自研表单校验\n\n## 验收自测\n- 标题为空时按钮禁用\n- 结束早于开始时报错",
    summary_file_path: "/tmp/mock/summary2.md",
    iteration_records: [],
    design_content:
      "# 模块设计方案：新建日程表单\n\n## 目标\n提供新建日程弹窗，含必填与时间区间校验。\n\n## 交互\n- 点击「＋ 新建」打开弹窗\n- 提交成功后关闭弹窗并刷新列表\n",
    iteration_id: 11
  },
  {
    id: 103,
    task_id: 1,
    subtask_name: "拖拽调整日程时间",
    sort_index: 3,
    accept_criteria: "拖拽后时间按 15 分钟吸附；跨天拖拽有提示。",
    assigned_coder_id: 2,
    current_round: 1,
    max_round: 3,
    status: "completed" as const,
    current_stage: "done" as const,
    summary_content: "# 模块名称\n拖拽调整日程时间\n\n## 技术栈\n原生 Pointer Events\n",
    summary_file_path: "/tmp/mock/summary3.md",
    iteration_records: [],
    design_content: "# 模块设计方案：拖拽交互\n\n## 目标\n支持拖拽日程卡片调整时间，15 分钟吸附。\n",
    iteration_id: 11
  },
  {
    id: 104,
    task_id: 1,
    subtask_name: "导出当日计划为 Markdown",
    sort_index: 4,
    accept_criteria: "点击导出按钮下载 .md 文件；空数据时给出提示。",
    assigned_coder_id: 2,
    current_round: 1,
    max_round: 3,
    status: "in_progress" as const,
    current_stage: "code_review" as const,
    summary_content: "",
    summary_file_path: "",
    iteration_records: [],
    design_content:
      "# 模块设计方案：导出当日计划\n\n## 目标\n在日历工具栏增加「导出 Markdown」按钮，把当天日程导出为 .md。\n\n## 技术选型\n- 复用现有 `Schedule` 数据结构\n- 使用 Blob + a[download] 触发下载\n\n## 边界\n- 空数据时提示「当天没有日程」，不生成文件\n",
    iteration_id: 12
  }
];

const events: TaskEvent[] = [
  {
    id: 1,
    task_id: 1,
    iteration_id: 11,
    subtask_id: null,
    kind: "agent_message:reviewer",
    content:
      "收到需求，先梳理技术栈与全局契约：\n1. Vue 3 + TypeScript + Vite\n2. 时间统一使用 ISO 字符串\n3. 组件间只通过 props / emits 通信，不引入全局状态库\n\n拆解为 3 个模块：日历骨架、新建日程、拖拽交互。",
    create_time: "2026-10-27 09:02:00"
  },
  {
    id: 2,
    task_id: 1,
    iteration_id: 11,
    subtask_id: 101,
    kind: "module_design",
    content: subtasks[0].design_content,
    create_time: "2026-10-27 09:06:00"
  },
  {
    id: 3,
    task_id: 1,
    iteration_id: 11,
    subtask_id: 101,
    kind: "tool_call",
    content: JSON.stringify({ tool: "read_file", input: { path: "src/App.vue" } }),
    create_time: "2026-10-27 09:08:00"
  },
  {
    id: 4,
    task_id: 1,
    iteration_id: 11,
    subtask_id: 101,
    kind: "tool_result",
    content: JSON.stringify({ status: "completed", result: "<template>...</template>" }),
    create_time: "2026-10-27 09:08:20"
  },
  {
    id: 5,
    task_id: 1,
    iteration_id: 11,
    subtask_id: 101,
    kind: "agent_message:coder",
    content:
      "已完成日历骨架：新增 CalendarView / WeekView 两个组件，实现了跨月日期灰显；数据结构定义在 src/types.ts。等待评审。",
    create_time: "2026-10-27 09:22:00"
  },
  {
    id: 6,
    task_id: 1,
    iteration_id: 11,
    subtask_id: 101,
    kind: "git_diff",
    content: "diff --git a/src/components/CalendarView.vue b/src/components/CalendarView.vue\n+<template>\n+  <div class=\"calendar-grid\">...</div>\n+</template>",
    create_time: "2026-10-27 09:23:00"
  },
  {
    id: 7,
    task_id: 1,
    iteration_id: 11,
    subtask_id: 101,
    kind: "agent_message:reviewer",
    content:
      "评审结论：不通过\n日期格子没有处理跨月日期，请补充 prevMonth/nextMonth 的灰显逻辑；另外把日期计算抽成纯函数，便于测试。",
    create_time: "2026-10-27 09:24:00"
  },
  {
    id: 8,
    task_id: 1,
    iteration_id: 11,
    subtask_id: 101,
    kind: "agent_message:coder",
    content:
      "已按意见修改：新增 `buildMonthMatrix()` 纯函数处理跨月灰显，并补充了 3 个单元测试。本轮 diff 已更新。",
    create_time: "2026-10-27 09:39:00"
  },
  {
    id: 9,
    task_id: 1,
    iteration_id: 11,
    subtask_id: 101,
    kind: "agent_message:reviewer",
    content: "评审结论：通过\n跨月日期已灰显，结构与契约一致，通过。继续下一个模块：新建日程表单与校验。",
    create_time: "2026-10-27 09:41:00"
  },
  {
    id: 10,
    task_id: 1,
    iteration_id: 11,
    subtask_id: 102,
    kind: "module_design",
    content: subtasks[1].design_content,
    create_time: "2026-10-27 09:43:00"
  },
  {
    id: 11,
    task_id: 1,
    iteration_id: 11,
    subtask_id: 102,
    kind: "summary_file",
    content: subtasks[1].summary_content,
    create_time: "2026-10-27 09:58:00"
  },
  {
    id: 12,
    task_id: 1,
    iteration_id: 11,
    subtask_id: 102,
    kind: "agent_message:reviewer",
    content: "评审结论：通过\n表单校验完整，提交后刷新逻辑正确，通过。",
    create_time: "2026-10-27 10:00:00"
  },
  {
    id: 13,
    task_id: 1,
    iteration_id: 11,
    subtask_id: null,
    kind: "agent_message:reviewer",
    content:
      "全局评审第 1 轮：通过\n全部模块已完成，跨模块命名与接口一致，无遗留问题，任务收口完成。",
    create_time: "2026-10-27 10:12:00"
  },
  {
    id: 14,
    task_id: 1,
    iteration_id: 12,
    subtask_id: null,
    kind: "requirement_queued",
    content: "任务完成后追加新需求：\n给日历页增加「导出当日计划为 Markdown」的按钮，并在空数据时给出提示。",
    create_time: "2026-10-28 03:39:00"
  },
  {
    id: 15,
    task_id: 1,
    iteration_id: 12,
    subtask_id: null,
    kind: "agent_message:reviewer",
    content:
      "追加需求梳理完成（简单改动 · 单模块）：只需要在日历工具栏增加导出按钮，不改动已完成模块。\n本次新增 1 个子任务：导出当日计划为 Markdown。",
    create_time: "2026-10-28 03:41:00"
  },
  {
    id: 16,
    task_id: 1,
    iteration_id: 12,
    subtask_id: 104,
    kind: "module_design",
    content: subtasks[3].design_content,
    create_time: "2026-10-28 03:44:00"
  },
  {
    id: 17,
    task_id: 1,
    iteration_id: 12,
    subtask_id: 104,
    kind: "tool_call",
    content: JSON.stringify({ tool: "write_file", input: { path: "src/components/ExportButton.vue" } }),
    create_time: "2026-10-28 03:52:00"
  },
  {
    id: 18,
    task_id: 1,
    iteration_id: 12,
    subtask_id: 104,
    kind: "agent_message:coder",
    content:
      "导出按钮已实现并接入工具栏：空数据时弹出提示，有数据时生成 Markdown 并触发下载。请求评审。",
    create_time: "2026-10-28 04:01:00"
  }
];

const logs: TaskLog[] = [
  { id: 1, task_id: 1, iteration_id: 11, level: "info", message: "评审 Agent 开始拆解顶层需求、选择技术栈并生成全局契约…", create_time: "2026-10-27 09:01:00" },
  { id: 2, task_id: 1, iteration_id: 11, level: "info", message: "拆解完成：共 3 个模块，等待串行开发", create_time: "2026-10-27 09:02:10" },
  { id: 3, task_id: 1, iteration_id: 11, level: "warn", message: "代码评审不通过：子任务 #1 第 1 轮", create_time: "2026-10-27 09:24:05" },
  { id: 4, task_id: 1, iteration_id: 11, level: "info", message: "全部子任务已完成，进入【全局评审收口阶段】", create_time: "2026-10-27 10:09:00" },
  { id: 5, task_id: 1, iteration_id: 12, level: "info", message: "收到追加需求，评审开始梳理增量", create_time: "2026-10-28 03:40:00" },
  { id: 6, task_id: 1, iteration_id: 12, level: "info", message: "追加需求梳理完成：新增 1 个子任务（dev-1.1）", create_time: "2026-10-28 03:41:00" }
];

const taskMain = {
  id: 1,
  task_name: "日历应用 · 迭代开发",
  top_requirement: "做一个本地日历应用：支持新建日程、按天查看、拖拽调整时间。\n后续追加：导出当日计划为 Markdown。",
  global_accept_criteria: "所有日程可新增、查看、拖拽调整；导出结果为合法 Markdown；无控制台报错。",
  reviewer_agent_id: 1,
  coder_agent_ids: [2],
  module_max_round: 3,
  global_max_round: 2,
  global_contract:
    "# 全局契约\n1. 时间统一 ISO 字符串\n2. 组件间只通过 props / emits 通信\n3. 样式使用 CSS 变量，支持浅色/深色主题\n4. 所有用户可见文案使用中文",
  task_phase: "module_dev" as const,
  status: "running" as const,
  work_dir: "/Users/demo/Projects/calendar-app",
  current_global_round: 0,
  global_review_cycle: 2,
  global_review_records: [
    {
      cycle: 1,
      round: 1,
      passed: true,
      feedback: "全部模块已完成，跨模块命名与接口一致，无遗留问题。",
      issues: [],
      created_at: "2026-10-27 10:12:00"
    }
  ],
  create_time: "2026-10-27 08:59:00",
  manual_review_enabled: false,
  tech_stack:
    "# 技术栈\n- 前端：Vue 3 + TypeScript + Vite\n- 状态：组件本地状态 + props/emits\n- 样式：原生 CSS + CSS 变量\n- 测试：Vitest\n- 构建：Vite build\n\n## 依赖与接口约定\n\n| 模块 | 接口 | 约定 |\n| --- | --- | --- |\n| 日程 CRUD | `/api/events` | 时间统一 ISO 字符串，前端按本地时区展示；分页参数 page 从 1 开始，size 默认 20 且最大 100 |\n| 导出 | `/api/export/markdown` | 返回 `text/markdown`，文件名形如 `2026-10-27-plan.md`，导出失败返回统一错误结构 |\n\n> 超长标识符也不会把面板顶破：`ApiResponse<PagedResult<CalendarEventVO>>`",
  pending_plan_json: "",
  cancel_requested: false,
  execution_mode: "later" as const,
  schedule_start: "01:00",
  schedule_end: "06:00",
  schedule_enabled: true,
  schedule_override: false,
  start_requested: false,
  pause_reason: "",
  followup_requirement: "",
  followup_mode: "auto" as const
};

const summaries: TaskSummary[] = [
  {
    ...taskMain,
    reviewer_name: reviewer.name,
    coder_names: [coder.name],
    subtask_count: 4,
    completed_count: 3,
    version: "dev-1.1"
  },
  {
    id: 2,
    task_name: "待启动的博客系统",
    top_requirement: "做一个本地 Markdown 博客系统。",
    global_accept_criteria: "可写、可读、可构建。",
    reviewer_agent_id: 1,
    coder_agent_ids: [2],
    module_max_round: 3,
    global_max_round: 2,
    global_contract: "",
    task_phase: "module_dev" as const,
    status: "pending" as const,
    work_dir: "/Users/demo/Projects/blog",
    current_global_round: 0,
    global_review_cycle: 1,
    global_review_records: [],
    create_time: "2026-10-28 02:00:00",
    manual_review_enabled: true,
    tech_stack: "",
    pending_plan_json: "",
    cancel_requested: false,
    execution_mode: "scheduled" as const,
    schedule_start: "01:00",
    schedule_end: "06:00",
    schedule_enabled: true,
    schedule_override: false,
    start_requested: false,
    pause_reason: "",
    followup_requirement: "",
    followup_mode: "auto" as const,
    reviewer_name: reviewer.name,
    coder_names: [coder.name],
    subtask_count: 0,
    completed_count: 0,
    version: "dev-1.0"
  }
];

function mockScene() {
  try {
    return new URLSearchParams(location.search).get("scene") ?? "";
  } catch {
    return "";
  }
}

function planApprovalDetail(): TaskDetail {
  const task = {
    ...taskMain,
    task_phase: "plan_approval" as const,
    status: "needs_human" as const,
    global_contract: "",
    tech_stack: "",
    pending_plan_json: "{}"
  };
  return {
    task,
    subtasks: [],
    logs: [],
    reviewer,
    coders: [coder],
    execution_events: [],
    human_messages: [],
    pending_plan: {
      tech_stack:
        "# 技术栈\n- 前端：Vue 3 + TypeScript + Vite\n- 状态：组件本地状态\n- 样式：原生 CSS 变量\n- 测试：Vitest\n- 构建：Vite build",
      global_contract:
        "# 全局契约\n1. 时间统一 ISO 字符串\n2. 组件间只通过 props / emits 通信\n3. 所有用户可见文案使用中文\n4. 样式使用 CSS 变量，支持浅色/深色主题\n\n| 约定 | 示例 |\n| --- | --- |\n| 事件名 | `schedule:create` |\n| 错误码 | `E_INVALID_TIME` |",
      subtasks: [
        { name: "日历视图骨架与数据模型", accept_criteria: "- 月/周视图可切换\n- 数据结构含 `title/start/end`" },
        { name: "新建日程表单与校验", accept_criteria: "标题必填；结束时间晚于开始时间" },
        { name: "拖拽调整日程时间", accept_criteria: "15 分钟吸附；跨天有提示" }
      ]
    },
    chat_messages: [],
    iterations: [
      {
        ...iterations[0],
        status: "pending",
        start_time: "",
        end_time: "",
        subtask_total: 0,
        subtask_completed: 0,
        duration_sec: 0
      }
    ]
  };
}

function taskDetail(iterationId: number | null): TaskDetail {
  if (mockScene() === "plan") return planApprovalDetail();
  // 已完成任务：用来看「追加需求，继续迭代」固定面板
  const task =
    mockScene() === "completed"
      ? { ...taskMain, status: "completed" as const, task_phase: "module_dev" as const }
      : taskMain;
  return {
    task,
    subtasks,
    logs: iterationId ? logs.filter((l) => l.iteration_id === iterationId) : logs,
    reviewer,
    coders: [coder],
    execution_events: iterationId
      ? events.filter((e) => e.iteration_id === iterationId)
      : events,
    human_messages: [],
    pending_plan: null,
    chat_messages: [],
    iterations
  };
}

function iterationDetail(iterationId: number): IterationDetail {
  if (mockScene() === "plan") {
    const detail = planApprovalDetail();
    return {
      task: detail.task,
      iteration: detail.iterations[0],
      iterations: detail.iterations,
      subtasks: [],
      logs: [],
      reviewer,
      coders: [coder],
      execution_events: [],
      human_messages: []
    };
  }
  const iteration = iterations.find((i) => i.id === iterationId) ?? iterations[0];
  return {
    task: taskMain,
    iteration,
    iterations,
    subtasks: subtasks.filter((s) => s.iteration_id === iterationId),
    logs: logs.filter((l) => l.iteration_id === iterationId),
    reviewer,
    coders: [coder],
    execution_events: events.filter((e) => e.iteration_id === iterationId),
    human_messages: []
  };
}

let callbackId = 0;
const callbacks: Record<number, (payload: unknown) => void> = {};

export function installMock() {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {
    invoke: async (cmd: string, args: Record<string, unknown> = {}) => {
      switch (cmd) {
        case "list_agents":
          return [reviewer, coder];
        case "list_tasks":
          return summaries;
        case "get_task_detail":
          return taskDetail(null);
        case "list_iterations":
          return iterations;
        case "get_iteration_detail":
          return iterationDetail(Number(args.iterationId ?? iterations[1].id));
        case "list_task_events":
          return events;
        case "get_settings":
          return {
            default_work_dir: "/Users/demo/Projects",
            llm_timeout_sec: "180",
            dsh_timeout_sec: "1800",
            dsh_bin: "dsh",
            dsh_profile: "headless",
            storage_root: "/Users/demo/.agentcrew",
            log_persistence: true,
            default_execution_mode: "later",
            default_module_max_round: "3",
            default_global_max_round: "2",
            max_concurrent_tasks: "1",
            scheduled_enabled: true,
            llm_rate_limit_per_min: "0"
          };
        case "plugin:event|listen": {
          const id = ++callbackId;
          callbacks[id] = () => {};
          return id;
        }
        case "plugin:event|unlisten":
          return null;
        case "plugin:dialog|open":
        case "plugin:dialog|save":
          return null;
        default:
          return null;
      }
    },
    transformCallback: (cb: (payload: unknown) => void) => {
      const id = ++callbackId;
      callbacks[id] = cb;
      return id;
    },
    convertFileSrc: (p: string) => p,
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } }
  };
}
