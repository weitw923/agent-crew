<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import type { AgentConfig, AgentInput, RoleType } from "../types";
import {
  deleteAgent,
  exportAgents,
  importAgents,
  listAgents,
  saveAgent
} from "../api";
import { errorMessage, showToast } from "../toast";

const agents = ref<AgentConfig[]>([]);
const selectedId = ref<number | null>(null);
const saving = ref(false);
const showKey = ref(false);

const emptyForm = (): AgentInput => ({
  name: "",
  role_type: "coder",
  provider: "openai-compatible",
  model_name: "",
  base_url: "",
  api_key: "",
  system_prompt: "",
  temperature: 0.3
});

const form = reactive<AgentInput>(emptyForm());

const isEditing = computed(() => selectedId.value !== null);
const reviewers = computed(() => agents.value.filter((a) => a.role_type === "reviewer"));
const coders = computed(() => agents.value.filter((a) => a.role_type === "coder"));

async function load(keepSelection = false) {
  try {
    agents.value = await listAgents();
    if (keepSelection && selectedId.value !== null) {
      const found = agents.value.find((a) => a.id === selectedId.value);
      if (found) selectAgent(found);
      else resetForm();
    }
  } catch (e) {
    showToast(errorMessage(e), "error");
  }
}

function selectAgent(agent: AgentConfig) {
  selectedId.value = agent.id;
  Object.assign(form, {
    name: agent.name,
    role_type: agent.role_type,
    provider: agent.provider,
    model_name: agent.model_name,
    base_url: agent.base_url,
    api_key: agent.api_key,
    system_prompt: agent.system_prompt,
    temperature: agent.temperature
  });
}

function resetForm() {
  selectedId.value = null;
  Object.assign(form, emptyForm());
}

function setRole(role: RoleType) {
  form.role_type = role;
}

async function submit() {
  if (!form.name.trim()) return showToast("请填写 Agent 名称", "error");
  if (!form.model_name.trim()) return showToast("请填写 model 名称", "error");
  if (!form.base_url.trim()) return showToast("请填写 baseUrl", "error");
  saving.value = true;
  try {
    const saved = await saveAgent({ ...form, id: selectedId.value ?? undefined });
    await load();
    selectedId.value = saved.id;
    selectAgent(saved);
    showToast("Agent 配置已保存", "success");
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    saving.value = false;
  }
}

async function remove(agent: AgentConfig) {
  if (!window.confirm(`确定删除 Agent「${agent.name}」？`)) return;
  try {
    await deleteAgent(agent.id);
    if (selectedId.value === agent.id) resetForm();
    await load();
    showToast("已删除", "success");
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
    await load();
    showToast(`已导入 ${n} 条 Agent 配置`, "success");
  } catch (e) {
    showToast(errorMessage(e), "error");
  }
}

onMounted(load);
</script>

