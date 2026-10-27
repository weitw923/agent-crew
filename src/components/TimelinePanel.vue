<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import { markdownToSnippet } from "../markdown";
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
  /** 首次装载时是否自动滚到最新一条（方案审批页需要从顶部开始读）。 */
  autoScroll?: boolean;
}>();

const emit = defineEmits<{
  (e: "preview", payload: PreviewPayload): void;
  (e: "update:showTools", value: boolean): void;
  (e: "clear-focus"): void;
}>();

type Role = "reviewer" | "coder" | "human" | "system";

const roleFilter = ref<"all" | Role>("all");
const expandedIds = ref<Set<number>>(new Set());
const searchText = ref("");
const collapsedGroups = ref<Set<string>>(new Set());
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

const keyword = computed(() => searchText.value.trim().toLowerCase());

function matchesSearch(ev: TaskEvent) {
  if (!keyword.value) return true;
  return ev.content.toLowerCase().includes(keyword.value);
}

/** 时间线主体：过滤噪音事件，工具调用默认收起，再按角色 / 关键词过滤。 */
const timeline = computed(() => {
  const out: TaskEvent[] = [];
  for (const ev of props.events) {
    const base = baseKind(ev.kind);
    if (NOISE_BASE.has(base)) continue;
    if (isMirroredTool(ev) && !props.showTools) continue;
    if (props.focusSubtaskId != null && ev.subtask_id !== props.focusSubtaskId) continue;
    if (roleFilter.value !== "all" && roleOf(ev) !== roleFilter.value) continue;
    if (!matchesSearch(ev)) continue;
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

const hitCount = computed(() => (keyword.value ? timeline.value.length : 0));

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

function groupKey(id: number | null) {
  return id == null ? "iteration" : String(id);
}

interface EvGroup {
  key: string;
  subtaskId: number | null;
  subtask: Subtask | null;
  title: string;
  events: TaskEvent[];
  startTime: string;
}

/** 按子任务把对话切成可折叠的整段。 */
const groups = computed<EvGroup[]>(() => {
  const map = new Map<string, EvGroup>();
  for (const ev of timeline.value) {
    const key = groupKey(ev.subtask_id);
    let group = map.get(key);
    if (!group) {
      const sub = ev.subtask_id != null ? subtaskNames.value.get(ev.subtask_id) ?? null : null;
      group = {
        key,
        subtaskId: ev.subtask_id,
        subtask: sub,
        title:
          ev.subtask_id == null
            ? "迭代级对话（需求梳理 / 全局评审）"
            : subtaskTitle(ev.subtask_id),
        events: [],
        startTime: ev.create_time
      };
      map.set(key, group);
    }
    group.events.push(ev);
  }
  return [...map.values()];
});

/** 搜索时强制展开所有分组，避免命中项被折叠隐藏。 */
function isGroupOpen(group: EvGroup) {
  if (keyword.value) return true;
  return !collapsedGroups.value.has(group.key);
}

function toggleGroup(group: EvGroup) {
  const next = new Set(collapsedGroups.value);
  if (next.has(group.key)) next.delete(group.key);
  else next.add(group.key);
  collapsedGroups.value = next;
}

const allCollapsed = computed(
  () =>
    groups.value.length > 0 && groups.value.every((g) => collapsedGroups.value.has(g.key))
);

function toggleAllGroups() {
  if (allCollapsed.value) {
    collapsedGroups.value = new Set();
  } else {
    collapsedGroups.value = new Set(groups.value.map((g) => g.key));
  }
}

/** 分组内的轮次锚点：优先 diff，其次评审发言。 */
function roundAnchors(sub: Subtask | null) {
  if (!sub) return [];
  return sub.iteration_records.map((rec, idx) => {
    const reviewEvents = props.events.filter(
      (e) => e.subtask_id === sub.id && e.kind.startsWith("agent_message:reviewer")
    );
    const diffEvents = props.events.filter(
      (e) => e.subtask_id === sub.id && baseKind(e.kind) === "git_diff"
    );
    const anchor = diffEvents[idx] ?? reviewEvents[idx] ?? null;
    return {
      round: rec.round || idx + 1,
      decision: rec.review_decision,
      eventId: anchor?.id ?? null
    };
  });
}

async function jumpToEvent(eventId: number | null) {
  if (eventId == null) return;
  for (const group of groups.value) {
    if (!group.events.some((e) => e.id === eventId)) continue;
    const next = new Set(collapsedGroups.value);
    next.delete(group.key);
    collapsedGroups.value = next;
  }
  await nextTick();
  const target = box.value?.querySelector<HTMLElement>(`[data-eid="${eventId}"]`);
  target?.scrollIntoView({ block: "center", behavior: "smooth" });
}

async function jumpToRound(sub: Subtask | null, index: number) {
  const anchor = roundAnchors(sub)[index];
  if (!anchor) return;
  await jumpToEvent(anchor.eventId);
}

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
    dsh_error: "Coder 错误"
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
  return ev.content.length > 6000
    ? `${ev.content.slice(0, 6000)}\n…（已截断，完整内容见「原始事件」）`
    : ev.content;
}

/** 关键词高亮：把文本切成命中/未命中片段，避免 innerHTML。 */
interface Segment {
  text: string;
  hit: boolean;
}
function highlight(raw: string, limit = 6000): Segment[] {
  const text = raw.length > limit ? `${raw.slice(0, limit)}…` : raw;
  if (!keyword.value) return [{ text, hit: false }];
  const lower = text.toLowerCase();
  const key = keyword.value;
  const out: Segment[] = [];
  let cursor = 0;
  while (cursor < text.length) {
    const idx = lower.indexOf(key, cursor);
    if (idx < 0) {
      out.push({ text: text.slice(cursor), hit: false });
      break;
    }
    if (idx > cursor) out.push({ text: text.slice(cursor, idx), hit: false });
    out.push({ text: text.slice(idx, idx + key.length), hit: true });
    cursor = idx + key.length;
  }
  return out;
}

/** 时间线自身不再滚动：整体滚动交给中栏的 .wb-mid-scroll。 */
function scrollContainer(): HTMLElement | null {
  let el = box.value?.parentElement ?? null;
  while (el) {
    const style = getComputedStyle(el);
    if (/(auto|scroll)/.test(style.overflowY) && el.scrollHeight > el.clientHeight) return el;
    el = el.parentElement;
  }
  return null;
}

function scrollToBottom() {
  const el = scrollContainer();
  if (el) el.scrollTop = el.scrollHeight;
}

// 切换迭代后重新允许自动定位到底部
watch(
  () => props.iteration?.id,
  () => {
    firstScrollDone = false;
  }
);

/** 用户已经往上翻看历史时，不要用新消息把他拽回底部。 */
function nearBottom() {
  const el = scrollContainer();
  if (!el) return true;
  return el.scrollHeight - el.scrollTop - el.clientHeight < 240;
}

let firstScrollDone = false;

watch(
  () => [props.events.length, props.focusSubtaskId, roleFilter.value, props.showTools],
  async () => {
    await nextTick();
    if (!firstScrollDone) {
      firstScrollDone = true;
      if (props.autoScroll !== false) scrollToBottom();
      return;
    }
    if (nearBottom()) scrollToBottom();
  }
);

defineExpose({ scrollToBottom });
</script>

<template>
  <section class="timeline-panel">
    <!-- 工具条 + 搜索：吸附在中栏滚动区顶部，避免滚到对话深处后找不到筛选 -->
    <div class="timeline-sticky">
    <div class="conv-toolbar">
      <span class="conv-title">评审者与 Coder 的完整对话</span>
      <button class="chip" :class="{ active: roleFilter === 'all' }" @click="roleFilter = 'all'">
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
        class="chip"
        :title="allCollapsed ? '展开全部对话段' : '折叠全部对话段'"
        @click="toggleAllGroups"
      >
        {{ allCollapsed ? "▾ 展开全部" : "▸ 折叠全部" }}
      </button>
      <button
        v-if="focusSubtaskId != null"
        class="chip focus-chip"
        title="取消只看该子任务"
        @click="emit('clear-focus')"
      >
        只看：{{ subtaskTitle(focusSubtaskId) }} ✕
      </button>
    </div>

    <div class="timeline-search">
      <span class="search-icon">🔍</span>
      <input
        v-model="searchText"
        type="text"
        placeholder="搜索对话关键词，例如「不通过」「权限」「导出」"
        @keydown.enter.prevent="jumpToEvent(timeline[0]?.id ?? null)"
      />
      <span v-if="keyword" class="search-count">{{ hitCount }} 条命中</span>
      <button v-if="keyword" class="link-btn" @click="searchText = ''">清除</button>
    </div>
    </div>

    <div ref="box" class="timeline-box">
      <div v-if="!timeline.length" class="muted small conv-empty">
        <template v-if="keyword">没有匹配「{{ searchText }}」的对话。</template>
        <template v-else>
          该迭代还没有对话记录。评审与 Coder 的关键交互、模块设计、评审结论都会按时间出现在这里。<br />
          如果该迭代创建时未开启「日志持久化」，历史对话不会保存，需要重新开启后才能记录。
        </template>
      </div>

      <div v-for="group in groups" :key="group.key" class="timeline-group">
        <div class="timeline-group-head" @click="toggleGroup(group)">
          <span class="group-caret">{{ isGroupOpen(group) ? "▾" : "▸" }}</span>
          <span class="group-title">{{ group.title }}</span>
          <span class="group-count">{{ group.events.length }} 条</span>
          <span class="group-time">{{ group.startTime.slice(11, 16) }} 起</span>
          <span v-if="group.subtask" class="group-rounds" @click.stop>
            <button
              v-for="(anchor, idx) in roundAnchors(group.subtask)"
              :key="idx"
              class="round-chip"
              :class="anchor.decision"
              :title="`跳转到第 ${anchor.round} 轮评审`"
              @click.stop="jumpToRound(group.subtask, idx)"
            >
              第{{ anchor.round }}轮
            </button>
          </span>
        </div>

        <div v-if="isGroupOpen(group)" class="timeline-group-body">
          <div
            v-for="ev in group.events"
            :key="ev.id"
            :data-eid="ev.id"
            class="timeline-item"
            :class="roleOf(ev)"
          >
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
                  <pre class="timeline-preview-text"><span
                    v-for="(seg, i) in highlight(markdownToSnippet(ev.content, 260), 260)"
                    :key="i"
                    :class="{ 'timeline-hit': seg.hit }"
                  >{{ seg.text }}</span></pre>
                </div>
              </template>

              <template v-else-if="roleOf(ev) === 'system'">
                <div class="timeline-system"><span
                  v-for="(seg, i) in highlight(ev.content)"
                  :key="i"
                  :class="{ 'timeline-hit': seg.hit }"
                >{{ seg.text }}</span></div>
              </template>

              <template v-else>
                <div class="timeline-text">
                  <pre
                    class="timeline-pre"
                    :class="{ clamped: isLong(ev) && !isExpanded(ev.id) }"
                  ><span
                    v-for="(seg, i) in highlight(eventText(ev))"
                    :key="i"
                    :class="{ 'timeline-hit': seg.hit }"
                  >{{ seg.text }}</span></pre>
                  <button v-if="isLong(ev)" class="link-btn" @click="toggleExpand(ev.id)">
                    {{ isExpanded(ev.id) ? "收起 ▴" : "展开全文 ▾" }}
                  </button>
                </div>
              </template>
            </div>
          </div>
        </div>
      </div>
    </div>
  </section>
</template>
