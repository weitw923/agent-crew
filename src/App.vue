<script setup lang="ts">
import { ref } from "vue";
import AgentsView from "./views/AgentsView.vue";
import TasksView from "./views/TasksView.vue";
import WorkbenchView from "./views/WorkbenchView.vue";
import SettingsView from "./views/SettingsView.vue";
import { toasts } from "./toast";

type TabKey = "agents" | "tasks" | "workbench" | "settings";

const activeTab = ref<TabKey>("tasks");
const activeTaskId = ref<number | null>(null);

function openTask(id: number) {
  activeTaskId.value = id;
  activeTab.value = "workbench";
}

function switchTab(tab: TabKey) {
  // 没有打开过任务时，工作台 Tab 只显示引导页。
  activeTab.value = tab;
}
</script>

<template>
  <div class="app-shell">
    <header class="topbar">
      <div class="brand">
        <div class="brand-mark">A</div>
        <div>
          <div class="brand-title">AgentCrew</div>
          <div class="brand-sub">多角色 · 串行增量评审</div>
        </div>
      </div>

      <nav class="tab-nav">
        <button
          class="tab-btn"
          :class="{ active: activeTab === 'agents' }"
          @click="switchTab('agents')"
        >
          Agent 配置
        </button>
        <button
          class="tab-btn"
          :class="{ active: activeTab === 'tasks' }"
          @click="switchTab('tasks')"
        >
          任务列表
        </button>
        <button
          class="tab-btn"
          :class="{ active: activeTab === 'workbench' }"
          @click="switchTab('workbench')"
        >
          任务工作台
        </button>
        <button
          class="tab-btn"
          :class="{ active: activeTab === 'settings' }"
          @click="switchTab('settings')"
        >
          设置
        </button>
      </nav>

      <div class="topbar-right">
        <span class="dot"></span>
        <span>本地 SQLite · 数据不上云</span>
      </div>
    </header>

    <main class="main-area">
      <AgentsView v-if="activeTab === 'agents'" />
      <TasksView v-else-if="activeTab === 'tasks'" @open-task="openTask" />
      <WorkbenchView
        v-else-if="activeTab === 'workbench'"
        :task-id="activeTaskId"
        @back="activeTab = 'tasks'"
        @open-task="openTask"
      />
      <SettingsView v-else />
    </main>

    <div class="toast-wrap">
      <div v-for="t in toasts" :key="t.id" class="toast" :class="t.kind">
        {{ t.message }}
      </div>
    </div>
  </div>
</template>
