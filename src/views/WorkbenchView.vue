<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import SubtaskCard from "../components/SubtaskCard.vue";
import type { Subtask, TaskDetail, TaskEvent } from "../types";
import {
  approveDesign,
  approvePlan,
  cancelTask,
  getTaskDetail,
  globalHumanForcePass,
  globalHumanRework,
  humanForcePass,
  humanReject,
  rejectDesign,
  rejectPlan,
  resumeTask,
  sendHumanMessage,
  updateTaskConfig
} from "../api";
import { errorMessage, showToast } from "../toast";

const props = defineProps<{ taskId: number | null }>();
const emit = defineEmits<{
  (e: "back"): void;
  (e: "open-task", id: number): void;
}>();

const detail = ref<TaskDetail | null>(null);
const selectedSubtaskId = ref<number | null>(null);
const loading = ref(false);
const actionBusy = ref(false);

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
const configForm = reactive({
  module_max_round: 3,
  global_max_round: 2,
  manual_review_enabled: false
});

const humanMessage = ref("");
const contractOpen = ref(true);
const techStackOpen = ref(true);
const openSummaryIds = ref<Set<number>>(new Set());
const logBox = ref<HTMLElement | null>(null);
const eventBox = ref<HTMLElement | null>(null);
const showConsole = ref(false);

let unlisten: UnlistenFn | null = null;
let eventUnlisten: UnlistenFn | null = null;
let pollTimer: number | null = null;
let reloadTimer: number | null = null;

