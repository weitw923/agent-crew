<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import ActionDock from "../components/ActionDock.vue";
import ChatPanel from "../components/ChatPanel.vue";
import FollowUpModal from "../components/FollowUpModal.vue";
import IterationList from "../components/IterationList.vue";
import IterationTree from "../components/IterationTree.vue";
import PreviewPanel from "../components/PreviewPanel.vue";
import RequirementsModal from "../components/RequirementsModal.vue";
import RuntimeConfigModal from "../components/RuntimeConfigModal.vue";
import TimelinePanel from "../components/TimelinePanel.vue";
import type {
  IterationDetail,
  PreviewPayload,
  RuntimeConfigInput,
  Subtask,
  TaskDetail,
  TaskEvent,
  TaskMain
} from "../types";
import {
  approveDesign,
  approvePlan,
  extendSubtaskRounds,
  getIterationDetail,
  getTaskDetail,
  listTaskEvents,
  globalHumanForcePass,
  globalHumanRework,
  humanForcePass,
  humanReject,
  pauseTask,
  rejectDesign,
  rejectPlan,
  resumeTask,
  sendHumanMessage,
  startTask,
  terminateTask,
  updateTaskConfig
} from "../api";
import { errorMessage, showToast } from "../toast";

const props = defineProps<{ taskId: number | null }>();
const emit = defineEmits<{
  (e: "back"): void;
  (e: "open-task", id: number): void;
}>();

const detail = ref<TaskDetail | null>(null);
const iterDetail = ref<IterationDetail | null>(null);
const loading = ref(false);
const actionBusy = ref(false);

/** 任务详情第一屏：按版本倒序的迭代列表；点击某个迭代后才进入工作台。 */
const showIterationList = ref(true);
const activeIterationId = ref<number | null>(null);
const selectedSubtaskId = ref<number | null>(null);

/** 右栏预览内容。 */
const preview = ref<PreviewPayload | null>(null);
/** 中栏时间线只看某个子任务（左栏「查看」）。 */
const focusSubtaskId = ref<number | null>(null);
/** 工具调用默认收起。 */
const showTools = ref(false);

const showReject = ref(false);
const rejectText = ref("");
const showPlanReject = ref(false);
const planRejectText = ref("");
const showDesignReject = ref(false);
const designRejectText = ref("");
const showGlobalRework = ref(false);
const globalTargetId = ref<number>(0);
const globalFeedback = ref("");

const showConfig = ref(false);
const showChat = ref(false);
const showRequirements = ref(false);
const showFollowup = ref(false);

const humanMessage = ref("");
const allEvents = ref<TaskEvent[]>([]);
const showConsole = ref(false);
const logBox = ref<HTMLElement | null>(null);

let unlisten: UnlistenFn | null = null;
let eventUnlisten: UnlistenFn | null = null;
let pollTimer: number | null = null;
let reloadTimer: number | null = null;

const task = computed(() => detail.value?.task ?? null);
const iterations = computed(() => detail.value?.iterations ?? []);
const currentIteration = computed(
  () => iterations.value.find((i) => i.id === activeIterationId.value) ?? null
);
/** 工作台作用域：只包含当前迭代的子任务 / 事件 / 日志。 */
const subtasks = computed<Subtask[]>(() => iterDetail.value?.subtasks ?? []);
const executionEvents = computed<TaskEvent[]>(() => iterDetail.value?.execution_events ?? []);
const logs = computed(() => iterDetail.value?.logs ?? []);
const isLatestIteration = computed(() => {
  if (!iterations.value.length) return true;
  const ordered = [...iterations.value].sort(
    (a, b) => a.major - b.major || a.minor - b.minor || a.id - b.id
  );
  return ordered[ordered.length - 1].id === activeIterationId.value;
});
const isGlobal = computed(() => task.value?.task_phase === "global_review");
const isPlanApproval = computed(() => task.value?.task_phase === "plan_approval");
const isCompleted = computed(() => task.value?.status === "completed");
const isRunning = computed(
  () => task.value?.status === "running" || task.value?.status === "planning"
);
const selected = computed(
  () => subtasks.value.find((s) => s.id === selectedSubtaskId.value) ?? null
);
const designGateSubtask = computed(
  () => subtasks.value.find((s) => s.current_stage === "design_approval") ?? null
);
const isDesignAwaiting = computed(() => !!designGateSubtask.value);
const pendingMessages = computed(
  () => (detail.value?.human_messages ?? []).filter((m) => m.status === "pending")
);
const pendingPlan = computed(() => detail.value?.pending_plan ?? null);
const canRun = computed(
  () =>
    !!task.value &&
    (task.value.status === "pending" || task.value.status === "failed") &&
    !isPlanApproval.value &&
    !isDesignAwaiting.value
);
const canResume = computed(
  () =>
    !!task.value &&
    !isRunning.value &&
    (task.value.status === "paused" || task.value.status === "needs_human") &&
    !isPlanApproval.value &&
    !isDesignAwaiting.value
);
const canPause = computed(
  () => !!task.value && (task.value.status === "running" || task.value.status === "planning")
);
const canTerminate = computed(
  () => !!task.value && task.value.status !== "completed" && task.value.status !== "terminated"
);
const canContinue = computed(() => !!task.value && task.value.status === "terminated");

