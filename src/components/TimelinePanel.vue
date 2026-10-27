<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import type { Iteration, PreviewPayload, Subtask, TaskEvent } from "../types";

const props = defineProps<{
  events: TaskEvent[];
  subtasks: Subtask[];
  iteration: Iteration | null;
  reviewerName: string;
  coderName: string;
  /** 只看某个子任务（左栏「查看」触发）。 */
  focusSubtaskId: number | null;
  showTools: boolean;
}>();

const emit = defineEmits<{
  (e: "preview", payload: PreviewPayload): void;
  (e: "update:showTools", value: boolean): void;
  (e: "clear-focus"): void;
}>();

type Role = "reviewer" | "coder" | "human" | "system";

const roleFilter = ref<"all" | Role>("all");
const expandedIds = ref<Set<number>>(new Set());
const box = ref<HTMLElement | null>(null);

const TOOL_KINDS = new Set(["tool_call", "tool_result"]);
const NOISE_BASE = new Set([
  "llm_request",
  "llm_response",
  "llm_error",
  "dsh_raw",
  "dsh_stdout",
  "dsh_stderr",
  "coder_prompt",
  "summary_prompt"
]);

function baseKind(kind: string) {
  return kind.includes(":") ? kind.slice(0, kind.indexOf(":")) : kind;
}

function roleOf(ev: TaskEvent): Role {
  const base = baseKind(ev.kind);
  if (ev.kind.startsWith("agent_message:reviewer")) return "reviewer";
  if (ev.kind.startsWith("agent_message:coder")) return "coder";
  if (base === "module_design" || base === "summary_file") return "reviewer";
  if (base === "human_message" || base === "human_message_delivered") return "human";
  if (base === "requirements_updated") return "human";
  return "system";
}

const roleLabel: Record<Role, string> = {
  reviewer: "评审者",
  coder: "Coder",
  human: "你",
  system: "系统"
};

function isMirroredTool(ev: TaskEvent) {
  return TOOL_KINDS.has(baseKind(ev.kind));
}

/** 时间线主体：过滤掉只进「原始事件」的 LLM 请求/响应，以及默认收起的工具调用。 */
const timeline = computed(() => {
  const out: TaskEvent[] = [];
  for (const ev of props.events) {
    const base = baseKind(ev.kind);
    if (NOISE_BASE.has(base)) continue;
    if (isMirroredTool(ev) && !props.showTools) continue;
    if (props.focusSubtaskId != null && ev.subtask_id !== props.focusSubtaskId) continue;
    if (roleFilter.value !== "all" && roleOf(ev) !== roleFilter.value) continue;
    out.push(ev);
  }
  return out;
});

const toolCount = computed(
  () =>
    props.events.filter(
      (ev) =>
        isMirroredTool(ev) &&
        (props.focusSubtaskId == null || ev.subtask_id === props.focusSubtaskId)
    ).length
);

const subtaskNames = computed(() => {
  const map = new Map<number, Subtask>();
  for (const s of props.subtasks) map.set(s.id, s);
  return map;
});

function subtaskTitle(id: number | null) {
  if (id == null) return "";
  const sub = subtaskNames.value.get(id);
  return sub ? `#${sub.sort_index} ${sub.subtask_name}` : `子任务 #${id}`;
}

/** 子任务分组标题：只在切换子任务时插入一次。 */
function showsSubtaskDivider(index: number) {
  if (index === 0) return true;
  return timeline.value[index].subtask_id !== timeline.value[index - 1].subtask_id;
}

const focusTitle = computed(() => {
  if (props.focusSubtaskId == null) return "";
  return subtaskTitle(props.focusSubtaskId);
});

function kindText(ev: TaskEvent) {
  const base = baseKind(ev.kind);
  const map: Record<string, string> = {
    agent_message: ev.kind.endsWith("reviewer") ? "评审意见" : "Coder 汇报",
    module_design: "模块设计方案",
    summary_file: "模块完成摘要",
    git_diff: "代码变更",
    human_message: "人工消息",
    human_message_delivered: "已注入 Agent",
    requirements_updated: "需求更新",
    plan_approval: "方案待审批",
    plan_accepted: "方案已确认",
    plan_approved: "方案通过",
    plan_rejected: "方案驳回",
    design_approved: "设计通过",
    design_rejected: "设计驳回",
    requirement_queued: "追加需求已提交",
    requirement_accepted: "追加需求已落成子任务",
    requirement_intake: "需求梳理",
    config_updated: "配置更新",
    human_extend_rounds: "人工追加轮次",
    system_log: "系统",
    tool_call: "工具调用",
    tool_result: "工具结果",
    dsh_error: "Coder 错误",
    module_design_rejected: "设计驳回"
  };
  return map[base] ?? base;
}

function isClickable(ev: TaskEvent) {
  const base = baseKind(ev.kind);
  return ["module_design", "summary_file", "git_diff"].includes(base);
}

function isLong(ev: TaskEvent) {
  return ev.content.length > 700;
}

function isExpanded(id: number) {
  return expandedIds.value.has(id);
}

function toggleExpand(id: number) {
  const next = new Set(expandedIds.value);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  expandedIds.value = next;
}

