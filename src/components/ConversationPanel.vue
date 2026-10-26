<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import type { TaskEvent } from "../types";

const props = defineProps<{
  events: TaskEvent[];
  reviewerName: string;
  coderName: string;
}>();

const emit = defineEmits<{ (e: "open-console"): void }>();

type Filter = "conversation" | "tools" | "all";
const filter = ref<Filter>("conversation");
const showToolResults = ref(false);
const box = ref<HTMLElement | null>(null);

const CONVERSATION_KINDS = new Set([
  "agent_message",
  "module_design",
  "plan_approval",
  "plan_accepted",
  "plan_approved",
  "plan_rejected",
  "design_approved",
  "design_rejected",
  "git_diff",
  "summary_file",
  "human_message",
  "human_message_delivered",
  "requirements_updated",
  "config_updated",
  "tool_call",
  "dsh_error",
  "llm_error",
  "system_log"
]);

function baseKind(kind: string) {
  return kind.includes(":") ? kind.slice(0, kind.indexOf(":")) : kind;
}

function include(ev: TaskEvent) {
  const base = baseKind(ev.kind);
  if (filter.value === "all") return true;
  if (filter.value === "tools") return base === "tool_call" || base === "tool_result";
  if (base === "tool_result") return showToolResults.value;
  if (base === "llm_response") {
    return (
      ev.kind === "llm_response:code_review" ||
      ev.kind === "llm_response:summary_review" ||
      ev.kind === "llm_response:global_review"
    );
  }
  return CONVERSATION_KINDS.has(base);
}

const visible = computed(() => props.events.filter(include).slice(-500));

function roleOf(ev: TaskEvent) {
  const base = baseKind(ev.kind);
  if (
    ev.kind === "agent_message:reviewer" ||
    base === "module_design" ||
    base === "llm_response"
  )
    return { name: props.reviewerName || "技术总监", cls: "reviewer", label: "技术总监" };
  if (ev.kind === "agent_message:coder" || base === "git_diff" || base === "summary_file")
    return { name: props.coderName || "Coder", cls: "coder", label: "Coder" };
  if (base === "tool_call" || base === "tool_result")
    return { name: props.coderName || "Coder", cls: "coder", label: "工具" };
  if (ev.kind.startsWith("human_message") || base === "requirements_updated")
    return { name: "你", cls: "human", label: "人工" };
  if (base === "system_log") return { name: "系统", cls: "system", label: "系统" };
  return { name: "系统", cls: "system", label: "事件" };
}

function labelOf(ev: TaskEvent) {
  const base = baseKind(ev.kind);
  const map: Record<string, string> = {
    agent_message: "Agent 发言",
    module_design: "模块设计文档",
    plan_approval: "方案待审批",
    plan_accepted: "方案已确认",
    plan_approved: "方案审批通过",
    plan_rejected: "方案被驳回",
    design_approved: "设计审批通过",
    design_rejected: "设计被驳回",
    git_diff: "代码变更",
    summary_file: "模块摘要",
    human_message: "人工消息",
    human_message_delivered: "人工消息已注入",
    requirements_updated: "需求已更新",
    config_updated: "配置更新",
    tool_call: "工具调用",
    tool_result: "工具结果",
    dsh_error: "Coder 错误",
    llm_error: "LLM 错误",
    system_log: "系统",
    llm_response: "评审原始结果",
    dsh_raw: "dsh 原始输出"
  };
  return map[base] ?? ev.kind;
}

function toolInfo(ev: TaskEvent) {
  try {
    const v = JSON.parse(ev.content);
    return {
      tool: String(v.tool ?? "tool"),
      input: v.input ?? {},
      status: String(v.status ?? ""),
      result: String(v.result ?? "")
    };
  } catch {
    return { tool: "tool", input: {}, status: "", result: ev.content };
  }
}

function toolArgSummary(ev: TaskEvent) {
  const info = toolInfo(ev);
  const input: Record<string, unknown> = info.input;
  const raw =
    input.file_path ??
    input.path ??
    input.pattern ??
    input.command ??
    input.query ??
    input.description ??
    "";
  const text = String(raw).replace(/\s+/g, " ").trim();
  return text.length > 90 ? `${text.slice(0, 90)}…` : text;
}