const phaseLabel = computed(() => {
  if (task.value?.status === "completed") return "全局评审完成";
  const map: Record<string, string> = {
    module_dev: "模块开发",
    global_review: "全局评审",
    plan_approval: "方案审批",
    requirement_intake: "需求梳理"
  };
  return map[task.value?.task_phase ?? ""] ?? "模块开发";
});
const pendingHumanSubtask = computed(
  () => subtasks.value.find((s) => s.status === "needs_human") ?? null
);
const humanTarget = computed(() => {
  if (selected.value?.status === "needs_human") return selected.value;
  return pendingHumanSubtask.value;
});
const canHuman = computed(
  () =>
    !!task.value &&
    !isRunning.value &&
    task.value.status !== "completed" &&
    task.value.status !== "terminated" &&
    !!humanTarget.value &&
    !isDesignAwaiting.value
);
const humanStageLabel = computed(() => {
  const map: Record<string, string> = {
    code_review: "代码评审",
    summary_review: "摘要校验"
  };
  return map[humanTarget.value?.current_stage ?? ""] ?? humanTarget.value?.current_stage ?? "";
});
const humanActionLabel = computed(() => {
  if (humanTarget.value?.current_stage === "code_review") return "强制通过代码评审，继续生成摘要";
  if (humanTarget.value?.current_stage === "summary_review") return "强制通过摘要，完成模块";
  return "人工强制通过";
});
const humanReason = computed(() => {
  const target = humanTarget.value;
  if (!target) return "";
  const record = [...target.iteration_records]
    .reverse()
    .find((r) => r.review_feedback.trim() || r.review_decision);
  return (
    record?.review_feedback?.trim() ||
    "该模块已达到当前轮次上限，需要人工判断是否通过、追加轮次或驳回修改。"
  );
});
const statusText = computed(() => {
  const t = task.value;
  if (!t) return "";
  if (
    t.task_phase === "requirement_intake" &&
    (t.status === "planning" || t.status === "running")
  )
    return "梳理追加需求";
  if (t.status === "pending") {
    if (t.pause_reason === "concurrency") return "等待并发名额";
    if (t.execution_mode === "scheduled")
      return t.schedule_enabled ? "等待时间窗口" : "待运行（定时窗口关闭）";
    return "待运行";
  }
  if (t.status === "paused" && t.pause_reason === "waiting_window") return "等待时间窗口";
  if (t.status === "paused") return "已暂停";
  const map: Record<string, string> = {
    planning: "拆解需求中",
    running: "执行中",
    needs_human: "需要人工介入",
    terminated: "已终止",
    completed: "已完成",
    failed: "执行失败"
  };
  return map[t.status] ?? t.status;
});
const executionModeLabel = computed(() => {
  const map: Record<string, string> = {
    immediate: "立即执行",
    later: "稍后执行",
    scheduled: "定时执行"
  };
  return map[task.value?.execution_mode ?? ""] ?? task.value?.execution_mode ?? "";
});
const scheduleText = computed(() => {
  const t = task.value;
  if (!t || t.execution_mode !== "scheduled") return "";
  return `每日 ${t.schedule_start}–${t.schedule_end}${t.schedule_enabled ? "" : "（窗口关闭）"}`;
});
const phaseBanner = computed(() => {
  const t = task.value;
  if (!t) return { text: "", cls: "" };
  if (t.status === "completed")
    return { text: "✅ 当前阶段：全部完成 · 可继续追加需求", cls: "done" };
  if (
    t.task_phase === "requirement_intake" &&
    (t.status === "planning" || t.status === "running")
  )
    return { text: "🧠 当前阶段：评审正在梳理追加需求", cls: "global" };
  if (t.status === "terminated")
    return { text: "⛔ 当前阶段：任务已终止，可继续任务", cls: "human" };
  if (t.status === "failed") return { text: "❌ 当前阶段：执行失败", cls: "human" };
  if (t.status === "pending") {
    if (t.pause_reason === "concurrency")
      return { text: "⏳ 当前阶段：等待并发名额", cls: "global" };
    if (t.execution_mode === "scheduled")
      return t.schedule_enabled
        ? {
            text: `🕒 当前阶段：等待时间窗口 ${t.schedule_start}–${t.schedule_end}`,
            cls: "global"
          }
        : { text: "⏸ 当前阶段：待运行（定时窗口关闭）", cls: "global" };
    return { text: "⏸ 当前阶段：待运行", cls: "global" };
  }
  if (t.status === "paused" && t.pause_reason === "waiting_window")
    return {
      text: `🕒 当前阶段：窗口已结束，等待下一个 ${t.schedule_start}–${t.schedule_end} 窗口`,
      cls: "global"
    };
  if (t.status === "paused") return { text: "⏸ 当前阶段：已暂停，可恢复", cls: "human" };
  if (t.task_phase === "plan_approval")
    return { text: "当前阶段：等待人工审批拆解方案", cls: "global" };
  if (isDesignAwaiting.value)
    return { text: "当前阶段：等待人工审批模块设计", cls: "global" };
  if (t.task_phase === "global_review") {
    if (t.status === "needs_human")
      return { text: "当前阶段：全局评审收口阶段 · 等待人工介入", cls: "global" };
    return { text: "当前阶段：全局评审收口阶段", cls: "global" };
  }
  if (t.status === "needs_human") return { text: "⛔ 当前阶段：等待人工介入", cls: "human" };
  return { text: "当前阶段：模块开发", cls: "" };
});

