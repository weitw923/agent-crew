<script setup lang="ts">
import { computed } from "vue";
import MarkdownView from "./MarkdownView.vue";
import { stripDuplicateHeading } from "../markdown";
import type { Iteration, PreviewPayload, Subtask, TaskMain } from "../types";

const props = defineProps<{
  preview: PreviewPayload | null;
  task: TaskMain;
  iteration: Iteration | null;
  selectedSubtask: Subtask | null;
}>();

const emit = defineEmits<{
  (e: "preview", payload: PreviewPayload): void;
  (e: "close"): void;
}>();

const hasPreview = computed(() => !!props.preview);
const previewMarkdown = computed(() =>
  props.preview ? stripDuplicateHeading(props.preview.content, props.preview.title) : ""
);

function contractPreview(kind: "tech_stack" | "contract" | "requirement" | "criteria") {
  if (!props.task) return;
  const subtask = props.selectedSubtask;
  const map = {
    tech_stack: { kind: "tech_stack" as const, title: "技术栈", content: props.task.tech_stack || "拆解完成后生成。" },
    contract: {
      kind: "contract" as const,
      title: "全局契约",
      content: props.task.global_contract || "拆解完成后生成。"
    },
    requirement: {
      kind: "requirement" as const,
      title: "顶层需求",
      content: props.task.top_requirement || "（空）"
    },
    criteria: {
      kind: "criteria" as const,
      title: "项目验收标准",
      content: props.task.global_accept_criteria || "（空）"
    }
  };
  const payload = map[kind];
  emit("preview", {
    ...payload,
    source: [props.iteration?.version, subtask ? `#${subtask.sort_index} ${subtask.subtask_name}` : ""]
      .filter(Boolean)
      .join(" · ")
  });
}
</script>

<template>
  <aside class="wb-col right preview-col">
    <div class="wb-section-title">
      预览
      <button v-if="hasPreview" class="link-btn" style="margin-left: auto" @click="emit('close')">
        关闭 ✕
      </button>
    </div>

    <!-- 快捷入口：技术栈 / 契约 / 需求 / 验收标准，点击即在右栏预览 -->
    <div class="preview-shortcuts">
      <button class="chip" @click="contractPreview('tech_stack')">技术栈</button>
      <button class="chip" @click="contractPreview('contract')">全局契约</button>
      <button class="chip" @click="contractPreview('requirement')">顶层需求</button>
      <button class="chip" @click="contractPreview('criteria')">验收标准</button>
    </div>

    <div v-if="preview" class="preview-panel">
      <div class="preview-head">
        <div class="preview-title">{{ preview.title }}</div>
        <div v-if="preview.source" class="preview-source">{{ preview.source }}</div>
        <div v-if="preview.time" class="preview-time">{{ preview.time }}</div>
      </div>
      <!-- diff 保持原始等宽文本；其余均为 Agent 产出的 Markdown，按预览渲染。 -->
      <pre v-if="preview.kind === 'diff'" class="preview-content">{{ preview.content || "（空）" }}</pre>
      <div v-else class="preview-content-md">
        <MarkdownView :content="previewMarkdown" />
      </div>
    </div>

    <div v-else class="preview-empty">
      <div class="big">🔍</div>
      <div class="title">选择内容查看详情</div>
      <div class="small muted">
        点击中间对话里的「模块设计方案」「模块完成摘要」「代码变更」，或在左栏点击模块的「设计」按钮，
        这里会显示对应的详细内容。
      </div>
      <button
        v-if="selectedSubtask?.design_content"
        class="btn btn-sm"
        style="margin-top: 10px"
        @click="
          emit('preview', {
            kind: 'design',
            title: '模块设计方案',
            content: selectedSubtask?.design_content ?? '',
            source: `#${selectedSubtask?.sort_index} ${selectedSubtask?.subtask_name}`
          })
        "
      >
        查看当前模块设计
      </button>
    </div>

    <div v-if="selectedSubtask" class="preview-subtask">
      <div class="wb-section-title" style="padding: 0">
        当前模块
        <span class="count">#{{ selectedSubtask.sort_index }}</span>
      </div>
      <div class="small">
        <b>{{ selectedSubtask.subtask_name }}</b>
      </div>
      <div class="small muted" style="margin-top: 4px">
        状态 {{ selectedSubtask.status }} · 第 {{ selectedSubtask.current_round }}/{{ selectedSubtask.max_round }} 轮
      </div>
      <div class="preview-actions">
        <button
          class="link-btn"
          :disabled="!selectedSubtask.design_content"
          @click="
            emit('preview', {
              kind: 'design',
              title: '模块设计方案',
              content: selectedSubtask?.design_content ?? '',
              source: `#${selectedSubtask?.sort_index} ${selectedSubtask?.subtask_name}`
            })
          "
        >
          设计方案
        </button>
        <button
          class="link-btn"
          :disabled="!selectedSubtask.summary_content"
          @click="
            emit('preview', {
              kind: 'summary',
              title: '模块完成摘要',
              content: selectedSubtask?.summary_content ?? '',
              source: `#${selectedSubtask?.sort_index} ${selectedSubtask?.subtask_name}`
            })
          "
        >
          完成摘要
        </button>
      </div>
      <div v-if="selectedSubtask.accept_criteria" class="preview-criteria">
        <MarkdownView :content="selectedSubtask.accept_criteria" />
      </div>
    </div>
  </aside>
</template>