function openPreview(ev: TaskEvent) {
  const base = baseKind(ev.kind);
  const sub = ev.subtask_id != null ? subtaskNames.value.get(ev.subtask_id) : null;
  const source = [
    props.iteration?.version || "当前迭代",
    sub ? `#${sub.sort_index} ${sub.subtask_name}` : "",
    sub ? `第 ${sub.current_round} 轮` : ""
  ]
    .filter(Boolean)
    .join(" · ");
  const map: Record<string, { kind: PreviewPayload["kind"]; title: string }> = {
    module_design: { kind: "design", title: "模块设计方案" },
    summary_file: { kind: "summary", title: "模块完成摘要" },
    git_diff: { kind: "diff", title: "代码变更 diff" }
  };
  const meta = map[base];
  if (!meta) return;
  emit("preview", {
    kind: meta.kind,
    title: meta.title,
    content: ev.content,
    source,
    time: ev.create_time
  });
}

function toolLabel(ev: TaskEvent) {
  try {
    const value = JSON.parse(ev.content) as { tool?: string; input?: unknown };
    return value.tool ?? "tool";
  } catch {
    return "tool";
  }
}

function toolSummary(ev: TaskEvent) {
  return ev.content.length > 140 ? `${ev.content.slice(0, 140)}…` : ev.content;
}

function eventText(ev: TaskEvent) {
  const base = baseKind(ev.kind);
  if (base === "tool_call" || base === "tool_result") return "";
  return ev.content.length > 6000 ? `${ev.content.slice(0, 6000)}\n…（已截断，完整内容见「原始事件」）` : ev.content;
}

function scrollToBottom() {
  const el = box.value;
  if (el) el.scrollTop = el.scrollHeight;
}

watch(
  () => [props.events.length, props.focusSubtaskId, roleFilter.value, props.showTools],
  async () => {
    await nextTick();
    scrollToBottom();
  }
);

defineExpose({ scrollToBottom });
</script>

<template>
  <section class="timeline-panel">
    <div class="conv-toolbar">
      <span class="conv-title">评审者与 Coder 的完整对话</span>
      <button
        class="chip"
        :class="{ active: roleFilter === 'all' }"
        @click="roleFilter = 'all'"
      >
        全部
      </button>
      <button
        class="chip"
        :class="{ active: roleFilter === 'reviewer' }"
        @click="roleFilter = 'reviewer'"
      >
        评审者
      </button>
      <button
        class="chip"
        :class="{ active: roleFilter === 'coder' }"
        @click="roleFilter = 'coder'"
      >
        Coder
      </button>
      <button
        class="chip"
        :class="{ active: roleFilter === 'human' }"
        @click="roleFilter = 'human'"
      >
        人工
      </button>
      <button
        class="chip tool-toggle"
        :class="{ active: showTools }"
        :title="showTools ? '收起工具调用' : '展开工具调用'"
        @click="emit('update:showTools', !showTools)"
      >
        <span class="caret">{{ showTools ? "▾" : "▸" }}</span>
        工具调用（{{ toolCount }}）
      </button>
      <button
        v-if="focusSubtaskId != null"
        class="chip focus-chip"
        title="取消只看该子任务"
        @click="emit('clear-focus')"
      >
        只看：{{ focusTitle }} ✕
      </button>
    </div>

    <div ref="box" class="timeline-box">
      <div v-if="!timeline.length" class="muted small conv-empty">
        该迭代还没有对话记录。评审与 Coder 的关键交互、模块设计、评审结论都会按时间出现在这里。<br />
        如果该迭代创建时未开启「日志持久化」，历史对话不会保存，需要重新开启后才能记录。
      </div>

      <template v-for="(ev, idx) in timeline" :key="ev.id">
        <div v-if="showsSubtaskDivider(idx) && ev.subtask_id != null" class="timeline-divider">
          <span>{{ subtaskTitle(ev.subtask_id) }}</span>
        </div>

        <div class="timeline-item" :class="roleOf(ev)">
          <div class="timeline-role" :class="roleOf(ev)">{{ roleLabel[roleOf(ev)] }}</div>
          <div class="timeline-body">
            <div class="timeline-meta">
              <span class="timeline-kind">{{ kindText(ev) }}</span>
              <span class="timeline-time">{{ ev.create_time.slice(11) }}</span>
              <button
                v-if="isClickable(ev)"
                class="link-btn"
                style="margin-left: auto"
                @click="openPreview(ev)"
              >
                在右侧查看详情 →
              </button>
            </div>

            <template v-if="baseKind(ev.kind) === 'tool_call'">
              <div class="tool-line">
                <span class="tool-icon">🔧</span>
                <b>{{ toolLabel(ev) }}</b>
                <span class="tool-arg">{{ toolSummary(ev) }}</span>
              </div>
            </template>

            <template v-else-if="baseKind(ev.kind) === 'tool_result'">
              <details class="conv-detail">
                <summary>查看工具返回结果</summary>
                <pre>{{ ev.content }}</pre>
              </details>
            </template>

            <template v-else-if="isClickable(ev)">
              <div class="timeline-card" @click="openPreview(ev)">
                <b>{{ kindText(ev) }}</b>
                <span class="small muted">点击在右侧预览</span>
                <pre class="timeline-preview-text">{{
                  ev.content.length > 260 ? ev.content.slice(0, 260) + "…" : ev.content
                }}</pre>
              </div>
            </template>

            <template v-else-if="roleOf(ev) === 'system'">
              <div class="timeline-system">{{ eventText(ev) }}</div>
            </template>

            <template v-else>
              <div class="timeline-text">
                <pre class="timeline-pre" :class="{ clamped: isLong(ev) && !isExpanded(ev.id) }">{{
                  eventText(ev)
                }}</pre>
                <button
                  v-if="isLong(ev)"
                  class="link-btn"
                  @click="toggleExpand(ev.id)"
                >
                  {{ isExpanded(ev.id) ? "收起 ▴" : "展开全文 ▾" }}
                </button>
              </div>
            </template>
          </div>
        </div>
      </template>
    </div>
  </section>
</template>
