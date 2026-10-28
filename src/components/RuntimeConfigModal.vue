<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import type { AgentConfig, ExecutionMode, RoleType, RuntimeConfigInput, TaskMain } from "../types";
import { listAgents, listRoles } from "../api";
import { capabilityText, roleBadgeStyle, validateWorkflowAgents } from "../roles";
import { errorMessage, showToast } from "../toast";

const props = defineProps<{
  task: TaskMain;
  busy?: boolean;
}>();

const emit = defineEmits<{
  (e: "close"): void;
  (e: "save", input: RuntimeConfigInput): void;
}>();

const agents = ref<AgentConfig[]>([]);
const roles = ref<RoleType[]>([]);
const workflow = ref<number[]>([...(props.task.workflow_agent_ids ?? [])]);
const pickId = ref<number>(0);
const editableWorkflow = computed(() => props.task.status === "pending");

const agentById = computed(() => new Map(agents.value.map((a) => [a.id, a])));
const roleByKey = computed(() => new Map(roles.value.map((r) => [r.key, r])));
const workflowAgents = computed(() =>
  workflow.value.map((id) => agentById.value.get(id)).filter(Boolean) as AgentConfig[]
);
const availableAgents = computed(() => agents.value.filter((a) => !workflow.value.includes(a.id)));
const workflowError = computed(() =>
  editableWorkflow.value ? validateWorkflowAgents(workflowAgents.value) : ""
);

function roleOf(agent: AgentConfig) {
  return roleByKey.value.get(agent.role_type) ?? null;
}

