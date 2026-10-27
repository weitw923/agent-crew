<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { Iteration, Subtask, SubtaskStage, SubtaskStatus, TaskMain } from "../types";

const props = defineProps<{
  task: TaskMain;
  iterations: Iteration[];
  subtasks: Subtask[];
  activeIterationId: number | null;
  selectedSubtaskId: number | null;
  frozen: boolean;
}>();

const emit = defineEmits<{
  (e: "open-iteration", id: number): void;
  (e: "select-subtask", id: number): void;
  (e: "view", subtask: Subtask): void;
  (e: "design", subtask: Subtask): void;
  (e: "history", subtask: Subtask): void;
}>();

/** 默认展开当前迭代，其它迭代折叠；版本倒序。 */
const openIds = ref<Set<number>>(new Set());

const ordered = computed(() =>
  [...props.iterations].sort(
    (a, b) => b.major - a.major || b.minor - a.minor || b.id - a.id
  )
);

watch(
  () => props.activeIterationId,
  (id) => {
    if (id == null) return;
    const next = new Set(openIds.value);
    next.add(id);
    openIds.value = next;
  },
  { immediate: true }
);

function subtasksOf(iterationId: number) {
  return props.subtasks.filter((s) => s.iteration_id === iterationId);
}

function toggle(id: number) {
  const next = new Set(openIds.value);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  openIds.value = next;
}

function isOpen(id: number) {
  return openIds.value.has(id) || id === props.activeIterationId;
}

const statusMeta: Record<SubtaskStatus, { label: string; cls: string }> = {
  waiting: { label: "⏳ 等待", cls: "" },
  in_progress: { label: "🟡 开发中", cls: "badge-yellow" },
  completed: { label: "✅ 完成", cls: "badge-green" },
  needs_human: { label: "⛔ 人工", cls: "badge-red" }
};

const stageLabel: Record<SubtaskStage, string> = {
  queued: "等待派发",
  design_generation: "生成设计中",
  design_approval: "设计待审批",
  coding: "Coder 编码中",
  code_review: "代码评审中",
  summary_generation: "生成摘要中",
  summary_review: "摘要校验中",
  done: "已完成",
  needs_human: "待人工处理"
};

const iterationStatusLabel: Record<string, string> = {
  pending: "待开始",
  running: "进行中",
  completed: "已完成",
  failed: "失败",
  terminated: "已终止"
};

function canDesign(sub: Subtask) {
  return !!sub.design_content;
}
</script>

<template>
  <aside class="wb-col left">
    <div class="wb-section-title">
      迭代与子任务
      <span class="count">{{ iterations.length }} 个版本</span>
    </div>

    <div class="iteration-tree">
      <div v-for="it in ordered" :key="it.id" class="iteration-node">
        <div
          class="iteration-node-head"
          :class="{ current: it.id === activeIterationId }"
          @click="toggle(it.id)"
        >
          <span class="iteration-caret">{{ isOpen(it.id) ? "▾" : "▸" }}</span>
          <span class="iteration-node-version">{{ it.version || "dev-?.? 待定" }}</span>
          <span class="iteration-node-status">{{ iterationStatusLabel[it.status] ?? it.status }}</span>
          <span class="iteration-node-count">
            {{ it.subtask_completed }}/{{ it.subtask_total }}
          </span>
          <button
            v-if="it.id !== activeIterationId"
            class="link-btn"
            @click.stop="emit('open-iteration', it.id)"
          >
            切换
          </button>
        </div>

        <div v-if="isOpen(it.id)" class="iteration-node-body">
          <div v-if="!subtasksOf(it.id).length" class="small muted" style="padding: 6px 8px">
            {{ it.status === "pending" ? "需求梳理中，子任务待生成。" : "该迭代暂无子任务。" }}
          </div>

          <article
            v-for="sub in subtasksOf(it.id)"
            :key="sub.id"
            class="subtask-card tree-subtask"
            :class="{
              selected: sub.id === selectedSubtaskId,
              frozen,
              needs_human: sub.status === 'needs_human'
            }"
          >
            <div class="subtask-main" @click="!frozen && emit('select-subtask', sub.id)">
              <div class="subtask-head">
                <div class="subtask-index">{{ sub.sort_index }}</div>
                <div class="subtask-name">{{ sub.subtask_name }}</div>
              </div>
              <div class="subtask-meta-row">
                <span class="badge" :class="statusMeta[sub.status]?.cls">
                  {{ statusMeta[sub.status]?.label ?? sub.status }}
                </span>
                <span class="round-pill">{{ sub.current_round }}/{{ sub.max_round }} 轮</span>
                <span v-if="sub.status === 'in_progress'" class="round-pill">
                  {{ stageLabel[sub.current_stage] ?? sub.current_stage }}
                </span>
              </div>
            </div>

            <div class="tree-subtask-actions">
              <button class="link-btn" title="只看该子任务的主要日志" @click.stop="emit('view', sub)">
                查看
              </button>
              <button
                class="link-btn"
                :disabled="!canDesign(sub)"
                :title="canDesign(sub) ? '在右侧查看模块设计方案' : '该模块还没有设计文档'"
                @click.stop="emit('design', sub)"
              >
                设计
              </button>
              <button
                class="link-btn"
                :disabled="!sub.iteration_records.length"
                :title="
                  sub.iteration_records.length
                    ? '在右侧查看该模块每一轮评审意见与 diff'
                    : '该模块还没有轮次记录'
                "
                @click.stop="emit('history', sub)"
              >
                历史
              </button>
            </div>
          </article>
        </div>
      </div>
    </div>
  </aside>
</template>