function toolIcon(tool: string) {
  const map: Record<string, string> = {
    read: "📖",
    write: "✍️",
    edit: "✏️",
    bash: "⌘",
    glob: "🔎",
    grep: "🔍",
    git: "⑂"
  };
  return map[tool] ?? "🛠";
}

function diffLineCount(ev: TaskEvent) {
  return ev.content.split("\n").length;
}

function shortText(text: string, n = 240) {
  return text.length > n ? `${text.slice(0, n)}…` : text;
}

function eventClass(ev: TaskEvent) {
  const base = baseKind(ev.kind);
  if (base === "llm_error" || base === "dsh_error" || base.endsWith("_rejected"))
    return "error";
  if (base === "human_message" || base === "requirements_updated") return "human";
  if (base === "tool_call" || base === "tool_result") return "tool";
  return "";
}

function scrollBottom() {
  nextTick(() => {
    if (box.value) box.value.scrollTop = box.value.scrollHeight;
  });
}

watch(
  () => [visible.value.length, filter.value, showToolResults.value],
  scrollBottom
);
</script>

<template>
  <div class="conversation-panel">
    <div class="conv-toolbar">
      <button class="chip" :class="{ active: filter === 'conversation' }" @click="filter = 'conversation'">
        会话
      </button>
      <button class="chip" :class="{ active: filter === 'tools' }" @click="filter = 'tools'">
        工具
      </button>
      <button class="chip" :class="{ active: filter === 'all' }" @click="filter = 'all'">
        全部
      </button>
      <label v-if="filter !== 'tools'" class="tiny-check">
        <input v-model="showToolResults" type="checkbox" />
        工具结果
      </label>
      <button class="btn btn-sm btn-ghost" style="margin-left: auto" @click="emit('open-console')">
        原始事件
      </button>
    </div>

    <div ref="box" class="conv-box">
      <div v-if="!visible.length" class="muted small conv-empty">
        暂无 Agent 对话。开始拆解后，这里会按时间线展示 Agent 之间的关键对话和工具调用。
      </div>

      <div v-for="ev in visible" :key="ev.id" class="conv-item" :class="eventClass(ev)">
        <div class="conv-role" :class="roleOf(ev).cls">
          {{ roleOf(ev).label }}
        </div>
        <div class="conv-body">
          <div class="conv-meta">
            <span>{{ labelOf(ev) }}</span>
            <span v-if="ev.subtask_id" class="conv-sub">#{{ ev.subtask_id }}</span>
            <span class="conv-time">{{ ev.create_time.slice(11) }}</span>
          </div>

          <template v-if="ev.kind === 'tool_call'">
            <div class="tool-call">
              <span class="tool-icon">{{ toolIcon(toolInfo(ev).tool) }}</span>
              <b>{{ toolInfo(ev).tool }}</b>
              <span class="tool-arg">{{ toolArgSummary(ev) }}</span>
            </div>
          </template>

          <template v-else-if="ev.kind === 'tool_result'">
            <details class="conv-detail">
              <summary>查看工具返回结果</summary>
              <pre>{{ toolInfo(ev).result }}</pre>
            </details>
          </template>

          <template v-else-if="ev.kind.startsWith('git_diff')">
            <details class="conv-detail">
              <summary>查看代码 diff（{{ diffLineCount(ev) }} 行）</summary>
              <pre>{{ ev.content }}</pre>
            </details>
          </template>

          <template v-else-if="ev.kind === 'module_design' || ev.kind === 'summary_file'">
            <details class="conv-detail">
              <summary>展开内容</summary>
              <pre>{{ ev.content }}</pre>
            </details>
          </template>

          <template v-else-if="ev.kind.startsWith('llm_response')">
            <details class="conv-detail">
              <summary>查看评审原始 JSON</summary>
              <pre>{{ ev.content }}</pre>
            </details>
          </template>

          <template v-else-if="ev.kind === 'system_log'">
            <div class="conv-text system-text">{{ ev.content }}</div>
          </template>

          <template v-else>
            <div class="conv-text">{{ shortText(ev.content, 900) }}</div>
          </template>
        </div>
      </div>
    </div>
  </div>
</template>
