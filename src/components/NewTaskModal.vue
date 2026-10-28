<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref } from "vue";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type { AgentConfig, Capability, ExecutionMode, RoleType } from "../types";
import { createTask, getSettings, listAgents, listRoles, startTask } from "../api";
import {
  CAPABILITY_LABELS,
  capabilityText,
  roleBadgeStyle,
  roleName
} from "../roles";
import { errorMessage, showToast } from "../toast";

const emit = defineEmits<{
  (e: "close"): void;
  (e: "created", taskId: number): void;
}>();

const agents = ref<AgentConfig[]>([]);
const roles = ref<RoleType[]>([]);
const submitting = ref(false);

const form = reactive({
  task_name: "",
  top_requirement: "",
  global_accept_criteria: "",
  module_max_round: 3,
  global_max_round: 2,
  work_dir: "",
  manual_review_enabled: false,
  execution_mode: "later" as ExecutionMode,
  schedule_start: "01:00",
  schedule_end: "06:00",
  schedule_enabled: true
});

/** 线性工作流：按顺序的 Agent id。 */
const workflow = ref<number[]>([]);
const pickId = ref<number>(0);

const agentById = computed(() => new Map(agents.value.map((a) => [a.id, a])));
const roleByKey = computed(() => new Map(roles.value.map((r) => [r.key, r])));
const workflowAgents = computed(() =>
  workflow.value.map((id) => agentById.value.get(id)).filter(Boolean) as AgentConfig[]
);
const availableAgents = computed(() =>
  agents.value.filter((a) => !workflow.value.includes(a.id))
);
const groupedAvailable = computed(() =>
  roles.value
    .map((role) => ({
      role,
      items: availableAgents.value.filter((a) => a.role_type === role.key)
    }))
    .filter((g) => g.items.length)
);

const workflowCaps = computed<Capability[]>(() => {
  const caps: Capability[] = [];
  for (const agent of workflowAgents.value) {
    for (const cap of agent.role_capabilities ?? []) {
      if (!caps.includes(cap)) caps.push(cap);
    }
  }
  return caps;
});

const missingCaps = computed(() =>
  (["plan", "code", "review"] as Capability[]).filter((cap) => !workflowCaps.value.includes(cap))
);

const workflowError = computed(() => {
  if (workflow.value.length < 2) return "工作流至少需要 2 个 Agent（例如 评审者 → 编码者）";
  if (workflow.value.length > 6) return "工作流最多支持 6 个 Agent";
  if (missingCaps.value.length) {
    return `工作流缺少「${missingCaps.value
      .map((c) => CAPABILITY_LABELS[c])
      .join("、")}」能力的 Agent`;
  }
  return "";
});

function roleOf(agent: AgentConfig) {
  return roleByKey.value.get(agent.role_type) ?? null;
}

function defaultWorkflow(list: AgentConfig[]) {
  const picks: number[] = [];
  const push = (agent?: AgentConfig) => {
    if (agent && !picks.includes(agent.id)) picks.push(agent.id);
  };
  const has = (agent: AgentConfig, cap: Capability) =>
    (agent.role_capabilities ?? []).includes(cap);
  // 优先把「只做需求梳理」的角色（如产品经理）放在最前面。
  push(
    list.find(
      (a) =>
        has(a, "requirement") &&
        !has(a, "plan") &&
        !has(a, "code") &&
        !has(a, "review")
    )
  );
  push(list.find((a) => has(a, "plan")));
  push(list.find((a) => has(a, "code")));
  push(list.find((a) => has(a, "review")));
  return picks;
}

onMounted(async () => {
  try {
    const [agentList, roleList, settings] = await Promise.all([
      listAgents(),
      listRoles(),
      getSettings()
    ]);
    agents.value = agentList;
    roles.value = roleList;
    if (!form.work_dir) form.work_dir = settings.default_work_dir || "";
    form.execution_mode = settings.default_execution_mode || "later";
    form.module_max_round = Number(settings.default_module_max_round) || 3;
    form.global_max_round = Number(settings.default_global_max_round) || 2;
    workflow.value = defaultWorkflow(agentList);
    pickId.value = availableAgents.value[0]?.id ?? 0;
  } catch (e) {
    showToast(errorMessage(e), "error");
  }
});

