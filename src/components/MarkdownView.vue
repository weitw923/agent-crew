<script setup lang="ts">
import { computed } from "vue";
import { renderMarkdown } from "../markdown";

const props = defineProps<{
  content: string;
  /** 空内容占位文案。 */
  placeholder?: string;
}>();

const html = computed(() => renderMarkdown(props.content));
</script>

<template>
  <div v-if="html" class="md-body" v-html="html"></div>
  <div v-else class="md-body md-empty">{{ placeholder ?? "（空）" }}</div>
</template>
