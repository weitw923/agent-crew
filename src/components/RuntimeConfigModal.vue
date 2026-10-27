<script setup lang="ts">
import { reactive, ref } from "vue";
import type { ExecutionMode, RuntimeConfigInput, TaskMain } from "../types";
import { showToast } from "../toast";

const props = defineProps<{
  task: TaskMain;
  busy?: boolean;
}>();

const emit = defineEmits<{
  (e: "close"): void;
  (e: "save", input: RuntimeConfigInput): void;
}>();

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
  emit("save", {
    module_max_round: Number(form.module_max_round),
    global_max_round: Number(form.global_max_round),
    manual_review_enabled: form.manual_review_enabled,
    execution_mode: form.execution_mode,
    schedule_start: form.schedule_start,
    schedule_end: form.schedule_end,
    schedule_enabled: form.schedule_enabled
  });
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