const task = computed(() => detail.value?.task ?? null);
const subtasks = computed<Subtask[]>(() => detail.value?.subtasks ?? []);
const isGlobal = computed(() => task.value?.task_phase === "global_review");
const isPlanApproval = computed(() => task.value?.task_phase === "plan_approval");
const isRunning = computed(
  () => task.value?.status === "running" || task.value?.status === "planning"
);
const selected = computed(
  () => subtasks.value.find((s) => s.id === selectedSubtaskId.value) ?? null
);
const completedSubtasks = computed(() => subtasks.value.filter((s) => s.status === "completed"));
const latestRecord = computed(() => {
  const recs = selected.value?.iteration_records ?? [];
  return recs.length ? recs[recs.length - 1] : null;
});
const designGateSubtask = computed(
  () => subtasks.value.find((s) => s.current_stage === "design_approval") ?? null
);
const isDesignAwaiting = computed(() => !!designGateSubtask.value);
const isDesignStage = computed(
  () =>
    selected.value?.current_stage === "design_generation" ||
    selected.value?.current_stage === "design_approval"
);
const pendingMessages = computed(
  () => (detail.value?.human_messages ?? []).filter((m) => m.status === "pending")
);
const executionEvents = computed(() => detail.value?.execution_events ?? []);
const visibleEvents = computed(() => executionEvents.value.slice(-300));
const pendingPlan = computed(() => detail.value?.pending_plan ?? null);
const canResume = computed(
  () =>
    !!task.value &&
    !isRunning.value &&
    task.value.status !== "completed" &&
    !isPlanApproval.value &&
    !isDesignAwaiting.value
);
const canCancel = computed(
  () =>
    !!task.value &&
    task.value.status !== "completed" &&
    task.value.status !== "paused" &&
    task.value.status !== "failed"
);
const canHuman = computed(
  () =>
    !!task.value &&
    !isRunning.value &&
    task.value.status !== "completed" &&
    !!selected.value &&
    (selected.value.status === "in_progress" || selected.value.status === "needs_human") &&
    !isDesignAwaiting.value
);
const statusText = computed(() => {
  const s = task.value?.status ?? "";
  const map: Record<string, string> = {
    planning: "拆解需求中",
    running: "执行中",
    needs_human: "需要人工介入",
    paused: "已暂停",
    completed: "已完成",
    failed: "执行失败"
  };
  return map[s] ?? s;
});
const phaseBanner = computed(() => {
  const t = task.value;
  if (!t) return { text: "", cls: "" };
  if (t.status === "completed") return { text: "✅ 当前阶段：全部完成", cls: "done" };
  if (t.status === "failed") return { text: "❌ 当前阶段：执行失败", cls: "human" };
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
const stageLabel = computed(() => {
  const map: Record<string, string> = {
    queued: "等待派发",
    design_generation: "Reviewer 生成模块设计中",
    design_approval: "等待人工审批设计",
    coding: "Coder 编码中",
    code_review: "Reviewer 代码评审中",
    summary_generation: "Coder 生成模块摘要中",
    summary_review: "Reviewer 校验摘要中",
    done: "已完成",
    needs_human: "等待人工处理"
  };
  return map[selected.value?.current_stage ?? ""] ?? selected.value?.current_stage ?? "";
});
const subtaskStatusLabel = computed(() => {
  const map: Record<string, string> = {
    waiting: "等待队列中",
    in_progress: "当前开发中",
    completed: "已完成",
    needs_human: "需要人工介入"
  };
  return map[selected.value?.status ?? ""] ?? selected.value?.status ?? "";
});

function decisionText(decision: string) {
  if (decision === "pass") return "✅ 通过";
  if (decision === "reject") return "❌ 不通过";
  return "⏳ 评审中";
}
function feedbackText(rec: { review_decision: string; review_feedback: string } | null) {
  if (!rec) return "";
  if (!rec.review_decision) return "评审进行中，请稍候…";
  return rec.review_feedback || "（无评审意见）";
}
function eventKindLabel(kind: string) {
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
    design_approved: "设计通过",
    design_rejected: "设计驳回",
    config_updated: "配置更新",
    plan_accepted: "方案确认"
  };
  const base = kind.includes(":") ? kind.slice(0, kind.indexOf(":")) : kind;
  return map[base] ?? kind;
}
function eventKindClass(kind: string) {
  if (kind.startsWith("llm_error") || kind === "design_rejected" || kind === "plan_rejected")
    return "ev-error";
  if (kind.startsWith("llm_response") || kind === "plan_approved" || kind === "design_approved")
    return "ev-ok";
  if (kind === "human_message" || kind === "human_message_delivered") return "ev-human";
  if (kind.startsWith("llm_request")) return "ev-request";
  return "";
}

async function load() {
  if (!props.taskId) return;
  loading.value = true;
  try {
    const d = await getTaskDetail(props.taskId);
    detail.value = d;
    if (!showConfig.value) {
      configForm.module_max_round = d.task.module_max_round;
      configForm.global_max_round = d.task.global_max_round;
      configForm.manual_review_enabled = d.task.manual_review_enabled;
    }
    ensureSelection();
    await nextTick();
    scrollLogs();
    scrollEvents();
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    loading.value = false;
  }
}

function ensureSelection() {
  const list = detail.value?.subtasks ?? [];
  if (!list.length) {
    selectedSubtaskId.value = null;
    return;
  }
  if (selectedSubtaskId.value && list.some((s) => s.id === selectedSubtaskId.value)) return;
  const current =
    list.find((s) => s.current_stage === "design_approval") ??
    list.find((s) => s.status === "in_progress") ??
    list.find((s) => s.status === "needs_human") ??
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
function scrollEvents() {
  const el = eventBox.value;
  if (el) el.scrollTop = el.scrollHeight;
  const modal = document.querySelector(".console-modal-body") as HTMLElement | null;
  if (modal) modal.scrollTop = modal.scrollHeight;
}

function toggleSummary(id: number) {
  const next = new Set(openSummaryIds.value);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  openSummaryIds.value = next;
}

async function doResume() {
  if (!task.value) return;
  actionBusy.value = true;
  try {
    await resumeTask(task.value.id);
    showToast("已请求恢复调度", "success");
    scheduleReload();
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    actionBusy.value = false;
  }
}

async function doCancel() {
  if (!task.value) return;
  if (!window.confirm("确认终止当前任务？当前 dsh 会被立即停止；已产生的数据和执行记录都会保留，之后可以恢复。"))
    return;
  actionBusy.value = true;
  try {
    await cancelTask(task.value.id);
    showToast("已发送终止请求", "success");
    scheduleReload();
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    actionBusy.value = false;
  }
}

async function saveConfig() {
  if (!task.value) return;
  actionBusy.value = true;
  try {
    await updateTaskConfig(task.value.id, {
      module_max_round: Number(configForm.module_max_round),
      global_max_round: Number(configForm.global_max_round),
      manual_review_enabled: configForm.manual_review_enabled
    });
    showToast("运行中配置已保存", "success");
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
    await rejectDesign(
      task.value.id,
      designGateSubtask.value.id,
      designRejectText.value.trim()
    );
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
  if (!task.value || !selected.value) return;
  if (!window.confirm(`确认人工强制通过子任务「${selected.value.subtask_name}」？`)) return;
  actionBusy.value = true;
  try {
    await humanForcePass(task.value.id, selected.value.id);
    showToast("已人工强制通过，调度继续", "success");
    scheduleReload();
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    actionBusy.value = false;
  }
}

function openRejectModal() {
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
  if (!task.value || !selected.value) return;
  if (!rejectText.value.trim()) return showToast("请填写人工修改意见", "error");
  actionBusy.value = true;
  try {
    await humanReject(task.value.id, selected.value.id, rejectText.value.trim());
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

function applyLiveEvent(event: TaskEvent) {
  if (!detail.value || event.task_id !== detail.value.task.id) return;
  const exists = detail.value.execution_events.some((e) => e.id === event.id);
  if (!exists) detail.value.execution_events.push(event);
  if (detail.value.execution_events.length > 4000) {
    detail.value.execution_events = detail.value.execution_events.slice(-3000);
  }
  void nextTick().then(scrollEvents);
}

watch(
  () => props.taskId,
  () => {
    selectedSubtaskId.value = null;
    detail.value = null;
    void load();
  }
);

watch(
  () => detail.value?.logs.length,
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

    <template v-else-if="task">
      <div class="workbench-top">
        <button class="back" title="返回任务列表" @click="emit('back')">←</button>
        <div>
          <div class="task-title">{{ task.task_name }}</div>
          <div class="meta">
            <span>评审：{{ detail?.reviewer?.name ?? "—" }}</span>
            <span>Coder：{{ detail?.coders.map((c) => c.name).join("、") || "—" }}</span>
            <span>阶段：{{ task.task_phase === "global_review" ? "全局评审" : task.task_phase === "plan_approval" ? "方案审批" : "模块开发" }}</span>
            <span>状态：{{ statusText }}</span>
            <span>人工审核：{{ task.manual_review_enabled ? "开启" : "关闭" }}</span>
            <span>模块轮次：{{ task.module_max_round }}</span>
            <span>全局轮次：{{ task.current_global_round }}/{{ task.global_max_round }}</span>
          </div>
        </div>
        <div class="flex" style="margin-left: auto">
          <button class="btn btn-sm" @click="showConfig = !showConfig">
            ⚙ 运行中配置
          </button>
          <button
            v-if="canCancel"
            class="btn btn-sm btn-danger"
            :disabled="actionBusy"
            @click="doCancel"
          >
            终止执行
          </button>
          <button v-if="canResume" class="btn btn-sm" :disabled="actionBusy" @click="doResume">
            恢复调度
          </button>
        </div>
        <div class="phase-banner" :class="phaseBanner.cls" style="margin-left: 10px">
          {{ phaseBanner.text }}
        </div>
      </div>

      <div v-if="showConfig" class="runtime-config">
        <div class="rc-title">运行中配置 <span>修改后立即生效，不需要停止任务</span></div>
        <div class="rc-grid">
          <div class="field">
            <label>单个子任务最大轮次</label>
            <input v-model.number="configForm.module_max_round" type="number" min="1" max="99" />
          </div>
          <div class="field">
            <label>全局评审最大轮次</label>
            <input v-model.number="configForm.global_max_round" type="number" min="1" max="99" />
          </div>
          <div class="field" style="justify-content: flex-end">
            <label class="flex" style="cursor: pointer">
              <input v-model="configForm.manual_review_enabled" type="checkbox" />
              人工审核方案与模块设计
            </label>
          </div>
          <div class="field" style="justify-content: flex-end">
            <button class="btn btn-primary" :disabled="actionBusy" @click="saveConfig">
              保存配置
            </button>
          </div>
        </div>
      </div>

      <div class="wb-columns">
        <aside class="wb-col left">
          <div class="wb-section-title">
            <template v-if="isPlanApproval">待审批模块方案</template>
            <template v-else>子任务（模块）队列</template>
            <span v-if="!isPlanApproval" class="count">
              {{ completedSubtasks.length }}/{{ subtasks.length }} 已完成
            </span>
            <span v-else class="count">{{ pendingPlan?.subtasks.length ?? 0 }} 个模块</span>
          </div>

          <template v-if="isPlanApproval">
            <div class="plan-wait-tip">
              技术总监已完成拆解，同意方案后才会创建子任务并交给 Coder。
            </div>
            <article
              v-for="(item, idx) in pendingPlan?.subtasks ?? []"
              :key="idx"
              class="subtask-card plan-item"
            >
              <div class="subtask-main">
                <div class="subtask-head">
                  <div class="subtask-index">{{ idx + 1 }}</div>
                  <div class="subtask-name">{{ item.name }}</div>
                </div>
                <div class="subtask-meta-row">
                  <span class="badge badge-yellow">待人工审批</span>
                </div>
              </div>
            </article>
          </template>

          <template v-else>
            <div v-if="task.status === 'planning'" class="empty-state" style="padding: 30px 10px">
              <div class="big">🧠</div>
              <div class="title">技术总监正在拆解需求</div>
              <div class="small">可随时点击“终止执行”或输入人工消息介入。</div>
            </div>

            <div v-else-if="!subtasks.length" class="empty-state" style="padding: 30px 10px">
              <div class="small">暂无子任务</div>
            </div>

            <SubtaskCard
              v-for="sub in subtasks"
              :key="sub.id"
              :subtask="sub"
              :selected="sub.id === selectedSubtaskId"
              :frozen="isGlobal"
              @select="selectedSubtaskId = sub.id"
            />

            <div v-if="isGlobal" class="panel" style="padding: 11px">
              <div class="flex">
                <span class="badge badge-yellow">全局评审收口</span>
                <span class="small muted">队列已冻结</span>
              </div>
              <div class="small muted" style="margin-top: 6px">
                全部模块已完成，正在进行跨模块一致性/安全/命名规范收口。
              </div>
            </div>
          </template>
        </aside>

        <section class="wb-col">
          <!-- 操作台固定在中栏顶部，避免长内容把审批/兜底按钮顶出视野 -->
          <div class="action-dock">
            <template v-if="isPlanApproval">
              <div class="human-bar">
                <div class="tip">只有你同意方案后，Coder 才会开始工作。</div>
                <button class="btn btn-sm btn-primary" :disabled="actionBusy" @click="doApprovePlan">
                  同意方案，开始开发
                </button>
                <button class="btn btn-sm btn-danger" :disabled="actionBusy" @click="openPlanReject">
                  驳回，让技术总监修改
                </button>
              </div>
            </template>
            <template v-else-if="isDesignAwaiting">
              <div class="human-bar">
                <div class="tip">设计文档审批通过后，该模块才会派发给 Coder。</div>
                <button class="btn btn-sm btn-primary" :disabled="actionBusy" @click="doApproveDesign">
                  同意设计，派发 Coder
                </button>
                <button class="btn btn-sm btn-danger" :disabled="actionBusy" @click="openDesignReject">
                  驳回设计
                </button>
              </div>
            </template>
            <template v-else-if="isGlobal">
              <div v-if="task.status === 'needs_human'" class="human-bar">
                <div class="tip">全局评审轮次已耗尽：可强制通过收口，或指定模块返工。</div>
                <button class="btn btn-sm btn-primary" :disabled="actionBusy" @click="doGlobalForcePass">
                  人工强制通过全局评审
                </button>
                <button class="btn btn-sm btn-danger" :disabled="actionBusy" @click="openGlobalRework">
                  人工指定模块返工
                </button>
              </div>
            </template>
            <template v-else-if="!isPlanApproval">
              <div class="human-bar">
                <div class="tip">
                  人工兜底：仅在调度未运行时可用；终止后数据保留，可恢复。
                </div>
                <button
                  class="btn btn-sm btn-primary"
                  :disabled="!canHuman || actionBusy"
                  @click="forcePass"
                >
                  人工强制标记通过
                </button>
                <button
                  class="btn btn-sm btn-danger"
                  :disabled="!canHuman || actionBusy"
                  @click="openRejectModal"
                >
                  人工驳回
                </button>
              </div>
            </template>
          </div>

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
                <div class="t">全局评审收口阶段</div>
                <div class="s">
                  第 {{ task.current_global_round }} / {{ task.global_max_round }} 轮 ·
                  上下文 = 全局契约 + 所有模块摘要 + 全部模块 diff 汇总
                </div>
              </div>
              <span class="badge badge-yellow right">
                {{ task.status === "needs_human" ? "等待人工介入" : "评审中" }}
              </span>
            </div>

            <div v-if="!task.global_review_records.length" class="round-block">
              <div class="body muted">正在汇总全部模块的 summary 与 diff，提交全局评审…</div>
            </div>

            <div
              v-for="rec in task.global_review_records"
              :key="rec.round"
              class="global-round-card"
            >
              <div class="gh">
                <span class="badge badge-blue">全局第 {{ rec.round }} 轮</span>
                <span class="decision" :class="rec.passed ? 'pass' : 'reject'">
                  {{ rec.passed ? "✅ 通过" : "❌ 不通过" }}
                </span>
                <span class="small muted right">{{ rec.created_at }}</span>
              </div>
              <div style="white-space: pre-wrap; color: #c2cbdb; font-size: 12.5px">
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

          <div v-else class="current-panel">
            <div v-if="!selected" class="empty-state">
              <div class="big">⏳</div>
              <div class="title">等待子任务派发</div>
            </div>

            <template v-else>
              <div class="current-head">
                <div>
                  <div class="t">
                    <span class="muted">#{{ selected.sort_index }}</span>
                    {{ selected.subtask_name }}
                  </div>
                  <div class="s">
                    {{ stageLabel }} · 当前轮次 {{ selected.current_round }}/{{ selected.max_round }}
                  </div>
                </div>
                <span
                  class="badge right"
                  :class="{
                    'badge-yellow': selected.status === 'in_progress',
                    'badge-green': selected.status === 'completed',
                    'badge-red': selected.status === 'needs_human'
                  }"
                >
                  {{ subtaskStatusLabel }}
                </span>
              </div>

              <div v-if="selected.design_content" class="round-block">
                <div class="head">
                  模块详细设计（技术总监）
                  <span v-if="isDesignAwaiting" class="badge badge-yellow right">等待人工审批</span>
                </div>
                <div class="body">
                  <pre class="pre-box" style="max-height: 360px">{{ selected.design_content }}</pre>
                </div>
              </div>
              <div v-else-if="selected.current_stage === 'design_generation'" class="round-block">
                <div class="body muted">技术总监正在生成当前模块的详细设计文档…</div>
              </div>


              <div class="round-block">
                <div class="head">
                  本子任务验收标准
                  <span class="right small muted">评审拆分原文</span>
                </div>
                <div class="body" style="white-space: pre-wrap; color: #c2cbdb; font-size: 12.5px">
                  {{ selected.accept_criteria || "（评审未提供）" }}
                </div>
              </div>

              <div v-if="!latestRecord && !isDesignStage" class="round-block">
                <div class="body muted">
                  <template v-if="selected.status === 'waiting'">
                    子任务在队列中，等待前序模块完成。
                  </template>
                  <template v-else-if="selected.current_stage === 'coding'">
                    Coder 正在通过 dsh 编写代码，本轮 diff 完成后展示。
                  </template>
                  <template v-else-if="selected.current_stage === 'summary_generation'">
                    Coder 正在生成 summary.md 模块摘要。
                  </template>
                  <template v-else>等待调度事件…</template>
                </div>
              </div>

              <template v-if="latestRecord && !isDesignStage">
                <div class="round-block">
                  <div class="head">
                    Coder 提交 diff
                    <span class="badge badge-blue" style="margin-left: 6px">
                      第 {{ latestRecord.round }} 轮
                    </span>
                    <span class="right small muted">{{ latestRecord.created_at }}</span>
                  </div>
                  <div class="body">
                    <pre v-if="latestRecord.diff" class="pre-box diff-pre">{{ latestRecord.diff }}</pre>
                    <div v-else class="muted small">本轮没有代码变更（或 diff 为空）。</div>
                  </div>
                </div>

                <div v-if="latestRecord.summary_content" class="round-block">
                  <div class="head">Coder 提交的 summary.md</div>
                  <div class="body">
                    <pre class="pre-box">{{ latestRecord.summary_content }}</pre>
                  </div>
                </div>

                <div class="round-block">
                  <div class="head">
                    Reviewer 评审意见
                    <span class="decision right" :class="latestRecord.review_decision">
                      {{ decisionText(latestRecord.review_decision) }}
                    </span>
                  </div>
                  <div class="body" style="white-space: pre-wrap; color: #c2cbdb">
                    {{ feedbackText(latestRecord) }}
                  </div>
                </div>
              </template>


            </template>
          </div>

          <div v-if="task.status !== 'completed'" class="human-composer">
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
        </section>

        <aside class="wb-col right">
          <div class="wb-section-title">技术栈</div>
          <div class="contract-box">
            <div class="ch" @click="techStackOpen = !techStackOpen">
              <span>{{ techStackOpen ? "▾" : "▸" }}</span>
              <span>技术总监选型</span>
            </div>
            <div v-if="techStackOpen" class="cb">
              <pre v-if="task.tech_stack" class="pre-box" style="max-height: 180px">{{ task.tech_stack }}</pre>
              <div v-else class="small muted">拆解完成后生成。</div>
            </div>
          </div>

          <div class="wb-section-title">全局契约</div>
          <div class="contract-box">
            <div class="ch" @click="contractOpen = !contractOpen">
              <span>{{ contractOpen ? "▾" : "▸" }}</span>
              <span>项目统一规范（全程复用）</span>
            </div>
            <div v-if="contractOpen" class="cb">
              <pre v-if="task.global_contract" class="pre-box" style="max-height: 220px">{{
                task.global_contract
              }}</pre>
              <div v-else class="small muted">拆解完成后生成。</div>
            </div>
          </div>

          <div class="wb-section-title">
            已完成模块摘要
            <span class="count">{{ completedSubtasks.length }} 个</span>
          </div>
          <div v-if="!completedSubtasks.length" class="small muted">
            还没有完成的模块摘要。摘要只作为后续评审上下文，不会携带历史完整对话。
          </div>
          <div v-for="sub in completedSubtasks" :key="sub.id" class="summary-item">
            <div class="sh" @click="toggleSummary(sub.id)">
              <span>{{ openSummaryIds.has(sub.id) ? "▾" : "▸" }}</span>
              <span>#{{ sub.sort_index }} {{ sub.subtask_name }}</span>
              <span class="badge badge-green right">✅</span>
            </div>
            <div v-if="openSummaryIds.has(sub.id)" class="sb">
              <pre v-if="sub.summary_content" class="pre-box" style="max-height: 260px">{{
                sub.summary_content
              }}</pre>
              <div v-else class="small muted">摘要内容为空。</div>
            </div>
          </div>

          <div class="wb-section-title">
            实时执行输出
            <button class="btn btn-sm btn-ghost" style="margin-left: auto" @click="showConsole = true">
              展开全部
            </button>
          </div>
          <div ref="eventBox" class="event-box">
            <div v-if="!visibleEvents.length" class="muted small">暂无执行输出</div>
            <div
              v-for="ev in visibleEvents"
              :key="ev.id"
              class="event-line"
              :class="eventKindClass(ev.kind)"
            >
              <div class="ev-head">
                <span class="ev-kind">{{ eventKindLabel(ev.kind) }}</span>
                <span class="ev-time">{{ ev.create_time.slice(11) }}</span>
                <span v-if="ev.subtask_id" class="ev-sub">#{{ ev.subtask_id }}</span>
              </div>
              <pre class="ev-content">{{ ev.content }}</pre>
            </div>
          </div>

          <div class="wb-section-title">系统事件日志</div>
          <div ref="logBox" class="log-box">
            <div v-if="!detail?.logs.length" class="muted">暂无事件</div>
            <div
              v-for="log in detail?.logs ?? []"
              :key="log.id"
              class="log-line"
              :class="log.level"
            >
              <span class="log-time">{{ log.create_time.slice(11) }}</span>
              <span class="log-msg">{{ log.message }}</span>
            </div>
          </div>

          <div class="wb-section-title">任务信息</div>
          <div class="panel" style="padding: 11px">
            <div class="meta-list">
              <div class="kv"><span>任务 ID</span><span>{{ task.id }}</span></div>
              <div class="kv"><span>工作目录</span><span style="word-break: break-all">{{ task.work_dir }}</span></div>
              <div class="kv"><span>创建时间</span><span>{{ task.create_time }}</span></div>
              <div class="kv"><span>人工审核</span><span>{{ task.manual_review_enabled ? "开启" : "关闭" }}</span></div>
              <div class="kv"><span>子任务数</span><span>{{ subtasks.length }}</span></div>
            </div>
          </div>
        </aside>
      </div>
    </template>

    <div v-else class="empty-state" style="margin: auto">
      <div class="big">⏳</div>
      <div class="title">{{ loading ? "加载任务…" : "任务不存在" }}</div>
    </div>

    <div v-if="showReject" class="modal-mask" @click.self="showReject = false">
      <div class="modal narrow">
        <div class="modal-head">
          人工驳回 · 输入修改意见
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

    <div v-if="showConsole" class="modal-mask" @click.self="showConsole = false">
      <div class="modal console-modal">
        <div class="modal-head">
          全量执行输出
          <span class="small muted" style="margin-left: 10px">{{ executionEvents.length }} 条</span>
          <button class="x" @click="showConsole = false">×</button>
        </div>
        <div class="console-modal-body">
          <div v-if="!executionEvents.length" class="muted">暂无执行输出</div>
          <div
            v-for="ev in executionEvents"
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