function addStep() {
  if (!pickId.value || workflow.value.includes(pickId.value) || workflow.value.length >= 6) return;
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

onMounted(async () => {
  try {
    const [agentList, roleList] = await Promise.all([listAgents(), listRoles()]);
    agents.value = agentList;
    roles.value = roleList;
    if (!workflow.value.length) {
      workflow.value = props.task.reviewer_agent_id
        ? [props.task.reviewer_agent_id, ...props.task.coder_agent_ids]
        : agentList.slice(0, 2).map((a) => a.id);
    }
    pickId.value = availableAgents.value[0]?.id ?? 0;
  } catch (e) {
    showToast(errorMessage(e), "error");
  }
});

const form = reactive<RuntimeConfigInput>({
  module_max_round: props.task.module_max_round,
  global_max_round: props.task.global_max_round,
  manual_review_enabled: props.task.manual_review_enabled,
  execution_mode: props.task.execution_mode,
  schedule_start: props.task.schedule_start,
  schedule_end: props.task.schedule_end,
  schedule_enabled: props.task.schedule_enabled
});

const modeOptions: { value: ExecutionMode; label: string; desc: string }[] = [
  { value: "immediate", label: "立即执行", desc: "审批/保存后马上开始" },
  { value: "later", label: "稍后执行", desc: "需要手动点击运行" },
  { value: "scheduled", label: "定时执行", desc: "只在指定时间窗口内运行" }
];

const touched = ref(false);

function submit() {
  if (Number(form.module_max_round) < 1) return showToast("单个子任务最大轮次至少为 1", "error");
  if (Number(form.global_max_round) < 1) return showToast("全局评审最大轮次至少为 1", "error");
  if (form.execution_mode === "scheduled" && form.schedule_start === form.schedule_end)
    return showToast("定时窗口开始与结束时间不能相同", "error");
  if (editableWorkflow.value && workflowError.value)
    return showToast(workflowError.value, "error");
  const input: RuntimeConfigInput = {
    module_max_round: Number(form.module_max_round),
    global_max_round: Number(form.global_max_round),
    manual_review_enabled: form.manual_review_enabled,
    execution_mode: form.execution_mode,
    schedule_start: form.schedule_start,
    schedule_end: form.schedule_end,
    schedule_enabled: form.schedule_enabled
  };
  if (editableWorkflow.value) {
    input.workflow_agent_ids = [...workflow.value];
  }
  emit("save", input);
}
</script>

<template>
  <div class="modal-mask" @click.self="emit('close')">
    <div class="modal config-modal" @keydown.esc="emit('close')">
      <div class="modal-head">
        任务运行配置
        <button class="x" @click="emit('close')">×</button>
      </div>

      <div class="modal-body">
        <div class="config-hint">
          修改后立即生效，不需要停止任务。取消不会保存任何改动。
        </div>

        <div class="config-group">
          <div class="config-group-title">工作流</div>
          <div v-if="!editableWorkflow" class="config-hint" style="margin-bottom: 8px">
            任务已启动，工作流只读；如需调整请在任务运行前修改。
          </div>
          <div class="workflow-steps compact">
            <div
              v-for="(agent, index) in workflowAgents"
              :key="agent.id"
              class="workflow-step"
            >
              <span class="wf-order">{{ index + 1 }}</span>
              <span class="badge wf-role" :style="roleBadgeStyle(roleOf(agent)?.color ?? agent.role_color)">
                {{ roleOf(agent)?.name ?? agent.role_type }}
              </span>
              <span class="wf-agent">{{ agent.name }}</span>
              <span class="wf-caps">{{ capabilityText(agent.role_capabilities) }}</span>
              <span v-if="editableWorkflow" class="wf-actions">
                <button class="link-btn" :disabled="index === 0" @click="moveStep(index, -1)">上移</button>
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
          <div v-if="editableWorkflow" class="workflow-add" style="margin-top: 8px">
            <select v-model.number="pickId" :disabled="!availableAgents.length">
              <option v-if="!availableAgents.length" :value="0">没有可添加的 Agent</option>
              <option v-for="agent in availableAgents" :key="agent.id" :value="agent.id">
                {{ agent.name }}（{{ capabilityText(agent.role_capabilities) }}）
              </option>
            </select>
            <button class="btn btn-sm" :disabled="!availableAgents.length || workflow.length >= 6" @click="addStep">
              ＋ 添加步骤
            </button>
          </div>
          <div v-if="editableWorkflow" class="workflow-status">
            <span class="small muted">必需能力：</span>
            <span class="cap-chip ok">方案设计</span>
            <span class="cap-chip ok">编码实现</span>
            <span class="cap-chip ok">评审验收</span>
            <span v-if="workflowError" class="error-text">{{ workflowError }}</span>
            <span v-else class="ok-text">✓ 可保存</span>
          </div>
        </div>

        <div class="config-group">
          <div class="config-group-title">评审轮次</div>
          <div class="config-grid">
            <div class="field">
              <label>单个子任务最大轮次</label>
              <input
                v-model.number="form.module_max_round"
                type="number"
                min="1"
                max="99"
                @change="touched = true"
              />
              <div class="hint">同一模块「Coder 修改 → 评审」的最大往返次数</div>
            </div>
            <div class="field">
              <label>全局评审最大轮次</label>
              <input
                v-model.number="form.global_max_round"
                type="number"
                min="1"
                max="99"
                @change="touched = true"
              />
              <div class="hint">所有模块完成后，跨模块收口评审的最大轮次</div>
            </div>
          </div>
        </div>

        <div class="config-group">
          <div class="config-group-title">执行方式</div>
          <div class="config-modes">
            <label
              v-for="option in modeOptions"
              :key="option.value"
              class="mode-card"
              :class="{ active: form.execution_mode === option.value }"
            >
              <input v-model="form.execution_mode" type="radio" :value="option.value" />
              <b>{{ option.label }}</b>
              <small>{{ option.desc }}</small>
            </label>
          </div>

          <div v-if="form.execution_mode === 'scheduled'" class="config-grid" style="margin-top: 10px">
            <div class="field">
              <label>定时窗口开始</label>
              <input v-model="form.schedule_start" type="time" />
            </div>
            <div class="field">
              <label>定时窗口结束</label>
              <input v-model="form.schedule_end" type="time" />
            </div>
            <div class="field config-check-field">
              <label class="flex" style="cursor: pointer">
                <input v-model="form.schedule_enabled" type="checkbox" />
                启用定时窗口
              </label>
            </div>
          </div>
        </div>

        <div class="config-group">
          <div class="config-group-title">人工审核</div>
          <label class="config-check">
            <input v-model="form.manual_review_enabled" type="checkbox" />
            <span>
              人工审核拆解方案与模块设计
              <small>开启后，方案和每个模块的设计文档都需要你确认，才会继续开发。</small>
            </span>
          </label>
        </div>
      </div>

      <div class="modal-foot">
        <span v-if="touched" class="small muted" style="margin-right: auto">有未保存的改动</span>
        <button class="btn btn-ghost" @click="emit('close')">取消</button>
        <button class="btn btn-primary" :disabled="busy" @click="submit">
          {{ busy ? "保存中…" : "保存配置" }}
        </button>
      </div>
    </div>
  </div>
</template>
