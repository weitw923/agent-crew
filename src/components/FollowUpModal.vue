<script setup lang="ts">
import { computed, ref } from "vue";
import type { FollowupMode, TaskMain } from "../types";
import { appendRequirement } from "../api";
import { errorMessage, showToast } from "../toast";

const props = defineProps<{
  task: TaskMain;
  subtaskCount?: number;
  /** 当前最新迭代版本，用于预估本次追加后的版本号。 */
  currentVersion?: string;
}>();
const emit = defineEmits<{
  (e: "close"): void;
  (e: "queued", taskId: number): void;
}>();

const requirement = ref("");
const mode = ref<FollowupMode>("auto");
const busy = ref(false);

const charCount = computed(() => requirement.value.trim().length);

/** 预估版本号：默认小版本 +1；若拆分超过 5 个子任务则升大版本。 */
const predictedVersion = computed(() => {
  const raw = (props.currentVersion ?? "").trim();
  const matched = /^dev-(\d+)\.(\d+)$/.exec(raw);
  if (!matched) return { minor: "", major: "" };
  const major = Number(matched[1]);
  const minor = Number(matched[2]);
  return { minor: `dev-${major}.${minor + 1}`, major: `dev-${major + 1}.0` };
});
const canSubmit = computed(() => !!requirement.value.trim() && charCount.value <= 12000 && !busy.value);

async function submit() {
  const text = requirement.value.trim();
  if (!text) return showToast("请先输入新的需求或问题", "error");
  if (text.length > 12000) return showToast("需求过长，请控制在 12000 字以内", "error");
  busy.value = true;
  try {
    await appendRequirement(props.task.id, text, mode.value);
    showToast("已提交给评审，正在梳理本次增量需求…", "success");
    emit("queued", props.task.id);
    emit("close");
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <div class="modal-mask" @click.self="emit('close')">
    <div class="modal followup-modal">
      <div class="modal-head">
        追加需求 · 继续迭代
        <button class="x" @click="emit('close')">×</button>
      </div>
      <div class="modal-body">
        <div class="followup-context">
          <span class="badge badge-green">✅ 任务已完成</span>
          <b>{{ task.task_name }}</b>
          <span v-if="subtaskCount !== undefined" class="small muted">
            已有 {{ subtaskCount }} 个模块会保留，不会重跑
          </span>
        </div>

        <div class="field" style="margin-top: 16px">
          <label class="required">新的问题或需求</label>
          <textarea
            v-model="requirement"
            rows="7"
            autofocus
            placeholder="例如：给日历页增加“导出当日计划”的按钮，导出为 Markdown；或者：修复统计页在空数据时报错的问题。"
            @keydown.ctrl.enter.prevent="submit"
          ></textarea>
          <div class="followup-counter" :class="{ over: charCount > 12000 }">
            {{ charCount }} / 12000 · Ctrl + Enter 提交
          </div>
        </div>

        <div v-if="predictedVersion.minor" class="followup-version-tip">
          当前版本 <b>{{ currentVersion }}</b> →
          默认生成 <b>{{ predictedVersion.minor }}</b>；
          若本次拆出超过 5 个子任务，将自动升为大版本 <b>{{ predictedVersion.major }}</b>。
        </div>

        <div class="field" style="margin-top: 14px">
          <label>处理方式</label>
          <div class="mode-cards followup-modes">
            <label class="mode-card" :class="{ active: mode === 'auto' }">
              <input v-model="mode" type="radio" value="auto" />
              <b>🧠 自动判断（推荐）</b>
              <small>评审先梳理需求：简单改动只创建 1 个子任务，复杂需求才拆成少量模块。</small>
            </label>
            <label class="mode-card" :class="{ active: mode === 'single' }">
              <input v-model="mode" type="radio" value="single" />
              <b>⚡ 单模块快改</b>
              <small>强制只创建一个子任务，适合小功能、修 bug、样式调整，避免任何过度拆分。</small>
            </label>
          </div>
        </div>

        <div class="followup-note">
          评审会沿用现有技术栈、全局契约和已完成模块的接口，不会重新拆解整个项目。
          <template v-if="mode === 'single'">本次会强制输出 1 个子任务。</template>
          <template v-else>简单改动默认只给 1 个子任务，只有确实包含多个独立交付物时才拆分。</template>
        </div>
      </div>
      <div class="modal-foot">
        <button class="btn btn-ghost" :disabled="busy" @click="emit('close')">取消</button>
        <button class="btn btn-primary" :disabled="!canSubmit" @click="submit">
          {{ busy ? "提交中…" : "提交给评审并继续" }}
        </button>
      </div>
    </div>
  </div>
</template>