/* ---------------- 工作流编辑：添加 / 移除 / 排序 ---------------- */
function addAgent() {
  if (!pickId.value || workflow.value.includes(pickId.value)) return;
  if (workflow.value.length >= 6) return showToast("工作流最多 6 个 Agent", "error");
  workflow.value.push(pickId.value);
  pickId.value = availableAgents.value[0]?.id ?? 0;
}

function removeStep(index: number) {
  workflow.value.splice(index, 1);
  pickId.value = availableAgents.value[0]?.id ?? 0;
}

function moveStep(index: number, delta: number) {
  const target = index + delta;
  if (target < 0 || target >= workflow.value.length) return;
  const [item] = workflow.value.splice(index, 1);
  workflow.value.splice(target, 0, item);
}

/* 指针拖拽排序：按住手柄上下拖动，实时交换位置。 */
const stepEls = ref<HTMLElement[]>([]);
const dragIndex = ref<number | null>(null);

function setStepRef(el: Element | null, index: number) {
  if (el instanceof HTMLElement) stepEls.value[index] = el;
}

function onDragStart(index: number, event: PointerEvent) {
  dragIndex.value = index;
  (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
  document.body.classList.add("wf-dragging");
  window.addEventListener("pointermove", onDragMove);
  window.addEventListener("pointerup", onDragEnd);
  window.addEventListener("pointercancel", onDragEnd);
}

function onDragMove(event: PointerEvent) {
  if (dragIndex.value == null) return;
  const y = event.clientY;
  const target = stepEls.value.findIndex((el) => {
    if (!el) return false;
    const rect = el.getBoundingClientRect();
    return y >= rect.top && y <= rect.bottom;
  });
  if (target >= 0 && target !== dragIndex.value) {
    const [item] = workflow.value.splice(dragIndex.value, 1);
    workflow.value.splice(target, 0, item);
    dragIndex.value = target;
  }
}

function onDragEnd() {
  dragIndex.value = null;
  document.body.classList.remove("wf-dragging");
  window.removeEventListener("pointermove", onDragMove);
  window.removeEventListener("pointerup", onDragEnd);
  window.removeEventListener("pointercancel", onDragEnd);
}

onBeforeUnmount(onDragEnd);

async function pickDir() {
  try {
    const dir = await openDialog({ directory: true, multiple: false, title: "选择项目本地工作目录" });
    if (typeof dir === "string") form.work_dir = dir;
  } catch (e) {
    showToast(errorMessage(e), "error");
  }
}

async function submit() {
  if (!form.task_name.trim()) return showToast("请填写任务名称", "error");
  if (!form.top_requirement.trim()) return showToast("请填写顶层需求", "error");
  if (!form.global_accept_criteria.trim()) return showToast("请填写项目验收标准", "error");
  if (workflowError.value) return showToast(workflowError.value, "error");
  if (!form.work_dir.trim()) return showToast("请选择项目本地工作目录", "error");
  if (form.module_max_round < 1 || form.global_max_round < 1) {
    return showToast("轮次上限必须大于等于 1", "error");
  }
  if (form.execution_mode === "scheduled" && form.schedule_start === form.schedule_end) {
    return showToast("定时窗口开始时间和结束时间不能相同", "error");
  }

  // 旧字段按能力推导，保证与旧版本/回滚兼容。
  const reviewer =
    workflowAgents.value.find((a) => (a.role_capabilities ?? []).includes("plan")) ??
    workflowAgents.value.find((a) => (a.role_capabilities ?? []).includes("review")) ??
    workflowAgents.value[0];
  const coders = workflowAgents.value
    .filter((a) => (a.role_capabilities ?? []).includes("code"))
    .map((a) => a.id);

  submitting.value = true;
  try {
    const id = await createTask({
      task_name: form.task_name.trim(),
      top_requirement: form.top_requirement,
      global_accept_criteria: form.global_accept_criteria,
      reviewer_agent_id: reviewer?.id ?? 0,
      coder_agent_ids: coders,
      workflow_agent_ids: [...workflow.value],
      module_max_round: form.module_max_round,
      global_max_round: form.global_max_round,
      work_dir: form.work_dir.trim(),
      manual_review_enabled: form.manual_review_enabled,
      execution_mode: form.execution_mode,
      schedule_start: form.schedule_start,
      schedule_end: form.schedule_end,
      schedule_enabled: form.schedule_enabled
    });
    if (form.execution_mode === "immediate") {
      const outcome = await startTask(id);
      showToast(outcome.message, outcome.started ? "success" : "error");
    } else if (form.execution_mode === "scheduled") {
      showToast("任务已创建，将在每日定时窗口内自动执行", "success");
    } else {
      showToast("任务已创建，点击【运行】后才会开始", "success");
    }
    emit("created", id);
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    submitting.value = false;
  }
}
</script>

<template>
  <div class="modal-mask" @click.self="emit('close')">
    <div class="modal task-modal">
      <div class="modal-head">
        新建编排任务
        <button class="x" @click="emit('close')">×</button>
      </div>
      <div class="modal-body">
        <div class="form-grid">
          <div class="field span-2">
            <label class="required">任务名称</label>
            <input v-model="form.task_name" type="text" placeholder="例如：内部工单系统 MVP" />
          </div>

          <div class="field span-2">
            <label class="required">顶层业务需求</label>
            <textarea
              v-model="form.top_requirement"
              rows="6"
              placeholder="描述要开发的完整业务系统/功能。工作流中的方案 Agent 会据此拆解为有序子任务。"
            ></textarea>
          </div>

          <div class="field span-2">
            <label class="required">项目验收标准</label>
            <textarea
              v-model="form.global_accept_criteria"
              rows="4"
              placeholder="整个项目最终需要满足的验收标准。"
            ></textarea>
          </div>

          <!-- ============ 线性工作流 ============ -->
          <div class="field span-2">
            <label class="required">工作流（按顺序执行，可拖动排序）</label>
            <div class="workflow-editor">
              <div v-if="!workflow.length" class="workflow-empty">
                还没有选择 Agent。先到「Agent 配置」创建 Agent，再从下方添加。
              </div>
              <div v-else class="workflow-steps">
                <div
                  v-for="(agent, index) in workflowAgents"
                  :key="agent.id"
                  :ref="(el) => setStepRef(el as Element | null, index)"
                  class="workflow-step"
                  :class="{ dragging: dragIndex === index }"
                >
                  <span
                    class="wf-handle"
                    title="按住拖动调整顺序"
                    @pointerdown.prevent="onDragStart(index, $event)"
                  >
                    ⠿
                  </span>
                  <span class="wf-order">{{ index + 1 }}</span>
                  <span
                    class="badge wf-role"
                    :style="roleBadgeStyle(roleOf(agent)?.color ?? agent.role_color)"
                  >
                    {{ roleOf(agent)?.name ?? roleName(roleOf(agent)) }}
                  </span>
                  <span class="wf-agent">{{ agent.name }}</span>
                  <span class="wf-caps">{{ capabilityText(agent.role_capabilities) }}</span>
                  <span class="wf-actions">
                    <button class="link-btn" :disabled="index === 0" @click="moveStep(index, -1)">
                      上移
                    </button>
                    <button
                      class="link-btn"
                      :disabled="index === workflow.length - 1"
                      @click="moveStep(index, 1)"
                    >
                      下移
                    </button>
                    <button class="link-btn danger" @click="removeStep(index)">移除</button>
                  </span>
                </div>
              </div>

              <div class="workflow-add">
                <select v-model.number="pickId" :disabled="!availableAgents.length">
                  <option v-if="!availableAgents.length" :value="0">没有可添加的 Agent</option>
                  <optgroup
                    v-for="group in groupedAvailable"
                    :key="group.role.key"
                    :label="group.role.name"
                  >
                    <option v-for="agent in group.items" :key="agent.id" :value="agent.id">
                      {{ agent.name }}（{{ capabilityText(agent.role_capabilities) }}）
                    </option>
                  </optgroup>
                </select>
                <button
                  class="btn btn-sm"
                  :disabled="!availableAgents.length || workflow.length >= 6"
                  @click="addAgent"
                >
                  ＋ 添加步骤
                </button>
              </div>

              <div class="workflow-status">
                <span class="small muted">已具备能力：</span>
                <span
                  v-for="cap in workflowCaps"
                  :key="cap"
                  class="cap-chip"
                  :class="{ ok: workflowCaps.includes(cap) }"
                >
                  {{ CAPABILITY_LABELS[cap] }}
                </span>
                <span v-if="missingCaps.length" class="error-text">
                  缺少：{{ missingCaps.map((c) => CAPABILITY_LABELS[c]).join("、") }}
                </span>
                <span v-else class="ok-text">✓ 方案 / 编码 / 评审能力齐全</span>
              </div>
              <div v-if="workflowError" class="workflow-error">{{ workflowError }}</div>
              <span class="hint">
                执行时按能力标签解析：需求梳理 → 方案设计 → 编码实现 → 评审验收；
                上游产物自动注入下游。评审不通过只会在「编码 ↔ 评审」之间循环。
              </span>
            </div>
            <span v-if="!agents.length" class="error-text">
              还没有任何 Agent，请先到「Agent 配置」创建。
            </span>
          </div>

          <div class="field span-2">
            <label class="required">执行模式</label>
            <div class="mode-cards">
              <label class="mode-card" :class="{ active: form.execution_mode === 'immediate' }">
                <input v-model="form.execution_mode" type="radio" value="immediate" />
                <b>⚡ 立即执行</b>
                <small>创建后自动启动工作流拆解与开发。</small>
              </label>
              <label class="mode-card" :class="{ active: form.execution_mode === 'later' }">
                <input v-model="form.execution_mode" type="radio" value="later" />
                <b>⏸ 稍后执行</b>
                <small>创建后进入待运行，手动点击【运行】才开始。</small>
              </label>
              <label class="mode-card" :class="{ active: form.execution_mode === 'scheduled' }">
                <input v-model="form.execution_mode" type="radio" value="scheduled" />
                <b>🕒 定时执行</b>
                <small>只在每日指定时间窗口内运行，窗口外自动暂停。</small>
              </label>
            </div>
          </div>

          <div v-if="form.execution_mode === 'scheduled'" class="field span-2">
            <label class="required">定时执行窗口（本地时间）</label>
            <div class="flex">
              <input v-model="form.schedule_start" type="time" style="max-width: 160px" />
              <span class="muted">至</span>
              <input v-model="form.schedule_end" type="time" style="max-width: 160px" />
              <label class="flex" style="margin-left: 10px; cursor: pointer">
                <input v-model="form.schedule_enabled" type="checkbox" />
                <span>启用定时窗口</span>
              </label>
            </div>
            <span class="hint">
              例如 01:00–06:00。窗口结束时，已开始模块的当前轮次允许跑完，新模块等待下一个窗口。
              支持跨午夜窗口。
            </span>
          </div>

          <div class="field">
            <label class="required">单个子任务最大迭代轮次</label>
            <input v-model.number="form.module_max_round" type="number" min="1" max="99" />
            <span class="hint">代码返工与摘要返工都计入该子任务轮次。</span>
          </div>

          <div class="field">
            <label class="required">全局评审最大轮次</label>
            <input v-model.number="form.global_max_round" type="number" min="1" max="99" />
            <span class="hint">全局收口未通过、触发模块返工后，消耗全局轮次。</span>
          </div>

          <div class="field span-2">
            <label class="flex" style="cursor: pointer">
              <input v-model="form.manual_review_enabled" type="checkbox" />
              <span>开启人工审核模式（方案 + 每个模块设计文档都需要人工同意）</span>
            </label>
            <span class="hint">
              开启后：方案 Agent 拆解完模块与技术栈先等待你审批；每个模块的详细设计文档也要逐一同意，编码 Agent 才能开始写代码。
            </span>
          </div>

          <div class="field span-2">
            <label class="required">项目本地工作目录</label>
            <div class="flex">
              <input v-model="form.work_dir" type="text" placeholder="/Users/you/Projects/my-app" />
              <button class="btn" @click="pickDir">选择目录</button>
            </div>
            <span class="hint">
              编码 Agent 由 dsh 在该目录内用 read_file/write/bash/git 工具真实写代码；应用会自动建立 git 基线用于计算 diff。
            </span>
          </div>
        </div>
      </div>
      <div class="modal-foot">
        <button class="btn btn-ghost" @click="emit('close')">取消</button>
        <button
          class="btn btn-primary"
          :disabled="submitting || !!workflowError"
          @click="submit"
        >
          {{ submitting ? "创建中…" : "创建任务" }}
        </button>
      </div>
    </div>
  </div>
</template>
