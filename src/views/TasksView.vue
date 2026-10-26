<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import NewTaskModal from "../components/NewTaskModal.vue";
import type { TaskSummary } from "../types";
import { deleteTask, listTasks } from "../api";
import { errorMessage, showToast } from "../toast";

const emit = defineEmits<{ (e: "open-task", id: number): void }>();

const tasks = ref<TaskSummary[]>([]);
const loading = ref(false);
const showNew = ref(false);
let unlisten: UnlistenFn | null = null;

async function load() {
  loading.value = true;
  try {
    tasks.value = await listTasks();
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    loading.value = false;
  }
}

const statusMeta: Record<string, { label: string; cls: string }> = {
  planning: { label: "拆解中", cls: "badge-blue" },
  running: { label: "执行中", cls: "badge-yellow" },
  needs_human: { label: "需人工介入", cls: "badge-red" },
  paused: { label: "已终止（可恢复）", cls: "badge-yellow" },
  completed: { label: "已完成", cls: "badge-green" },
  failed: { label: "执行失败", cls: "badge-red" }
};

function statusOf(s: string) {
  return statusMeta[s] ?? { label: s, cls: "" };
}

async function remove(task: TaskSummary) {
  if (!window.confirm(`确定删除任务「${task.task_name}」及其全部子任务/迭代记录？`)) return;
  try {
    await deleteTask(task.id);
    await load();
    showToast("任务已删除", "success");
  } catch (e) {
    showToast(errorMessage(e), "error");
  }
}

function onCreated(taskId: number) {
  showNew.value = false;
  emit("open-task", taskId);
}

onMounted(async () => {
  await load();
  unlisten = await listen<{ task_id: number }>("task-updated", () => {
    void load();
  });
});

onUnmounted(() => unlisten?.());
</script>

<template>
  <div class="view-scroll">
    <div class="view-inner">
      <div class="page-head">
        <div>
          <h1>任务列表</h1>
          <p>历史编排任务与运行状态；点击卡片进入三栏工作台。</p>
        </div>
        <div class="spacer"></div>
        <button class="btn" @click="load">{{ loading ? "刷新中…" : "刷新" }}</button>
        <button class="btn btn-primary" @click="showNew = true">＋ 新建任务</button>
      </div>

      <div v-if="!tasks.length" class="panel">
        <div class="empty-state">
          <div class="big">🗂️</div>
          <div class="title">还没有任务</div>
          <div class="small">新建任务后，评审 Agent 会自动拆解模块并驱动 Coder 串行开发。</div>
          <div style="margin-top: 14px">
            <button class="btn btn-primary" @click="showNew = true">＋ 新建任务</button>
          </div>
        </div>
      </div>

      <div v-else class="task-grid">
        <article v-for="task in tasks" :key="task.id" class="task-card">
          <div class="title-row">
            <div class="title grow">{{ task.task_name }}</div>
            <span class="badge" :class="statusOf(task.status).cls">
              {{ statusOf(task.status).label }}
            </span>
          </div>

          <div class="task-meta">
            <div>评审 Agent：<b>{{ task.reviewer_name }}</b></div>
            <div>子任务：<b>{{ task.completed_count }}/{{ task.subtask_count }}</b></div>
            <div>Coder：<b>{{ task.coder_names.join("、") || "—" }}</b></div>
            <div>
              轮次：模块 <b>{{ task.module_max_round }}</b> / 全局
              <b>{{ task.global_max_round }}</b>
            </div>
            <div>阶段：<b>{{ task.task_phase === "global_review" ? "全局评审" : task.task_phase === "plan_approval" ? "方案审批" : "模块开发" }}</b></div>
            <div>创建：<b>{{ task.create_time }}</b></div>
          </div>

          <div style="font-size: 12px; color: var(--muted-2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis">
            📁 {{ task.work_dir }}
          </div>

          <div class="task-actions">
            <button class="btn btn-sm btn-primary" @click="emit('open-task', task.id)">
              打开详情
            </button>
            <button class="btn btn-sm btn-danger right" @click="remove(task)">删除任务</button>
          </div>
        </article>
      </div>
    </div>

    <NewTaskModal v-if="showNew" @close="showNew = false" @created="onCreated" />
  </div>
</template>
