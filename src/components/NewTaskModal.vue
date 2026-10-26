<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type { AgentConfig, ExecutionMode } from "../types";
import { createTask, getSettings, listAgents, startTask } from "../api";
import { errorMessage, showToast } from "../toast";

const emit = defineEmits<{
  (e: "close"): void;
  (e: "created", taskId: number): void;
}>();

const agents = ref<AgentConfig[]>([]);
const submitting = ref(false);

const form = reactive({
  task_name: "",
  top_requirement: "",
  global_accept_criteria: "",
  reviewer_agent_id: 0,
  coder_agent_ids: [] as number[],
  module_max_round: 3,
  global_max_round: 2,
  work_dir: "",
  manual_review_enabled: false,
  execution_mode: "later" as ExecutionMode,
  schedule_start: "01:00",
  schedule_end: "06:00",
  schedule_enabled: true
});

const reviewers = computed(() => agents.value.filter((a) => a.role_type === "reviewer"));
const coders = computed(() => agents.value.filter((a) => a.role_type === "coder"));

onMounted(async () => {
  try {
    const [list, settings] = await Promise.all([listAgents(), getSettings()]);
    agents.value = list;
    if (!form.work_dir) form.work_dir = settings.default_work_dir || "";
    form.execution_mode = settings.default_execution_mode || "later";
    form.module_max_round = Number(settings.default_module_max_round) || 3;
    form.global_max_round = Number(settings.default_global_max_round) || 2;
    form.reviewer_agent_id = reviewers.value[0]?.id ?? 0;
    form.coder_agent_ids = coders.value.slice(0, 1).map((c) => c.id);
  } catch (e) {
    showToast(errorMessage(e), "error");
  }
});

function toggleCoder(id: number) {
  const idx = form.coder_agent_ids.indexOf(id);
  if (idx >= 0) form.coder_agent_ids.splice(idx, 1);
  else form.coder_agent_ids.push(id);
}

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
  if (!form.reviewer_agent_id) return showToast("请选择一个评审者 Agent", "error");
  if (!form.coder_agent_ids.length) return showToast("请至少选择一个 Coder", "error");
  if (!form.work_dir.trim()) return showToast("请选择项目本地工作目录", "error");
  if (form.module_max_round < 1 || form.global_max_round < 1) {
    return showToast("轮次上限必须大于等于 1", "error");
  }
  if (form.execution_mode === "scheduled" && form.schedule_start === form.schedule_end) {
    return showToast("定时窗口开始时间和结束时间不能相同", "error");
  }

  submitting.value = true;
  try {
    const id = await createTask({
      task_name: form.task_name.trim(),
      top_requirement: form.top_requirement,
      global_accept_criteria: form.global_accept_criteria,
      reviewer_agent_id: form.reviewer_agent_id,
      coder_agent_ids: [...form.coder_agent_ids],
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
              placeholder="描述要开发的完整业务系统/功能。评审 Agent 会据此拆解为有序子任务。"
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

          <div class="field">
            <label class="required">评审 Agent</label>
            <select v-model.number="form.reviewer_agent_id">
              <option :value="0" disabled>请选择评审者</option>
              <option v-for="r in reviewers" :key="r.id" :value="r.id">
                {{ r.name }}（{{ r.model_name }}）
              </option>
            </select>
            <span v-if="!reviewers.length" class="error-text">
              还没有 Reviewer，请先到 Agent 配置页创建。
            </span>
          </div>

          <div class="field">
            <label class="required">Coder（可多选）</label>
            <div
              v-if="coders.length"
              style="
                border: 1px solid var(--line);
                border-radius: 8px;
                padding: 8px;
                background: var(--bg-soft);
                max-height: 126px;
                overflow: auto;
              "
            >
              <label
                v-for="c in coders"
                :key="c.id"
                class="flex"
                style="padding: 4px 2px; cursor: pointer"
              >
                <input
                  type="checkbox"
                  :checked="form.coder_agent_ids.includes(c.id)"
                  @change="toggleCoder(c.id)"
                />
                <span>{{ c.name }}</span>
                <span class="badge badge-coder">{{ c.model_name }}</span>
              </label>
            </div>
            <span v-else class="error-text">还没有 Coder，请先到 Agent 配置页创建。</span>
            <span class="hint">
              串行模式：所有子任务固定交给所选列表中的<b style="color: var(--yellow)">第一个 Coder</b>。
            </span>
          </div>

          <div class="field span-2">
            <label class="required">执行模式</label>
            <div class="mode-cards">
              <label class="mode-card" :class="{ active: form.execution_mode === 'immediate' }">
                <input v-model="form.execution_mode" type="radio" value="immediate" />
                <b>⚡ 立即执行</b>
                <small>创建后自动启动管理者 Agent 拆解与开发。</small>
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
              开启后：技术总监拆解完模块与技术栈先等待你审批；每个模块的详细设计文档也要逐一同意，Coder 才能开始写代码。
            </span>
          </div>

          <div class="field span-2">
            <label class="required">项目本地工作目录</label>
            <div class="flex">
              <input v-model="form.work_dir" type="text" placeholder="/Users/you/Projects/my-app" />
              <button class="btn" @click="pickDir">选择目录</button>
            </div>
            <span class="hint">
              Coder 由 dsh 在该目录内用 read_file/write/bash/git 工具真实写代码；应用会自动建立 git 基线用于计算 diff。
            </span>
          </div>
        </div>
      </div>
      <div class="modal-foot">
        <button class="btn btn-ghost" @click="emit('close')">取消</button>
        <button class="btn btn-primary" :disabled="submitting" @click="submit">
          {{ submitting ? "创建中…" : "创建任务" }}
        </button>
      </div>
    </div>
  </div>
</template>