// ---------------- 数据加载 ----------------
async function load() {
  if (!props.taskId) return;
  loading.value = true;
  try {
    const d = await getTaskDetail(props.taskId);
    detail.value = d;
    if (!d.iterations.length || showIterationList.value) {
      iterDetail.value = null;
      return;
    }
    const ordered = [...d.iterations].sort(
      (a, b) => a.major - b.major || a.minor - b.minor || a.id - b.id
    );
    if (
      activeIterationId.value == null ||
      !d.iterations.some((i) => i.id === activeIterationId.value)
    ) {
      activeIterationId.value = ordered[ordered.length - 1].id;
    }
    const liveOnly = (iterDetail.value?.execution_events ?? []).filter((e) => e.id < 0);
    const det = await getIterationDetail(props.taskId, activeIterationId.value);
    if (liveOnly.length) {
      const ids = new Set(det.execution_events.map((e) => e.id));
      det.execution_events = [
        ...det.execution_events,
        ...liveOnly.filter((e) => !ids.has(e.id) && e.iteration_id === activeIterationId.value)
      ];
    }
    iterDetail.value = det;
    ensureSelection();
    await nextTick();
    scrollLogs();
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    loading.value = false;
  }
}

function ensureSelection() {
  const list = subtasks.value;
  if (!list.length) {
    selectedSubtaskId.value = null;
    return;
  }
  if (selectedSubtaskId.value && list.some((s) => s.id === selectedSubtaskId.value)) return;
  const current =
    list.find((s) => s.current_stage === "design_approval") ??
    list.find((s) => s.status === "needs_human") ??
    list.find((s) => s.status === "in_progress") ??
    list[0];
  selectedSubtaskId.value = current?.id ?? null;
}

function scheduleReload() {
  if (reloadTimer) window.clearTimeout(reloadTimer);
  reloadTimer = window.setTimeout(() => void load(), 220);
}

function scrollLogs() {
  const el = logBox.value;
  if (el) el.scrollTop = el.scrollHeight;
}

// ---------------- 迭代导航 ----------------
async function openIteration(id: number) {
  activeIterationId.value = id;
  preview.value = null;
  focusSubtaskId.value = null;
  selectedSubtaskId.value = null;
  showIterationList.value = false;
  await load();
}

function backToIterations() {
  showIterationList.value = true;
  iterDetail.value = null;
  preview.value = null;
  focusSubtaskId.value = null;
}

// ---------------- 预览联动 ----------------
function onPreview(payload: PreviewPayload) {
  preview.value = payload;
}
function closePreview() {
  preview.value = null;
}
/** 左栏可能点击到其它迭代下的子任务：自动切到该迭代再选中。 */
async function selectSubtask(id: number) {
  const sub = detail.value?.subtasks.find((s) => s.id === id);
  if (sub?.iteration_id && sub.iteration_id !== activeIterationId.value) {
    await openIteration(sub.iteration_id);
  }
  selectedSubtaskId.value = id;
}
async function viewSubtask(sub: Subtask) {
  await selectSubtask(sub.id);
  focusSubtaskId.value = sub.id;
  showTools.value = false;
}
async function openDesign(sub: Subtask) {
  await selectSubtask(sub.id);
  preview.value = {
    kind: "design",
    title: "模块设计方案",
    content: sub.design_content || "该模块还没有设计文档。",
    source: `#${sub.sort_index} ${sub.subtask_name} · 第 ${sub.current_round} 轮`,
    time: currentIteration.value?.version
  };
}
/** 把某个模块的每一轮归档渲染成可读文本，在右侧预览。 */
function openHistory(sub: Subtask) {
  void selectSubtask(sub.id);
  const typeLabel: Record<string, string> = {
    code_review: "代码评审",
    summary_review: "摘要校验",
    human_reject: "人工驳回",
    global_rework: "全局返工"
  };
  const blocks = sub.iteration_records.map((rec, idx) => {
    const decision =
      rec.review_decision === "pass" ? "✅ 通过" : rec.review_decision === "reject" ? "❌ 不通过" : "—";
    const lines = [
      `## 第 ${rec.round || idx + 1} 轮 · ${typeLabel[rec.type] ?? rec.type} · ${decision}`,
      `时间：${rec.created_at || "—"}`
    ];
    if (rec.review_feedback) lines.push("", "评审意见：", rec.review_feedback);
    if (rec.summary_content) lines.push("", "摘要：", rec.summary_content);
    if (rec.diff) lines.push("", "本轮 diff：", "```diff", rec.diff, "```");
    return lines.join("\n");
  });
  preview.value = {
    kind: "review",
    title: "模块轮次历史",
    content: blocks.length
      ? blocks.join("\n\n---\n\n")
      : "该模块还没有归档的轮次记录。",
    source: `#${sub.sort_index} ${sub.subtask_name} · 共 ${sub.iteration_records.length} 轮`,
    time: currentIteration.value?.version
  };
}
function clearFocus() {
  focusSubtaskId.value = null;
}
function contractPreview(kind: "tech_stack" | "contract" | "requirement" | "criteria") {
  const t = task.value;
  if (!t) return;
  const map = {
    tech_stack: {
      kind: "tech_stack" as const,
      title: "技术栈",
      content: t.tech_stack || "拆解完成后生成。"
    },
    contract: {
      kind: "contract" as const,
      title: "全局契约",
      content: t.global_contract || "拆解完成后生成。"
    },
    requirement: {
      kind: "requirement" as const,
      title: "顶层需求",
      content: t.top_requirement || "（空）"
    },
    criteria: {
      kind: "criteria" as const,
      title: "项目验收标准",
      content: t.global_accept_criteria || "（空）"
    }
  };
  preview.value = { ...map[kind], source: currentIteration.value?.version };
}
function previewIterationRequirement() {
  const it = currentIteration.value;
  preview.value = {
    kind: "requirement",
    title: `本次迭代需求（${it?.version || "待定"}）`,
    content: it?.requirement || "（暂无需求描述）",
    source: it?.version,
    time: it?.start_time
  };
}
function previewSelectedDesign() {
  const sub = selected.value;
  if (!sub) return showToast("请先在左侧选择一个子任务", "error");
  openDesign(sub);
}

