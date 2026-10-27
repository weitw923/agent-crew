<script setup lang="ts">
import { computed, ref } from "vue";
import type { Iteration, IterationStatus, TaskMain } from "../types";

const props = defineProps<{
  task: TaskMain;
  iterations: Iteration[];
  loading?: boolean;
  busy?: boolean;
}>();

const emit = defineEmits<{
  (e: "open", iterationId: number): void;
  (e: "back"): void;
  (e: "followup"): void;
  (e: "refresh"): void;
}>();

const expanded = ref<number | null>(null);

/** 版本倒序：最新迭代在最上面。 */
const ordered = computed(() =>
  [...props.iterations].sort(
    (a, b) => b.major - a.major || b.minor - a.minor || b.id - a.id
  )
);

const statusMeta: Record<IterationStatus, { label: string; cls: string }> = {
  pending: { label: "⏳ 待开始", cls: "badge-blue" },
  running: { label: "🟡 进行中", cls: "badge-yellow" },
  completed: { label: "✅ 已完成", cls: "badge-green" },
  failed: { label: "❌ 执行失败", cls: "badge-red" },
  terminated: { label: "⛔ 已终止", cls: "badge-red" }
};

function versionText(it: Iteration) {
  if (it.version) return it.version;
  return "dev-?.? 待定";
}

function formatDuration(seconds: number) {
  if (!seconds || seconds < 0) return "—";
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = seconds % 60;
  if (h > 0) return `${h} 小时 ${m} 分`;
  if (m > 0) return `${m} 分 ${s} 秒`;
  return `${s} 秒`;
}

function timeRange(it: Iteration) {
  if (!it.start_time) return "尚未开始";
  return `${it.start_time} → ${it.end_time || "进行中"}`;
}

function subtaskProgress(it: Iteration) {
  return `${it.subtask_completed}/${it.subtask_total}`;
}

function progressPercent(it: Iteration) {
  if (!it.subtask_total) return 0;
  return Math.round((it.subtask_completed / it.subtask_total) * 100);
}

function tokenText(it: Iteration) {
  return it.token_total >= 0 ? `${it.token_total} tokens` : "未统计";
}

function toggleRequirement(id: number) {
  expanded.value = expanded.value === id ? null : id;
}
</script>

<template>
  <div class="iteration-list-view">
    <div class="iteration-list-head">
      <button class="back" title="返回任务列表" @click="emit('back')">←</button>
      <div>
        <div class="task-title">{{ task.task_name }}</div>
        <div class="iteration-list-meta">
          <span>创建：{{ task.create_time }}</span>
          <span>共 {{ iterations.length }} 个迭代</span>
          <span>最新版本：{{ ordered[0]?.version || "待定" }}</span>
          <span>当前状态：{{ task.status }}</span>
        </div>
      </div>
      <div class="flex" style="margin-left: auto">
        <button class="btn btn-sm" :disabled="busy" @click="emit('refresh')">刷新</button>
        <button
          v-if="task.status === 'completed'"
          class="btn btn-sm btn-primary"
          :disabled="busy"
          @click="emit('followup')"
        >
          ＋ 追加需求，继续迭代
        </button>
      </div>
    </div>

    <div class="iteration-list-body">
      <div class="iteration-list-tip">
        每次追加需求都会生成一个新的迭代版本：默认小版本 +1（dev-1.1）；单次需求拆出超过 5 个子任务时自动升大版本（dev-2.0）。
        进入某个迭代可以查看该版本范围内的子任务、评审对话与日志。
      </div>

      <div v-if="loading" class="empty-state" style="padding: 40px 10px">
        <div class="title">加载迭代…</div>
      </div>
      <div v-else-if="!ordered.length" class="empty-state" style="padding: 40px 10px">
        <div class="big">🧩</div>
        <div class="title">还没有迭代记录</div>
        <div class="small">任务首次拆解后会自动生成 dev-1.0。</div>
      </div>

      <article
        v-for="it in ordered"
        :key="it.id"
        class="iteration-card"
        :class="{ active: it.status === 'running' }"
      >
        <div class="iteration-card-head">
          <span class="iteration-version">{{ versionText(it) }}</span>
          <span class="badge" :class="statusMeta[it.status]?.cls">
            {{ statusMeta[it.status]?.label ?? it.status }}
          </span>
          <span class="iteration-duration">耗时：{{ formatDuration(it.duration_sec) }}</span>
          <button class="btn btn-sm btn-primary" style="margin-left: auto" @click="emit('open', it.id)">
            进入迭代 →
          </button>
        </div>

        <div class="iteration-card-metrics">
          <div class="metric">
            <span class="metric-label">需求</span>
            <span class="metric-value metric-requirement">
              {{ it.requirement || "（暂无需求描述）" }}
              <button
                v-if="(it.requirement || '').length > 90"
                class="link-btn"
                @click="toggleRequirement(it.id)"
              >
                {{ expanded === it.id ? "收起" : "展开" }}
              </button>
            </span>
          </div>
          <div class="metric">
            <span class="metric-label">开始时间</span>
            <span class="metric-value">{{ it.start_time || "—" }}</span>
          </div>
          <div class="metric">
            <span class="metric-label">结束时间</span>
            <span class="metric-value">{{ it.end_time || "—" }}</span>
          </div>
          <div class="metric">
            <span class="metric-label">任务数</span>
            <span class="metric-value">
              {{ subtaskProgress(it) }}
              <span class="progress-track">
                <span class="progress-fill" :style="{ width: progressPercent(it) + '%' }"></span>
              </span>
            </span>
          </div>
          <div class="metric">
            <span class="metric-label">耗时</span>
            <span class="metric-value">{{ formatDuration(it.duration_sec) }}</span>
          </div>
          <div class="metric">
            <span class="metric-label">Token</span>
            <span class="metric-value" :title="it.token_total < 0 ? '本版本尚未接入 token 统计' : ''">
              {{ tokenText(it) }}
            </span>
          </div>
        </div>

        <div class="iteration-card-foot">
          <span class="small muted">{{ timeRange(it) }}</span>
          <span v-if="it.analysis" class="iteration-analysis">
            评审梳理：{{ it.analysis }}
          </span>
        </div>

        <div v-if="expanded === it.id" class="iteration-requirement-full">
          <pre class="pre-box">{{ it.requirement }}</pre>
        </div>
      </article>
    </div>
  </div>
</template>
