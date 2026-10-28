<script setup lang="ts">
import { ref } from "vue";
import type { TaskMain } from "../types";
import { updateRequirements } from "../api";
import { errorMessage, showToast } from "../toast";

const props = defineProps<{ task: TaskMain }>();
const emit = defineEmits<{
  (e: "close"): void;
  (e: "saved", task: TaskMain): void;
}>();

const requirement = ref(props.task.top_requirement);
const acceptance = ref(props.task.global_accept_criteria);
const busy = ref(false);

async function save() {
  if (!requirement.value.trim()) return showToast("顶层需求不能为空", "error");
  if (!acceptance.value.trim()) return showToast("项目验收标准不能为空", "error");
  busy.value = true;
  try {
    const task = await updateRequirements(
      props.task.id,
      requirement.value.trim(),
      acceptance.value.trim()
    );
    showToast("需求已更新，将在下一个 Agent 步骤注入", "success");
    emit("saved", task);
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
    <div class="modal">
      <div class="modal-head">
        运行中修改需求与验收标准
        <button class="x" @click="emit('close')">×</button>
      </div>
      <div class="modal-body">
        <div class="field">
          <label class="required">最新顶层需求</label>
          <textarea v-model="requirement" rows="8"></textarea>
        </div>
        <div class="field" style="margin-top: 14px">
          <label class="required">最新项目验收标准</label>
          <textarea v-model="acceptance" rows="5"></textarea>
        </div>
        <div class="small muted" style="margin-top: 10px">
          保存后，新需求和验收标准会作为人工消息注入下一个 Agent 步骤；不会删除已有子任务、代码或历史记录。
        </div>
      </div>
      <div class="modal-foot">
        <button class="btn btn-ghost" @click="emit('close')">取消</button>
        <button class="btn btn-primary" :disabled="busy" @click="save">
          {{ busy ? "保存中…" : "保存并注入" }}
        </button>
      </div>
    </div>
  </div>
</template>