// ---------------- 任务操作 ----------------
async function doResume() {
  if (!task.value) return;
  actionBusy.value = true;
  try {
    await resumeTask(task.value.id);
    showToast(
      task.value.status === "terminated" ? "任务已继续，从当前进度恢复" : "已请求恢复调度",
      "success"
    );
    scheduleReload();
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    actionBusy.value = false;
  }
}

function openFollowup() {
  if (!task.value) return;
  showFollowup.value = true;
}

function onFollowupQueued() {
  showFollowup.value = false;
  showIterationList.value = true;
  scheduleReload();
}

async function doRun() {
  if (!task.value) return;
  actionBusy.value = true;
  try {
    const outcome = await startTask(task.value.id);
    showToast(outcome.message, outcome.started ? "success" : "error");
    scheduleReload();
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    actionBusy.value = false;
  }
}

async function doPause() {
  if (!task.value) return;
  actionBusy.value = true;
  try {
    await pauseTask(task.value.id);
    showToast("已发送暂停请求", "success");
    scheduleReload();
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    actionBusy.value = false;
  }
}

async function doTerminate() {
  if (!task.value) return;
  if (
    !window.confirm(
      "确认终止当前任务？终止后不会再派发新的子任务；已产生的代码、日志和模块快照会保留，之后可以点击「继续任务」恢复。"
    )
  )
    return;
  actionBusy.value = true;
  try {
    await terminateTask(task.value.id);
    showToast("任务已终止", "success");
    scheduleReload();
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    actionBusy.value = false;
  }
}

function onRequirementsSaved(updated: TaskMain) {
  if (detail.value) detail.value.task = updated;
  scheduleReload();
}

async function saveConfig(input: RuntimeConfigInput) {
  if (!task.value) return;
  actionBusy.value = true;
  try {
    await updateTaskConfig(task.value.id, input);
    showToast("运行中配置已保存", "success");
    showConfig.value = false;
    await load();
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    actionBusy.value = false;
  }
}

async function doApprovePlan() {
  if (!task.value) return;
  actionBusy.value = true;
  try {
    await approvePlan(task.value.id);
    showToast("方案已通过，开始创建子任务", "success");
    scheduleReload();
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    actionBusy.value = false;
  }
}

async function submitPlanReject() {
  if (!task.value) return;
  if (!planRejectText.value.trim()) return showToast("请填写驳回意见", "error");
  actionBusy.value = true;
  try {
    await rejectPlan(task.value.id, planRejectText.value.trim());
    showPlanReject.value = false;
    showToast("已驳回方案，技术总监将重新拆解", "success");
    scheduleReload();
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    actionBusy.value = false;
  }
}

async function doApproveDesign() {
  if (!task.value || !designGateSubtask.value) return;
  actionBusy.value = true;
  try {
    await approveDesign(task.value.id, designGateSubtask.value.id);
    showToast("设计已通过，开始派发 Coder", "success");
    scheduleReload();
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    actionBusy.value = false;
  }
}

async function submitDesignReject() {
  if (!task.value || !designGateSubtask.value) return;
  if (!designRejectText.value.trim()) return showToast("请填写设计修改意见", "error");
  actionBusy.value = true;
  try {
    await rejectDesign(task.value.id, designGateSubtask.value.id, designRejectText.value.trim());
    showDesignReject.value = false;
    showToast("已驳回设计，技术总监将重新设计", "success");
    scheduleReload();
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    actionBusy.value = false;
  }
}

async function sendMessage() {
  if (!task.value) return;
  const content = humanMessage.value.trim();
  if (!content) return showToast("请输入要介入的消息", "error");
  actionBusy.value = true;
  try {
    await sendHumanMessage(task.value.id, content);
    humanMessage.value = "";
    showToast("消息已发送，将在下一个 Agent 步骤注入", "success");
    scheduleReload();
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    actionBusy.value = false;
  }
}

async function forcePass() {
  if (!task.value || !humanTarget.value) return;
  const target = humanTarget.value;
  const action = humanActionLabel.value;
  if (!window.confirm(`确认对子任务「${target.subtask_name}」执行：${action}？`)) return;
  actionBusy.value = true;
  try {
    await humanForcePass(task.value.id, target.id);
    showToast("已人工通过，调度继续", "success");
    scheduleReload();
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    actionBusy.value = false;
  }
}

async function extendRounds(rounds: number) {
  if (!task.value || !humanTarget.value) return;
  const target = humanTarget.value;
  const extra = Number(rounds);
  if (!Number.isInteger(extra) || extra < 1) return showToast("请选择追加轮次", "error");
  if (
    !window.confirm(
      `确认给子任务「${target.subtask_name}」追加 ${extra} 轮？将从当前失败阶段继续自动执行。`
    )
  )
    return;
  actionBusy.value = true;
  try {
    await extendSubtaskRounds(task.value.id, target.id, extra);
    showToast(`已追加 ${extra} 轮，任务继续自动执行`, "success");
    scheduleReload();
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    actionBusy.value = false;
  }
}

function openRejectModal() {
  if (!humanTarget.value) return;
  rejectText.value = "";
  showReject.value = true;
}
function openPlanReject() {
  planRejectText.value = "";
  showPlanReject.value = true;
}
function openDesignReject() {
  designRejectText.value = "";
  showDesignReject.value = true;
}

