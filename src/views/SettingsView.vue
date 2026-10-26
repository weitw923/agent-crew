<script setup lang="ts">
import { onMounted, reactive, ref } from "vue";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import type { AppSettings } from "../types";
import { exportAgents, getSettings, importAgents, saveSettings } from "../api";
import { errorMessage, showToast } from "../toast";

const form = reactive<AppSettings>({
  default_work_dir: "",
  llm_timeout_sec: "180",
  dsh_timeout_sec: "1800",
  dsh_bin: "dsh",
  dsh_profile: "headless"
});
const loading = ref(true);
const saving = ref(false);

async function load() {
  loading.value = true;
  try {
    Object.assign(form, await getSettings());
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    loading.value = false;
  }
}

async function save() {
  if (Number(form.llm_timeout_sec) < 5) return showToast("LLM 超时至少 5 秒", "error");
  if (Number(form.dsh_timeout_sec) < 30) return showToast("dsh 超时至少 30 秒", "error");
  saving.value = true;
  try {
    Object.assign(form, await saveSettings({ ...form }));
    showToast("设置已保存", "success");
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
    <div class="view-inner" style="max-width: 900px">
      <div class="page-head">
        <div>
          <h1>设置</h1>
          <p>极简本地设置：默认工作目录、LLM 调用超时、dsh CLI 入口、配置导入导出。</p>
        </div>
      </div>

      <div v-if="loading" class="panel"><div class="panel-body muted">加载中…</div></div>

      <template v-else>
        <section class="panel">
          <div class="panel-head">项目默认</div>
          <div class="panel-body">
            <div class="field">
              <label>默认本地工作目录</label>
              <div class="flex">
                <input v-model="form.default_work_dir" type="text" placeholder="/Users/you/Projects" />
                <button class="btn" @click="pickDefaultDir">选择目录</button>
              </div>
              <span class="hint">新建任务时自动填入，仍可单独修改。</span>
            </div>
          </div>
        </section>

        <section class="panel" style="margin-top: 16px">
          <div class="panel-head">调用配置</div>
          <div class="panel-body">
            <div class="form-grid">
              <div class="field">
                <label>LLM 调用超时（秒）</label>
                <input v-model="form.llm_timeout_sec" type="number" min="5" />
                <span class="hint">Reviewer 直连 LLM 的 HTTP 超时。</span>
              </div>
              <div class="field">
                <label>dsh 单次执行超时（秒）</label>
                <input v-model="form.dsh_timeout_sec" type="number" min="30" />
                <span class="hint">Coder 调用 dsh 执行编码/生成摘要的最长等待时间。</span>
              </div>
              <div class="field">
                <label>dsh 可执行文件</label>
                <input v-model="form.dsh_bin" type="text" placeholder="dsh 或 /opt/homebrew/bin/dsh" />
                <span class="hint">
                  GUI 应用 PATH 可能不完整；找不到命令时建议填绝对路径。
                </span>
              </div>
              <div class="field">
                <label>dsh profile</label>
                <input v-model="form.dsh_profile" type="text" placeholder="headless" />
                <span class="hint">使用 `dsh --profile &lt;profile&gt; ...` 调用。</span>
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
              Coder 执行时设置 <code>DSH_PERMISSION_MODE=danger-full-access</code>，以便无人值守地修改工作目录。
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