<template>
  <div class="view-scroll">
    <div class="view-inner">
      <div class="page-head">
        <div>
          <h1>Agent 配置管理</h1>
          <p>预先配置评审者与编码者；每条配置绑定独立模型、密钥与 system prompt。</p>
        </div>
        <div class="spacer"></div>
        <button class="btn btn-sm" @click="importAll">导入配置</button>
        <button class="btn btn-sm" @click="exportAll">导出配置</button>
        <button class="btn btn-primary" @click="resetForm">＋ 新增 Agent</button>
      </div>

      <div class="split">
        <!-- 左：Agent 列表 -->
        <section class="panel">
          <div class="panel-head">
            已配置 Agent
            <span class="sub">
              评审者 {{ reviewers.length }} · 编码者 {{ coders.length }}
            </span>
          </div>
          <div class="panel-body">
            <div v-if="!agents.length" class="empty-state">
              <div class="big">🧩</div>
              <div class="title">还没有 Agent 配置</div>
              <div class="small">先在右侧创建 1 个 Reviewer 和至少 1 个 Coder</div>
            </div>
            <div v-else class="agent-list">
              <div
                v-for="agent in agents"
                :key="agent.id"
                class="agent-item"
                :class="{ active: agent.id === selectedId }"
                @click="selectAgent(agent)"
              >
                <div class="row1">
                  <span class="name">{{ agent.name }}</span>
                  <span
                    class="badge"
                    :class="agent.role_type === 'reviewer' ? 'badge-reviewer' : 'badge-coder'"
                  >
                    {{ agent.role_type === "reviewer" ? "Reviewer" : "Coder" }}
                  </span>
                  <button
                    class="btn btn-sm btn-danger right"
                    @click.stop="remove(agent)"
                  >
                    删除
                  </button>
                </div>
                <div class="meta">
                  {{ agent.provider }} / {{ agent.model_name }} · temp {{ agent.temperature }}
                </div>
                <div class="meta">{{ agent.base_url }}</div>
              </div>
            </div>
          </div>
        </section>

        <!-- 右：新增/编辑表单 -->
        <section class="panel form-panel">
          <div class="flex" style="margin-bottom: 16px">
            <h3 style="margin: 0; font-size: 15px">
              {{ isEditing ? "编辑 Agent" : "新增 Agent" }}
            </h3>
            <span v-if="isEditing" class="badge badge-blue">ID {{ selectedId }}</span>
            <button
              v-if="isEditing"
              class="btn btn-sm btn-ghost right"
              @click="resetForm"
            >
              取消编辑
            </button>
          </div>

          <div class="form-grid">
            <div class="field">
              <label class="required">Agent 名称</label>
              <input v-model="form.name" type="text" placeholder="例如：严格后端评审 / 全栈编码手" />
            </div>

            <div class="field">
              <label class="required">角色类型</label>
              <div class="flex">
                <button
                  class="btn btn-sm"
                  :class="{ 'btn-primary': form.role_type === 'reviewer' }"
                  @click="setRole('reviewer')"
                >
                  Reviewer 评审者
                </button>
                <button
                  class="btn btn-sm"
                  :class="{ 'btn-primary': form.role_type === 'coder' }"
                  @click="setRole('coder')"
                >
                  Coder 编码者
                </button>
              </div>
            </div>

            <div class="field">
              <label class="required">模型服务商 / API 协议</label>
              <select v-model="form.provider">
                <option value="openai-compatible">OpenAI 兼容（通用）</option>
                <option value="deepseek">DeepSeek</option>
                <option value="openai">OpenAI</option>
                <option value="anthropic">Anthropic Messages</option>
              </select>
              <span class="hint">Reviewer 直连 HTTP；Coder 会写入 dsh 的 provider patch。</span>
            </div>

            <div class="field">
              <label class="required">model 名称</label>
              <input v-model="form.model_name" type="text" placeholder="例如：deepseek-flash / gpt-4.1" />
              <span class="hint">
                必须填服务商 API 的 model ID，不要填界面展示名。DeepSeek 支持
                deepseek-flash / deepseek-v4-pro。
              </span>
            </div>

            <div class="field span-2">
              <label class="required">baseUrl</label>
              <input
                v-model="form.base_url"
                type="text"
                placeholder="例如：https://api.deepseek.com/v1"
              />
              <span class="hint">
                填到版本前缀即可（/v1）；也支持直接填完整的 /chat/completions 地址。
              </span>
            </div>

            <div class="field span-2">
              <label class="required">apiKey</label>
              <div class="flex">
                <input
                  v-model="form.api_key"
                  :type="showKey ? 'text' : 'password'"
                  placeholder="sk-..."
                  autocomplete="off"
                />
                <button class="btn btn-sm" @click="showKey = !showKey">
                  {{ showKey ? "隐藏" : "显示" }}
                </button>
              </div>
              <span class="hint">仅保存在本机 SQLite，不上传任何云端。</span>
            </div>

            <div class="field span-2">
              <label class="required">systemPrompt</label>
              <textarea
                v-model="form.system_prompt"
                rows="7"
                placeholder="描述该 Agent 的职责、代码规范、评审尺度等。"
              ></textarea>
            </div>

            <div class="field span-2">
              <label>temperature：{{ form.temperature.toFixed(1) }}</label>
              <input v-model.number="form.temperature" type="range" min="0" max="2" step="0.1" />
              <span class="hint">
                用于 Reviewer 请求；Coder 由 dsh 执行，dsh 当前不透传 temperature。
              </span>
            </div>
          </div>

          <div class="form-actions">
            <button class="btn btn-ghost" @click="resetForm">重置</button>
            <button class="btn btn-primary" :disabled="saving" @click="submit">
              {{ saving ? "保存中..." : isEditing ? "保存修改" : "创建 Agent" }}
            </button>
          </div>
        </section>
      </div>
    </div>
  </div>
</template>