async function submitReject() {
  if (!task.value || !humanTarget.value) return;
  if (!rejectText.value.trim()) return showToast("请填写人工修改意见", "error");
  actionBusy.value = true;
  try {
    await humanReject(task.value.id, humanTarget.value.id, rejectText.value.trim());
    showReject.value = false;
    showToast("已人工驳回并继续派发 Coder", "success");
    scheduleReload();
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    actionBusy.value = false;
  }
}

async function doGlobalForcePass() {
  if (!task.value) return;
  if (!window.confirm("确认人工强制通过全局评审，直接将任务标记为完成？")) return;
  actionBusy.value = true;
  try {
    await globalHumanForcePass(task.value.id);
    showToast("任务已人工收口完成", "success");
    scheduleReload();
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    actionBusy.value = false;
  }
}

function openGlobalRework() {
  const target = subtasks.value.find((s) => s.status !== "completed") ?? subtasks.value[0];
  globalTargetId.value = target?.id ?? 0;
  globalFeedback.value = "";
  showGlobalRework.value = true;
}

async function submitGlobalRework() {
  if (!task.value) return;
  if (!globalTargetId.value) return showToast("请选择要返工的模块", "error");
  if (!globalFeedback.value.trim()) return showToast("请填写返工意见", "error");
  actionBusy.value = true;
  try {
    await globalHumanRework(task.value.id, globalTargetId.value, globalFeedback.value.trim());
    showGlobalRework.value = false;
    showToast("已下发人工全局返工", "success");
    scheduleReload();
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    actionBusy.value = false;
  }
}

async function openConsole() {
  if (!task.value) return;
  try {
    allEvents.value = await listTaskEvents(task.value.id);
  } catch (e) {
    allEvents.value = executionEvents.value;
    showToast(errorMessage(e), "error");
  }
  showConsole.value = true;
}

function eventKindLabel(kind: string) {
  if (kind === "llm_response:requirement_intake") return "追加需求梳理结果";
  const map: Record<string, string> = {
    llm_request: "LLM 请求",
    llm_response: "LLM 响应",
    llm_error: "LLM 错误",
    dsh_stdout: "dsh 输出",
    dsh_stderr: "dsh stderr",
    coder_prompt: "Coder Prompt",
    summary_prompt: "摘要 Prompt",
    git_diff: "Git Diff",
    module_design: "模块设计",
    summary_file: "摘要文件",
    human_message: "人工消息",
    human_message_delivered: "人工消息已注入",
    plan_approval: "方案待审批",
    plan_approved: "方案通过",
    plan_rejected: "方案驳回",
    requirement_queued: "追加需求已提交",
    requirement_accepted: "追加需求已梳理",
    requirement_intake: "追加需求梳理",
    design_approved: "设计通过",
    design_rejected: "设计驳回",
    config_updated: "配置更新",
    plan_accepted: "方案确认",
    agent_message: "Agent 对话",
    tool_call: "工具调用",
    tool_result: "工具结果",
    dsh_error: "Coder 错误",
    dsh_raw: "dsh 原始输出",
    system_log: "系统日志",
    requirements_updated: "需求更新",
    human_extend_rounds: "人工追加轮次"
  };
  const base = kind.includes(":") ? kind.slice(0, kind.indexOf(":")) : kind;
  return map[base] ?? kind;
}
function eventKindClass(kind: string) {
  if (kind.startsWith("llm_error") || kind === "design_rejected" || kind === "plan_rejected")
    return "ev-error";
  if (kind.startsWith("llm_response") || kind === "plan_approved" || kind === "design_approved")
    return "ev-ok";
  if (
    kind === "human_message" ||
    kind === "human_message_delivered" ||
    kind === "requirement_queued"
  )
    return "ev-human";
  if (kind.startsWith("llm_request")) return "ev-request";
  return "";
}

function applyLiveEvent(event: TaskEvent) {
  if (!detail.value || event.task_id !== detail.value.task.id) return;
  if (!iterDetail.value || event.iteration_id !== activeIterationId.value) return;
  const exists = iterDetail.value.execution_events.some((e) => e.id === event.id);
  if (!exists) iterDetail.value.execution_events.push(event);
  if (iterDetail.value.execution_events.length > 5000) {
    iterDetail.value.execution_events = iterDetail.value.execution_events.slice(-4000);
  }
}

watch(
  () => props.taskId,
  () => {
    detail.value = null;
    iterDetail.value = null;
    activeIterationId.value = null;
    selectedSubtaskId.value = null;
    preview.value = null;
    focusSubtaskId.value = null;
    showIterationList.value = true;
    void load();
  }
);

watch(
  () => logs.value.length,
  async () => {
    await nextTick();
    scrollLogs();
  }
);

onMounted(async () => {
  await load();
  unlisten = await listen<{ task_id: number }>("task-updated", (ev) => {
    if (ev.payload?.task_id === props.taskId) scheduleReload();
  });
  eventUnlisten = await listen<{ task_id: number; event: TaskEvent }>("task-event", (ev) => {
    if (ev.payload?.task_id === props.taskId && ev.payload.event) applyLiveEvent(ev.payload.event);
  });
  pollTimer = window.setInterval(() => {
    if (isRunning.value) void load();
  }, 3000);
});

onUnmounted(() => {
  unlisten?.();
  eventUnlisten?.();
  if (pollTimer) window.clearInterval(pollTimer);
  if (reloadTimer) window.clearTimeout(reloadTimer);
});
</script>

