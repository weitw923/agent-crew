<script setup lang="ts">
import { onMounted, reactive, ref } from "vue";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import type { AppSettings } from "../types";
import { exportAgents, getSettings, importAgents, saveSettings } from "../api";
import { errorMessage, showToast } from "../toast";
import { setTheme, theme } from "../theme";

const form = reactive<AppSettings>({
  default_work_dir: "",
  llm_timeout_sec: "180",
  dsh_timeout_sec: "1800",
  dsh_bin: "dsh",
  dsh_profile: "headless",
  storage_root: "",
  log_persistence: true,
  default_execution_mode: "later",
  default_module_max_round: "3",
  default_global_max_round: "2",
  max_concurrent_tasks: "1",
  scheduled_enabled: true,
  llm_rate_limit_per_min: "0"
});
const loading = ref(true);
const saving = ref(false);
const originalStorage = ref("");

async function load() {
  loading.value = true;
  try {
    Object.assign(form, await getSettings());
    originalStorage.value = form.storage_root;
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    loading.value = false;
  }
}

async function save() {
  if (Number(form.llm_timeout_sec) < 5) return showToast("LLM 超时至少 5 秒", "error");
  if (Number(form.dsh_timeout_sec) < 30) return showToast("dsh 超时至少 30 秒", "error");
  if (!form.storage_root.trim()) return showToast("请选择存储目录", "error");
  if (Number(form.max_concurrent_tasks) < 1) return showToast("最大并发任务数至少为 1", "error");
  if (Number(form.default_module_max_round) < 1 || Number(form.default_global_max_round) < 1)
    return showToast("默认评审轮次至少为 1", "error");
  if (Number(form.llm_rate_limit_per_min) < 0) return showToast("限速值不能为负数", "error");
  saving.value = true;
  try {
    const saved = await saveSettings({ ...form });
    const moved = originalStorage.value && saved.storage_root !== originalStorage.value;
    Object.assign(form, saved);
    originalStorage.value = saved.storage_root;
    showToast(
      moved ? "设置已保存，任务存储目录已迁移（旧目录保留）" : "设置已保存",
      "success"
    );
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    saving.value = false;
  }
}

async function pickDefaultDir() {
  try {
    const dir = await openDialog({ directory: true, multiple: false, title: "选择默认工作目录" });
    if (typeof dir === "string") form.default_work_dir = dir;
  } catch (e) {
    showToast(errorMessage(e), "error");
  }
}

async function pickStorageDir() {
  try {
    const dir = await openDialog({
      directory: true,
      multiple: false,
      title: "选择任务数据/日志/模块快照的存储目录（必须在项目目录外）"
    });
    if (typeof dir === "string") {
      if (form.storage_root && form.storage_root !== dir) {
        const ok = window.confirm(
          "保存后会迁移数据库与模块快照到新目录；旧目录会保留作为备份。仍要更换吗？"
        );
        if (!ok) return;
      }
      form.storage_root = dir;
    }
  } catch (e) {
    showToast(errorMessage(e), "error");
  }
}

function chooseTheme(value: "dark" | "light") {
  setTheme(value);
}

async function exportAll() {
  try {
    const path = await saveDialog({
      title: "导出全部 Agent 配置",
      defaultPath: "agent-configs.json",
      filters: [{ name: "JSON", extensions: ["json"] }]
    });
    if (!path) return;
    const n = await exportAgents(path);
    showToast(`已导出 ${n} 条 Agent 配置`, "success");
  } catch (e) {
    showToast(errorMessage(e), "error");
  }
}

async function importAll() {
  try {
    const path = await openDialog({
      title: "导入 Agent 配置",
      multiple: false,
      filters: [{ name: "JSON", extensions: ["json"] }]
    });
    if (!path || Array.isArray(path)) return;
    const n = await importAgents(path);
    showToast(`已导入 ${n} 条 Agent 配置`, "success");
  } catch (e) {
    showToast(errorMessage(e), "error");
  }
}

onMounted(load);
</script>

