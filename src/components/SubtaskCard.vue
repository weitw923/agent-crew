<script setup lang="ts">
import { ref } from "vue";
import type { IterationRecord, Subtask } from "../types";

const props = defineProps<{
  subtask: Subtask;
  selected: boolean;
  frozen: boolean;
}>();

const emit = defineEmits<{ (e: "select"): void }>();
const expanded = ref(false);

const statusMeta: Record<string, { label: string; cls: string }> = {
  waiting: { label: "⏳ 等待队列中", cls: "" },
  in_progress: { label: "🟡 当前开发中", cls: "badge-yellow" },
  completed: { label: "✅ 已完成", cls: "badge-green" },
  needs_human: { label: "⛔ 轮次超限人工介入", cls: "badge-red" }
};

const stageLabel: Record<string, string> = {
  queued: "等待派发",
  design_generation: "生成模块设计中",
  design_approval: "设计待审批",
  coding: "Coder 编码中",
  code_review: "代码评审中",
  summary_generation: "生成摘要中",
  summary_review: "摘要校验中",
  done: "已完成",
  needs_human: "待人工处理"
};

const typeLabel: Record<string, string> = {
  code_review: "代码评审",
  summary_review: "摘要校验",
  human_reject: "人工驳回",
  global_rework: "全局返工"
};

function decisionLabel(r: IterationRecord) {
  if (r.review_decision === "pass") return "通过";
  if (r.review_decision === "reject") return "不通过";
  return "—";
}
</script>

<template>
  <article
    class="subtask-card"
    :class="{
      selected,
      frozen,
      needs_human: subtask.status === 'needs_human'
    }"
  >
    <div class="subtask-main" @click="!frozen && emit('select')">
      <div class="subtask-head">
        <div class="subtask-index">{{ subtask.sort_index }}</div>
        <div class="subtask-name">{{ subtask.subtask_name }}</div>
        <button class="expand-btn" @click.stop="expanded = !expanded">
          {{ expanded ? "收起 ▴" : "历史 ▾" }}
        </button>
      </div>

      <div class="subtask-meta-row">
        <span class="badge" :class="statusMeta[subtask.status]?.cls">
          {{ statusMeta[subtask.status]?.label ?? subtask.status }}
        </span>
        <span class="round-pill">
          第 {{ subtask.current_round }} / {{ subtask.max_round }} 轮
        </span>
        <span v-if="subtask.status === 'in_progress'" class="round-pill">
          {{ stageLabel[subtask.current_stage] ?? subtask.current_stage }}
        </span>
      </div>
    </div>

    <div v-if="expanded" class="history-box">
      <div v-if="!subtask.iteration_records.length" class="muted small">
        该子任务还没有历史迭代记录。
      </div>
      <div
        v-for="(rec, idx) in subtask.iteration_records"
        :key="idx"
        class="history-round"
        :class="rec.review_decision"
      >
        <div class="hd">
          <span>第 {{ rec.round }} 轮</span>
          <span class="badge" style="padding: 0 6px; font-size: 10px">
            {{ typeLabel[rec.type] ?? rec.type }}
          </span>
          <span class="right">{{ rec.created_at }}</span>
        </div>

        <div v-if="rec.review_feedback" class="fb">
          <b style="color: var(--muted)">评审意见：</b>{{ rec.review_feedback }}
        </div>
        <div v-else class="fb muted">（无评审意见）</div>

        <div v-if="rec.summary_content" style="margin-top: 6px">
          <div style="font-size: 10.5px; color: var(--muted)">摘要内容：</div>
          <pre class="pre-box" style="max-height: 150px">{{ rec.summary_content }}</pre>
        </div>

        <details v-if="rec.diff">
          <summary>查看本轮 Coder diff</summary>
          <pre class="pre-box">{{ rec.diff }}</pre>
        </details>

        <div style="margin-top: 6px; font-size: 10.5px; color: var(--muted)">
          本轮结论：
          <b :style="{ color: rec.review_decision === 'pass' ? 'var(--green)' : 'var(--red)' }">
            {{ decisionLabel(rec) }}
          </b>
        </div>
      </div>
    </div>
  </article>
</template>