<template>
  <div class="workbench">
    <div v-if="!props.taskId" class="empty-state" style="margin: auto">
      <div class="big">🛠️</div>
      <div class="title">尚未打开任务</div>
      <div class="small">请到「任务列表」打开一个任务详情。</div>
      <div style="margin-top: 14px">
        <button class="btn btn-primary" @click="emit('back')">去任务列表</button>
      </div>
    </div>

    <!-- 第一屏：迭代（版本）列表，按版本倒序 -->
    <IterationList
      v-else-if="task && showIterationList"
      :task="task"
      :iterations="iterations"
      :loading="loading"
      :busy="actionBusy"
      @open="openIteration"
      @back="emit('back')"
      @followup="openFollowup"
      @refresh="load"
    />

    <template v-else-if="task">
      <div class="workbench-top">
        <button class="back" title="返回迭代列表" @click="backToIterations">←</button>
        <div>
          <div class="task-title">
            <span class="iteration-badge">{{ currentIteration?.version || "dev-?.? 待定" }}</span>
            {{ task.task_name }}
          </div>
          <div class="meta">
            <span>评审：{{ detail?.reviewer?.name ?? "—" }}</span>
            <span>Coder：{{ detail?.coders.map((c) => c.name).join("、") || "—" }}</span>
            <span>阶段：{{ phaseLabel }}</span>
            <span>状态：{{ statusText }}</span>
            <span>执行：{{ executionModeLabel }}</span>
            <span v-if="scheduleText">窗口：{{ scheduleText }}</span>
            <span>人工审核：{{ task.manual_review_enabled ? "开启" : "关闭" }}</span>
            <span>轮次：{{ task.current_global_round }}/{{ task.global_max_round }}</span>
          </div>
        </div>
        <div class="flex" style="margin-left: auto">
          <button class="btn btn-sm" @click="showConfig = true">⚙ 配置</button>
          <button
            v-if="task.status !== 'completed' && task.status !== 'terminated'"
            class="btn btn-sm"
            :disabled="actionBusy"
            @click="showRequirements = true"
          >
            📝 需求
          </button>
          <button
            v-if="isCompleted"
            class="btn btn-sm btn-primary"
            :disabled="actionBusy"
            @click="openFollowup"
          >
            ＋ 追加需求
          </button>
          <button class="btn btn-sm" :disabled="actionBusy" @click="showChat = true">
            💬 问答
          </button>
          <button v-if="canRun" class="btn btn-sm btn-primary" :disabled="actionBusy" @click="doRun">
            运行
          </button>
          <button v-if="canPause" class="btn btn-sm" :disabled="actionBusy" @click="doPause">
            暂停
          </button>
          <button
            v-if="canResume && !humanTarget"
            class="btn btn-sm"
            :disabled="actionBusy"
            @click="doResume"
          >
            恢复
          </button>
          <button
            v-if="canContinue"
            class="btn btn-sm btn-primary"
            :disabled="actionBusy"
            @click="doResume"
          >
            继续任务
          </button>
          <button
            v-if="canTerminate"
            class="btn btn-sm btn-danger"
            :disabled="actionBusy"
            @click="doTerminate"
          >
            终止
          </button>
        </div>
        <div class="phase-banner" :class="phaseBanner.cls" style="margin-left: 10px">
          {{ phaseBanner.text }}
        </div>
      </div>

      <div class="wb-columns">
        <IterationTree
          :task="task"
          :iterations="iterations"
          :subtasks="detail?.subtasks ?? []"
          :active-iteration-id="activeIterationId"
          :selected-subtask-id="selectedSubtaskId"
          :frozen="isGlobal && !isCompleted"
          @open-iteration="openIteration"
          @select-subtask="selectSubtask"
          @view="viewSubtask"
          @design="openDesign"
          @history="openHistory"
        />

        <section class="wb-col">
          <div v-if="!isLatestIteration" class="history-banner">
            正在查看历史迭代 <b>{{ currentIteration?.version }}</b>
            （当前最新版本 {{ iterations[iterations.length - 1]?.version || "待定" }}）。
            <button class="link-btn" @click="backToIterations">返回迭代列表</button>
          </div>

          <ActionDock
            :task="task"
            :subtasks="subtasks"
            :human-target="humanTarget"
            :design-gate-subtask="designGateSubtask"
            :pending-human-subtask="pendingHumanSubtask"
            :pending-plan="pendingPlan"
            :can-human="canHuman"
            :busy="actionBusy"
            :human-stage-label="humanStageLabel"
            :human-action-label="humanActionLabel"
            :human-reason="humanReason"
            @run="doRun"
            @resume="doResume"
            @followup="openFollowup"
            @approve-plan="doApprovePlan"
            @reject-plan="openPlanReject"
            @approve-design="doApproveDesign"
            @reject-design="openDesignReject"
            @force-pass="forcePass"
            @reject="openRejectModal"
            @extend="extendRounds"
            @global-force-pass="doGlobalForcePass"
            @global-rework="openGlobalRework"
          />

          <!-- 方案与契约：主要内容入口固定在中栏顶部，点击后在右栏预览 -->
          <div class="contract-bar">
            <span class="contract-bar-title">方案与契约</span>
            <button class="chip" @click="previewIterationRequirement">本次迭代需求</button>
            <button class="chip" @click="contractPreview('requirement')">顶层需求</button>
            <button class="chip" @click="contractPreview('criteria')">验收标准</button>
            <button class="chip" @click="contractPreview('tech_stack')">技术栈</button>
            <button class="chip" @click="contractPreview('contract')">全局契约</button>
            <button class="chip" @click="previewSelectedDesign">
              当前模块设计{{ selected?.design_content ? "" : "（暂无）" }}
            </button>
            <button class="chip" style="margin-left: auto" @click="openConsole">原始事件</button>
          </div>

          <TimelinePanel
            v-model:show-tools="showTools"
            :events="executionEvents"
            :subtasks="subtasks"
            :iteration="currentIteration"
            :reviewer-name="detail?.reviewer?.name ?? '技术总监'"
            :coder-name="detail?.coders[0]?.name ?? 'Coder'"
            :focus-subtask-id="focusSubtaskId"
            @preview="onPreview"
            @clear-focus="clearFocus"
          />

          <div v-if="isPlanApproval" class="global-view">
            <div class="current-head">
              <div>
                <div class="t">拆解方案待人工审批</div>
                <div class="s">
                  技术总监已生成技术栈、全局契约和 {{ pendingPlan?.subtasks.length ?? 0 }} 个模块。
                  同意后才会创建子任务并派发 Coder。
                </div>
              </div>
              <span class="badge badge-yellow right">等待审批</span>
            </div>

            <div class="round-block">
              <div class="head">技术栈</div>
              <div class="body">
                <pre class="pre-box" style="max-height: 160px">{{ pendingPlan?.tech_stack || task.tech_stack || "（未提供）" }}</pre>
              </div>
            </div>

            <div class="round-block">
              <div class="head">全局契约</div>
              <div class="body">
                <pre class="pre-box" style="max-height: 260px">{{ pendingPlan?.global_contract || task.global_contract || "（未提供）" }}</pre>
              </div>
            </div>

            <div class="round-block">
              <div class="head">模块拆解与验收标准</div>
              <div class="body">
                <div
                  v-for="(item, idx) in pendingPlan?.subtasks ?? []"
                  :key="idx"
                  class="plan-module-row"
                >
                  <div class="pm-name">#{{ idx + 1 }} {{ item.name }}</div>
                  <div class="pm-criteria">{{ item.accept_criteria }}</div>
                </div>
              </div>
            </div>
          </div>

          <div v-else-if="isGlobal" class="global-view">
            <div class="current-head">
              <div>
                <div class="t">
                  {{ isCompleted ? "全局评审收口完成" : "全局评审收口阶段" }}
                </div>
                <div class="s">
                  迭代 {{ currentIteration?.version }} · 第
                  {{ task.current_global_round }} / {{ task.global_max_round }} 轮 ·
                  上下文 = 全局契约 + 所有模块摘要 + 全部模块 diff 汇总
                </div>
              </div>
              <span class="badge right" :class="isCompleted ? 'badge-green' : 'badge-yellow'">
                {{
                  isCompleted
                    ? "✅ 已完成"
                    : task.status === "needs_human"
                      ? "等待人工介入"
                      : "评审中"
                }}
              </span>
            </div>

            <div v-if="!task.global_review_records.length" class="round-block">
              <div class="body muted">正在汇总全部模块的 summary 与 diff，提交全局评审…</div>
            </div>

            <div v-for="rec in task.global_review_records" :key="rec.round" class="global-round-card">
              <div class="gh">
                <span class="badge badge-blue">
                  第 {{ rec.cycle || 1 }} 次迭代 · 全局第 {{ rec.round }} 轮
                </span>
                <span class="decision" :class="rec.passed ? 'pass' : 'reject'">
                  {{ rec.passed ? "✅ 通过" : "❌ 不通过" }}
                </span>
                <span class="small muted right">{{ rec.created_at }}</span>
              </div>
              <div style="white-space: pre-wrap; color: var(--text-2); font-size: 12.5px">
                {{ rec.feedback || "（无补充说明）" }}
              </div>
              <div v-for="(issue, i) in rec.issues" :key="i" class="issue-item">
                <div class="mod">归属模块：{{ issue.subtask_name || `#${issue.subtask_id}` }}</div>
                <div style="white-space: pre-wrap; font-size: 12px; margin-top: 4px">
                  {{ issue.feedback }}
                </div>
              </div>
            </div>
          </div>

          <div
            v-if="
              isLatestIteration &&
              task.status !== 'completed' &&
              task.status !== 'terminated'
            "
            class="human-composer"
          >
            <div class="hc-title">
              人工实时介入
              <span class="small muted">消息会在下一个 Agent 步骤注入；不会清空任何数据</span>
            </div>
            <div class="hc-row">
              <textarea
                v-model="humanMessage"
                rows="2"
                placeholder="随时输入消息，例如：先完成登录再做列表；接口统一使用 snake_case；停止当前方案…"
                @keydown.ctrl.enter.prevent="sendMessage"
              ></textarea>
              <button class="btn btn-primary" :disabled="actionBusy" @click="sendMessage">
                发送介入
              </button>
            </div>
            <div v-if="pendingMessages.length" class="pending-msgs">
              <span class="small muted">待注入：</span>
              <span v-for="m in pendingMessages" :key="m.id" class="pending-msg">
                {{ m.content }}
              </span>
            </div>
          </div>

          <details class="system-logs">
            <summary>系统事件日志（{{ logs.length }}）</summary>
            <div ref="logBox" class="log-box">
              <div v-if="!logs.length" class="muted">
                该迭代没有持久化日志（可能创建时未开启日志持久化）。
              </div>
              <div v-for="log in logs" :key="log.id" class="log-line" :class="log.level">
                <span class="log-time">{{ log.create_time.slice(11) }}</span>
                <span class="log-msg">{{ log.message }}</span>
              </div>
            </div>
          </details>
        </section>

        <PreviewPanel
          :preview="preview"
          :task="task"
          :iteration="currentIteration"
          :selected-subtask="selected"
          @preview="onPreview"
          @close="closePreview"
        />
      </div>
    </template>

    <div v-else class="empty-state" style="margin: auto">
      <div class="big">⏳</div>
      <div class="title">{{ loading ? "加载任务…" : "任务不存在" }}</div>
    </div>

    <div v-if="showReject" class="modal-mask" @click.self="showReject = false">
      <div class="modal narrow">
        <div class="modal-head">
          人工驳回 · {{ humanTarget?.subtask_name ?? "当前模块" }}
          <button class="x" @click="showReject = false">×</button>
        </div>
        <div class="modal-body">
          <div class="field">
            <label>修改意见（会自动加入当前子任务轮次）</label>
            <textarea v-model="rejectText" rows="6" placeholder="明确指出需要 Coder 修改的内容。"></textarea>
          </div>
          <div class="small muted" style="margin-top: 8px">
            提交后会立刻把意见派发给当前 Coder 重新修改，并消耗子任务轮次。
          </div>
        </div>
        <div class="modal-foot">
          <button class="btn btn-ghost" @click="showReject = false">取消</button>
          <button class="btn btn-primary" :disabled="actionBusy" @click="submitReject">确认驳回并继续</button>
        </div>
      </div>
    </div>

    <div v-if="showPlanReject" class="modal-mask" @click.self="showPlanReject = false">
      <div class="modal narrow">
        <div class="modal-head">
          驳回拆解方案
          <button class="x" @click="showPlanReject = false">×</button>
        </div>
        <div class="modal-body">
          <div class="field">
            <label>给技术总监的修改意见</label>
            <textarea v-model="planRejectText" rows="6" placeholder="例如：模块拆分太粗；技术栈改为 Vue3 + Python；补充权限模块。"></textarea>
          </div>
        </div>
        <div class="modal-foot">
          <button class="btn btn-ghost" @click="showPlanReject = false">取消</button>
          <button class="btn btn-primary" :disabled="actionBusy" @click="submitPlanReject">驳回并重新拆解</button>
        </div>
      </div>
    </div>

    <div v-if="showDesignReject" class="modal-mask" @click.self="showDesignReject = false">
      <div class="modal narrow">
        <div class="modal-head">
          驳回模块设计
          <button class="x" @click="showDesignReject = false">×</button>
        </div>
        <div class="modal-body">
          <div class="field">
            <label>设计修改意见</label>
            <textarea v-model="designRejectText" rows="6" placeholder="例如：接口返回体不符合契约；缺少权限校验；补充表结构。"></textarea>
          </div>
        </div>
        <div class="modal-foot">
          <button class="btn btn-ghost" @click="showDesignReject = false">取消</button>
          <button class="btn btn-primary" :disabled="actionBusy" @click="submitDesignReject">驳回并重新设计</button>
        </div>
      </div>
    </div>

    <div v-if="showGlobalRework" class="modal-mask" @click.self="showGlobalRework = false">
      <div class="modal narrow">
        <div class="modal-head">
          人工指定模块返工
          <button class="x" @click="showGlobalRework = false">×</button>
        </div>
        <div class="modal-body">
          <div class="field">
            <label>返工模块</label>
            <select v-model.number="globalTargetId">
              <option v-for="s in subtasks" :key="s.id" :value="s.id">
                #{{ s.sort_index }} {{ s.subtask_name }}
              </option>
            </select>
          </div>
          <div class="field" style="margin-top: 12px">
            <label>返工意见</label>
            <textarea v-model="globalFeedback" rows="6" placeholder="全局评审发现问题，需要该模块修改的内容。"></textarea>
          </div>
        </div>
        <div class="modal-foot">
          <button class="btn btn-ghost" @click="showGlobalRework = false">取消</button>
          <button class="btn btn-primary" :disabled="actionBusy" @click="submitGlobalRework">下发返工并继续</button>
        </div>
      </div>
    </div>

    <RuntimeConfigModal
      v-if="showConfig && task"
      :task="task"
      :busy="actionBusy"
      @close="showConfig = false"
      @save="saveConfig"
    />

    <ChatPanel
      v-if="showChat && task"
      :task-id="task.id"
      :task-name="task.task_name"
      :reviewer-name="detail?.reviewer?.name ?? '技术总监'"
      @close="showChat = false"
    />

    <RequirementsModal
      v-if="showRequirements && task"
      :task="task"
      @close="showRequirements = false"
      @saved="onRequirementsSaved"
    />

    <FollowUpModal
      v-if="showFollowup && task"
      :task="task"
      :subtask-count="detail?.subtasks.length ?? 0"
      @close="showFollowup = false"
      @queued="onFollowupQueued"
    />

    <div v-if="showConsole" class="modal-mask" @click.self="showConsole = false">
      <div class="modal console-modal">
        <div class="modal-head">
          原始事件 · 全链路
          <span class="small muted" style="margin-left: 10px">
            {{ (allEvents.length ? allEvents : executionEvents).length }} 条
          </span>
          <button class="x" @click="showConsole = false">×</button>
        </div>
        <div class="console-modal-body">
          <div v-if="!(allEvents.length ? allEvents : executionEvents).length" class="muted">
            暂无执行输出
          </div>
          <div
            v-for="ev in allEvents.length ? allEvents : executionEvents"
            :key="'all-' + ev.id"
            class="event-line"
            :class="eventKindClass(ev.kind)"
          >
            <div class="ev-head">
              <span class="ev-kind">{{ eventKindLabel(ev.kind) }}</span>
              <span class="ev-time">{{ ev.create_time }}</span>
              <span v-if="ev.subtask_id" class="ev-sub">subtask #{{ ev.subtask_id }}</span>
            </div>
            <pre class="ev-content">{{ ev.content }}</pre>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