<template>
  <div class="view-scroll">
    <div class="view-inner" style="max-width: 980px">
      <div class="page-head">
        <div>
          <h1>设置</h1>
          <p>存储位置、运行日志、主题、默认执行模式与调用参数统一在这里管理。</p>
        </div>
        <div class="spacer"></div>
        <button class="btn btn-primary" :disabled="saving" @click="save">
          {{ saving ? "保存中…" : "保存设置" }}
        </button>
      </div>

      <div v-if="loading" class="panel"><div class="panel-body muted">加载中…</div></div>

      <template v-else>
        <section class="panel">
          <div class="panel-head">存储与运行日志</div>
          <div class="panel-body">
            <div class="field">
              <label class="required">任务数据 / 日志 / 模块快照存储目录</label>
              <div class="flex">
                <input
                  v-model="form.storage_root"
                  type="text"
                  placeholder="/Users/you/AgentCrewData"
                />
                <button class="btn" @click="pickStorageDir">选择目录</button>
              </div>
              <span class="hint">
                数据库、运行日志、模块设计 / diff / summary 快照统一保存在这里，必须在项目目录外部。
                更换目录时会自动快照迁移，旧目录保留不删除。迁移前需要先暂停所有任务。
              </span>
            </div>
            <div class="settings-divider"></div>
            <label class="switch-row">
              <input v-model="form.log_persistence" type="checkbox" />
              <span>
                <b>持久化运行日志</b>
                <small>
                  开启：保存 Agent 对话、工具调用、评审和修改历史，可回看全链路。
                  关闭：只在当前页面实时展示，不写入数据库。
                </small>
              </span>
            </label>
          </div>
        </section>

        <section class="panel" style="margin-top: 16px">
          <div class="panel-head">界面主题</div>
          <div class="panel-body">
            <div class="theme-cards">
              <button
                class="theme-card"
                :class="{ active: theme === 'light' }"
                @click="chooseTheme('light')"
              >
                <span class="theme-preview light"></span>
                <b>浅色</b>
                <small>明亮清爽，适合白天</small>
              </button>
              <button
                class="theme-card"
                :class="{ active: theme === 'dark' }"
                @click="chooseTheme('dark')"
              >
                <span class="theme-preview dark"></span>
                <b>深色</b>
                <small>低亮度，适合夜间</small>
              </button>
            </div>
            <div class="small muted" style="margin-top: 8px">主题选择会持久保存，下次启动继续使用。</div>
          </div>
        </section>

        <section class="panel" style="margin-top: 16px">
          <div class="panel-head">任务执行默认值</div>
          <div class="panel-body">
            <div class="form-grid">
              <div class="field">
                <label>默认执行模式</label>
                <select v-model="form.default_execution_mode">
                  <option value="later">稍后执行（推荐）</option>
                  <option value="immediate">立即执行</option>
                  <option value="scheduled">定时执行</option>
                </select>
                <span class="hint">新建任务默认选中，仍可在创建时单独修改。</span>
              </div>
              <div class="field">
                <label>最大并发任务数</label>
                <input v-model="form.max_concurrent_tasks" type="number" min="1" max="32" />
                <span class="hint">超过上限的任务进入等待队列，不会同时消耗 token。</span>
              </div>
              <div class="field">
                <label>默认真子任务最大轮次</label>
                <input v-model="form.default_module_max_round" type="number" min="1" max="99" />
              </div>
              <div class="field">
                <label>默认全局评审最大轮次</label>
                <input v-model="form.default_global_max_round" type="number" min="1" max="99" />
              </div>
              <div class="field span-2">
                <label class="switch-row" style="border: 0; padding: 0; background: transparent">
                  <input v-model="form.scheduled_enabled" type="checkbox" />
                  <span>
                    <b>启用定时任务全局开关</b>
                    <small>关闭后，所有定时任务都不会在窗口内自动启动，但仍可手动运行。</small>
                  </span>
                </label>
              </div>
            </div>
          </div>
        </section>

        <section class="panel" style="margin-top: 16px">
          <div class="panel-head">项目默认与调用配置</div>
          <div class="panel-body">
            <div class="field">
              <label>默认本地工作目录</label>
              <div class="flex">
                <input v-model="form.default_work_dir" type="text" placeholder="/Users/you/Projects" />
                <button class="btn" @click="pickDefaultDir">选择目录</button>
              </div>
              <span class="hint">新建任务时自动填入，仍可单独修改。</span>
            </div>
            <div class="settings-divider"></div>
            <div class="form-grid">
              <div class="field">
                <label>LLM 调用超时（秒）</label>
                <input v-model="form.llm_timeout_sec" type="number" min="5" />
              </div>
              <div class="field">
                <label>dsh 单次执行超时（秒）</label>
                <input v-model="form.dsh_timeout_sec" type="number" min="30" />
              </div>
              <div class="field">
                <label>dsh 可执行文件</label>
                <input v-model="form.dsh_bin" type="text" placeholder="dsh 或 /opt/homebrew/bin/dsh" />
              </div>
              <div class="field">
                <label>dsh profile</label>
                <input v-model="form.dsh_profile" type="text" placeholder="headless" />
              </div>
              <div class="field">
                <label>LLM 每分钟请求上限</label>
                <input v-model="form.llm_rate_limit_per_min" type="number" min="0" />
                <span class="hint">0 表示不限；用于防止短时间大量消耗 token。</span>
              </div>
            </div>
          </div>
        </section>

        <section class="panel" style="margin-top: 16px">
          <div class="panel-head">Agent 配置导入 / 导出</div>
          <div class="panel-body">
            <div class="flex flex-wrap">
              <button class="btn" @click="importAll">导入全部 Agent 配置</button>
              <button class="btn" @click="exportAll">导出全部 Agent 配置</button>
            </div>
            <div class="small muted" style="margin-top: 9px">
              导出为本地 JSON；apiKey 会以明文包含在文件中，请妥善保管。
            </div>
          </div>
        </section>

        <section class="panel" style="margin-top: 16px">
          <div class="panel-head">Coder 与 Harness CLI 的对接方式</div>
          <div class="panel-body small" style="color: var(--muted); line-height: 1.9">
            <div>
              每次派发 Coder，应用会按该 Coder Agent 的
              <b style="color: var(--text)">baseUrl / apiKey / model</b>
              生成一份临时 dsh provider patch，然后执行：
            </div>
            <pre class="pre-box" style="margin-top: 9px">dsh --profile headless --patch &lt;runtime-patch.yml&gt; --json -</pre>
            <div style="margin-top: 8px">
              prompt 从 stdin 传入，工作目录为任务选择的项目目录；dsh 使用自带
              read_file/write/bash/git 工具真实编码。应用只负责快照 git tree、计算 diff、调度评审。
            </div>
            <div style="margin-top: 8px">
              所有任务数据、日志和模块快照保存在设置页指定的外部存储目录，不写入项目目录。
            </div>
          </div>
        </section>

        <div class="flex" style="justify-content: flex-end; margin-top: 18px">
          <button class="btn btn-primary" :disabled="saving" @click="save">
            {{ saving ? "保存中…" : "保存设置" }}
          </button>
        </div>
      </template>
    </div>
  </div>
</template>
