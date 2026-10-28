<script setup lang="ts">
import { nextTick, onMounted, ref } from "vue";
import type { ChatMessage } from "../types";
import { clearChatMessages, getTaskDetail, sendChatMessage } from "../api";
import { errorMessage, showToast } from "../toast";

const props = defineProps<{
  taskId: number;
  taskName: string;
  reviewerName: string;
}>();
const emit = defineEmits<{ (e: "close"): void }>();

const messages = ref<ChatMessage[]>([]);
const input = ref("");
const busy = ref(false);
const loading = ref(true);
const box = ref<HTMLElement | null>(null);

async function load() {
  loading.value = true;
  try {
    const detail = await getTaskDetail(props.taskId);
    messages.value = detail.chat_messages ?? [];
    await nextTick();
    scrollBottom();
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    loading.value = false;
  }
}

function scrollBottom() {
  nextTick(() => {
    if (box.value) box.value.scrollTop = box.value.scrollHeight;
  });
}

async function send() {
  const content = input.value.trim();
  if (!content || busy.value) return;
  input.value = "";
  busy.value = true;
  try {
    const user: ChatMessage = {
      id: -Date.now(),
      task_id: props.taskId,
      role: "user",
      content,
      create_time: ""
    };
    messages.value.push(user);
    scrollBottom();
    const reply = await sendChatMessage(props.taskId, content);
    messages.value.push(reply);
    scrollBottom();
  } catch (e) {
    showToast(errorMessage(e), "error");
    await load();
  } finally {
    busy.value = false;
  }
}

async function clearChat() {
  if (!window.confirm("清空本任务的临时问答历史？不会影响任务流水线、日志和代码。")) return;
  try {
    await clearChatMessages(props.taskId);
    messages.value = [];
    showToast("临时会话已清空", "success");
  } catch (e) {
    showToast(errorMessage(e), "error");
  }
}

onMounted(load);
</script>

<template>
  <div class="modal-mask" @click.self="emit('close')">
    <div class="modal chat-modal">
      <div class="modal-head">
        <div>
          临时问答 · {{ taskName }}
          <div class="small muted" style="font-size: 11px; font-weight: 400">
            独立会话，不会触发子任务派发或自动开发流程
          </div>
        </div>
        <button class="x" @click="emit('close')">×</button>
      </div>
      <div ref="box" class="chat-body">
        <div v-if="loading" class="muted small">加载会话…</div>
        <div v-else-if="!messages.length" class="chat-empty">
          <div class="big">💬</div>
          <div class="title">临时咨询技术总监</div>
          <div class="small muted">
            可以问接口设计、技术选型、当前模块实现思路；这里只回答问题，不会启动开发。
          </div>
        </div>
        <div
          v-for="m in messages"
          :key="m.id"
          class="chat-msg"
          :class="m.role === 'user' ? 'user' : 'assistant'"
        >
          <div class="chat-avatar">{{ m.role === "user" ? "你" : "总" }}</div>
          <div class="chat-bubble">
            <div class="chat-meta">
              {{ m.role === "user" ? "你" : reviewerName || "技术总监" }}
              <span v-if="m.create_time">{{ m.create_time.slice(11) }}</span>
            </div>
            <div class="chat-text">{{ m.content }}</div>
          </div>
        </div>
      </div>
      <div class="chat-composer">
        <textarea
          v-model="input"
          rows="3"
          placeholder="例如：这个模块为什么选 JWT？如果想改成 Session 会影响哪些模块？"
          @keydown.ctrl.enter.prevent="send"
        ></textarea>
        <div class="chat-actions">
          <button class="btn btn-sm btn-ghost" @click="clearChat">清空会话</button>
          <span class="small muted">Ctrl + Enter 发送 · 仅问答，不派发任务</span>
          <button class="btn btn-sm btn-primary" :disabled="busy || !input.trim()" @click="send">
            {{ busy ? "思考中…" : "发送" }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>
