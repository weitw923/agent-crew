<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import NewTaskModal from "../components/NewTaskModal.vue";
import type { TaskSummary } from "../types";
import { deleteTask, listTasks, resumeTask, startTask } from "../api";
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
  pending: { label: "待运行", cls: "badge-blue" },
  planning: { label: "拆解中", cls: "badge-blue" },
  running: { label: "执行中", cls: "badge-yellow" },
  needs_human: { label: "需人工介入", cls: "badge-red" },
  paused: { label: "已暂停", cls: "badge-yellow" },
  terminated: { label: "已终止", cls: "badge-red" },
  completed: { label: "已完成", cls: "badge-green" },
  failed: { label: "执行失败", cls: "badge-red" }
};

function statusOf(task: TaskSummary) {
  if (task.status === "pending") {
    if (task.pause_reason === "concurrency") return { label: "等待并发名额", cls: "badge-yellow" };
    if (task.execution_mode === "scheduled" && task.schedule_enabled)
      return { label: "等待时间窗口", cls: "badge-blue" };
    return { label: "待运行", cls: "badge-blue" };
  }
  if (task.status === "paused" && task.pause_reason === "waiting_window")
    return { label: "等待时间窗口", cls: "badge-blue" };
  const meta = statusMeta[task.status] ?? { label: task.status, cls: "" };
  return meta;
}

const modeLabel: Record<string, string> = {
  immediate: "立即执行",
  later: "稍后执行",
  scheduled: "定时执行"
};

function scheduleText(task: TaskSummary) {
  if (task.execution_mode !== "scheduled") return "";
  return `每日 ${task.schedule_start}–${task.schedule_end}${task.schedule_enabled ? "" : "（窗口关闭）"}`;
}

async function runTask(task: TaskSummary) {
  try {
    if (task.status === "pending") {
      const outcome = await startTask(task.id);
      showToast(outcome.message, outcome.started ? "success" : "error");
    } else {
      await resumeTask(task.id);
      showToast(
        task.status === "terminated" ? "任务已继续，从当前进度恢复" : "已请求恢复调度",
        "success"
      );
    }
    await load();
  } catch (e) {
    showToast(errorMessage(e), "error");
  }
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
            <span class="badge" :class="statusOf(task).cls">
              {{ statusOf(task).label }}
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
            <div>执行：<b>{{ modeLabel[task.execution_mode] ?? task.execution_mode }}</b></div>
            <div v-if="scheduleText(task)">窗口：<b>{{ scheduleText(task) }}</b></div>
            <div>创建：<b>{{ task.create_time }}</b></div>
          </div>

          <div style="font-size: 12px; color: var(--muted-2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis">
            📁 {{ task.work_dir }}
          </div>

          <div class="task-actions">
            <button class="btn btn-sm btn-primary" @click="emit('open-task', task.id)">
              打开详情
            </button>
            <button
              v-if="
                task.status === 'pending' ||
                task.status === 'paused' ||
                task.status === 'failed' ||
                task.status === 'terminated'
              "
              class="btn btn-sm"
              @click="runTask(task)"
            >
              {{
                task.status === "pending"
                  ? "运行"
                  : task.status === "terminated"
                    ? "继续"
                    : "恢复"
              }}
            </button>
            <button
              v-if="task.status === 'needs_human'"
              class="btn btn-sm btn-primary"
              @click="emit('open-task', task.id)"
            >
              处理人工介入
            </button>
            <button class="btn btn-sm btn-danger right" @click="remove(task)">删除任务</button>
          </div>
        </article>
      </div>
    </div>

    <NewTaskModal v-if="showNew" @close="showNew = false" @created="onCreated" />
  </div>
</template>
