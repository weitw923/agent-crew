<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import type { AgentConfig, AgentInput, Capability, RoleInput, RoleType } from "../types";
import {
  deleteAgent,
  deleteRole,
  exportAgents,
  importAgents,
  listAgents,
  listRoles,
  saveAgent,
  saveRole
} from "../api";
import {
  CAPABILITIES,
  CAPABILITY_HINTS,
  CAPABILITY_LABELS,
  ROLE_COLOR_PRESETS,
  capabilityText,
  roleBadgeStyle
} from "../roles";
import { errorMessage, showToast } from "../toast";

const tab = ref<"agents" | "roles">("agents");

const agents = ref<AgentConfig[]>([]);
const roles = ref<RoleType[]>([]);
const selectedId = ref<number | null>(null);
const saving = ref(false);
const showKey = ref(false);

const emptyForm = (): AgentInput => ({
  name: "",
  role_type: "reviewer",
  provider: "openai-compatible",
  model_name: "",
  base_url: "",
  api_key: "",
  system_prompt: "",
  temperature: 0.3
});

const form = reactive<AgentInput>(emptyForm());
const isEditing = computed(() => selectedId.value !== null);
const currentRole = computed(() => roles.value.find((r) => r.key === form.role_type) ?? null);
const groupedAgents = computed(() =>
  roles.value
    .map((role) => ({
      role,
      items: agents.value.filter((a) => a.role_type === role.key)
    }))
    .filter((g) => g.items.length)
);

/* ---------------- 角色类型管理 ---------------- */
const roleForm = reactive<RoleInput>({
  key: "",
  name: "",
  description: "",
  capabilities: ["requirement"],
  color: ROLE_COLOR_PRESETS[0]
});
const editingRoleId = ref<number | null>(null);
const editingBuiltin = computed(
  () => roles.value.find((r) => r.id === editingRoleId.value)?.is_builtin ?? false
);

function emptyRoleForm() {
  editingRoleId.value = null;
  Object.assign(roleForm, {
    key: "",
    name: "",
    description: "",
    capabilities: ["requirement"] as Capability[],
    color: ROLE_COLOR_PRESETS[0]
  });
}

function editRole(role: RoleType) {
  editingRoleId.value = role.id;
  Object.assign(roleForm, {
    key: role.key,
    name: role.name,
    description: role.description,
    capabilities: [...role.capabilities],
    color: role.color || ROLE_COLOR_PRESETS[0]
  });
}

function toggleCapability(cap: Capability) {
  if (editingBuiltin.value) return;
  const idx = roleForm.capabilities.indexOf(cap);
  if (idx >= 0) roleForm.capabilities.splice(idx, 1);
  else roleForm.capabilities.push(cap);
}

async function submitRole() {
  if (!roleForm.name.trim()) return showToast("请填写角色名称", "error");
  if (!roleForm.capabilities.length) return showToast("请至少选择一个能力标签", "error");
  saving.value = true;
  try {
    const saved = await saveRole({ ...roleForm, id: editingRoleId.value ?? undefined });
    await load();
    const target = roles.value.find((r) => r.id === saved.id) ?? saved;
    editRole(target);
    showToast("角色已保存", "success");
  } catch (e) {
    showToast(errorMessage(e), "error");
  } finally {
    saving.value = false;
  }
}

async function removeRole(role: RoleType) {
  if (!window.confirm(`确定删除角色「${role.name}」？`)) return;
  try {
    await deleteRole(role.id);
    if (editingRoleId.value === role.id) emptyRoleForm();
    await load();
    showToast("角色已删除", "success");
  } catch (e) {
    showToast(errorMessage(e), "error");
  }
}

/* ---------------- Agent 管理 ---------------- */
async function load(keepSelection = false) {
  try {
    const [agentList, roleList] = await Promise.all([listAgents(), listRoles()]);
    agents.value = agentList;
    roles.value = roleList;
    if (!form.role_type || !roleList.some((r) => r.key === form.role_type)) {
      form.role_type = roleList.find((r) => r.key === "reviewer")?.key ?? roleList[0]?.key ?? "";
    }
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

async function submit() {
  if (!form.name.trim()) return showToast("请填写 Agent 名称", "error");
  if (!form.role_type) return showToast("请选择角色类型", "error");
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
          <p>
            角色类型可扩展：先定义角色能力（需求梳理 / 方案设计 / 编码实现 / 评审验收），
            再为每个角色创建绑定独立模型与提示词的 Agent。
          </p>
        </div>
        <div class="spacer"></div>
        <template v-if="tab === 'agents'">
          <button class="btn btn-sm" @click="importAll">导入配置</button>
          <button class="btn btn-sm" @click="exportAll">导出配置</button>
          <button class="btn btn-primary" @click="resetForm">＋ 新增 Agent</button>
        </template>
        <button v-else class="btn btn-primary" @click="emptyRoleForm">＋ 新增角色</button>
      </div>

      <div class="role-tabs">
        <button class="role-tab" :class="{ active: tab === 'agents' }" @click="tab = 'agents'">
          Agent 列表
          <span class="count">{{ agents.length }}</span>
        </button>
        <button class="role-tab" :class="{ active: tab === 'roles' }" @click="tab = 'roles'">
          角色类型
          <span class="count">{{ roles.length }}</span>
        </button>
      </div>

      <!-- ============ Agent 列表 ============ -->
      <div v-if="tab === 'agents'" class="split">
        <section class="panel">
          <div class="panel-head">
            已配置 Agent
            <span class="sub">
              {{ groupedAgents.map((g) => `${g.role.name} ${g.items.length}`).join(" · ") || "暂无" }}
            </span>
          </div>
          <div class="panel-body">
            <div v-if="!agents.length" class="empty-state">
              <div class="big">🧩</div>
              <div class="title">还没有 Agent 配置</div>
              <div class="small">
                先在右侧创建 Agent；如需产品经理等新角色，先到「角色类型」新增角色。
              </div>
            </div>
            <div v-else class="agent-list">
              <template v-for="group in groupedAgents" :key="group.role.key">
                <div class="agent-group-title">
                  <span class="badge" :style="roleBadgeStyle(group.role.color)">
                    {{ group.role.name }}
                  </span>
                  <span class="small muted">{{ capabilityText(group.role.capabilities) }}</span>
                </div>
                <div
                  v-for="agent in group.items"
                  :key="agent.id"
                  class="agent-item"
                  :class="{ active: agent.id === selectedId }"
                  @click="selectAgent(agent)"
                >
                  <div class="row1">
                    <span class="name">{{ agent.name }}</span>
                    <button class="btn btn-sm btn-danger right" @click.stop="remove(agent)">
                      删除
                    </button>
                  </div>
                  <div class="meta">
                    {{ agent.provider }} / {{ agent.model_name }} · temp {{ agent.temperature }}
                  </div>
                  <div class="meta">{{ agent.base_url }}</div>
                </div>
              </template>
            </div>
          </div>
        </section>

        <!-- 右：新增/编辑 Agent -->
        <section class="panel form-panel">
          <div class="flex" style="margin-bottom: 16px">
            <h3 style="margin: 0; font-size: 15px">
              {{ isEditing ? "编辑 Agent" : "新增 Agent" }}
            </h3>
            <span v-if="isEditing" class="badge badge-blue">ID {{ selectedId }}</span>
            <button v-if="isEditing" class="btn btn-sm btn-ghost right" @click="resetForm">
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
              <select v-model="form.role_type">
                <option v-for="role in roles" :key="role.key" :value="role.key">
                  {{ role.name }}{{ role.is_builtin ? "（内置）" : "" }}
                </option>
              </select>
              <span v-if="currentRole" class="hint">
                能力：{{ capabilityText(currentRole.capabilities) }}
                <template v-if="currentRole.capabilities.includes('code')">
                  · 该角色由 dsh 执行写代码
                </template>
                <template v-else> · 该角色走直接 LLM 调用</template>
              </span>
              <span v-else class="error-text">请先到「角色类型」创建一个角色</span>
            </div>

            <div class="field">
              <label class="required">模型服务商 / API 协议</label>
              <select v-model="form.provider">
                <option value="openai-compatible">OpenAI 兼容（通用）</option>
                <option value="deepseek">DeepSeek</option>
                <option value="openai">OpenAI</option>
                <option value="anthropic">Anthropic Messages</option>
              </select>
              <span class="hint">
                非编码角色直连 HTTP；编码角色会写入 dsh 的 provider patch。
              </span>
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
                placeholder="描述该 Agent 的职责、代码规范、评审尺度等。留空则使用系统内置角色提示词。"
              ></textarea>
            </div>

            <div class="field span-2">
              <label>temperature：{{ form.temperature.toFixed(1) }}</label>
              <input v-model.number="form.temperature" type="range" min="0" max="2" step="0.1" />
              <span class="hint">
                用于直接 LLM 调用；编码角色由 dsh 执行，dsh 当前不透传 temperature。
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

      <!-- ============ 角色类型 ============ -->
      <div v-else class="split">
        <section class="panel">
          <div class="panel-head">
            全部角色
            <span class="sub">
              {{ roles.filter((r) => r.is_builtin).length }} 个内置 · 可新增自定义角色
            </span>
          </div>
          <div class="panel-body">
            <div class="role-list">
              <div
                v-for="role in roles"
                :key="role.id"
                class="role-item"
                :class="{ active: editingRoleId === role.id }"
                @click="editRole(role)"
              >
                <div class="row1">
                  <span class="role-dot" :style="{ background: role.color || '#8b97aa' }"></span>
                  <span class="name">{{ role.name }}</span>
                  <span v-if="role.is_builtin" class="badge badge-blue">内置</span>
                  <span class="small muted right">{{ role.agent_count }} 个 Agent</span>
                </div>
                <div class="meta">
                  <span class="role-key">{{ role.key }}</span>
                  · {{ capabilityText(role.capabilities) }}
                </div>
                <div v-if="role.description" class="meta">{{ role.description }}</div>
                <div class="row1" style="margin-top: 6px">
                  <button
                    class="link-btn"
                    :disabled="role.is_builtin || role.agent_count > 0"
                    :title="
                      role.is_builtin
                        ? '内置角色不可删除'
                        : role.agent_count > 0
                          ? '仍有 Agent 使用该角色'
                          : '删除角色'
                    "
                    @click.stop="removeRole(role)"
                  >
                    删除角色
                  </button>
                </div>
              </div>
            </div>
          </div>
        </section>

        <section class="panel form-panel">
          <div class="flex" style="margin-bottom: 16px">
            <h3 style="margin: 0; font-size: 15px">
              {{ editingRoleId ? "编辑角色" : "新增角色" }}
            </h3>
            <span v-if="editingBuiltin" class="badge badge-yellow">内置角色</span>
            <button v-if="editingRoleId" class="btn btn-sm btn-ghost right" @click="emptyRoleForm">
              取消编辑
            </button>
          </div>

          <div class="form-grid">
            <div class="field">
              <label class="required">角色名称</label>
              <input v-model="roleForm.name" type="text" placeholder="例如：产品经理" />
            </div>
            <div class="field">
              <label>角色标识 key</label>
              <input
                v-model="roleForm.key"
                type="text"
                :disabled="!!editingRoleId"
                placeholder="自动生成，例如 product_manager"
              />
              <span class="hint">唯一标识，创建后不可修改；内置角色固定为 reviewer / coder。</span>
            </div>

            <div class="field span-2">
              <label class="required">能力标签</label>
              <div class="capability-grid">
                <label
                  v-for="cap in CAPABILITIES"
                  :key="cap"
                  class="capability-card"
                  :class="{ active: roleForm.capabilities.includes(cap), disabled: editingBuiltin }"
                >
                  <input
                    type="checkbox"
                    :checked="roleForm.capabilities.includes(cap)"
                    :disabled="editingBuiltin"
                    @change="toggleCapability(cap)"
                  />
                  <div>
                    <b>{{ CAPABILITY_LABELS[cap] }}</b>
                    <small>{{ CAPABILITY_HINTS[cap] }}</small>
                  </div>
                </label>
              </div>
              <span v-if="editingBuiltin" class="hint">
                内置角色的能力与标识不可修改，仅可补充说明。
              </span>
            </div>

            <div class="field span-2">
              <label>角色颜色</label>
              <div class="color-row">
                <button
                  v-for="color in ROLE_COLOR_PRESETS"
                  :key="color"
                  class="color-swatch"
                  :class="{ active: roleForm.color === color }"
                  :style="{ background: color }"
                  :title="color"
                  :disabled="editingBuiltin"
                  @click="roleForm.color = color"
                ></button>
                <span class="badge" :style="roleBadgeStyle(roleForm.color)">
                  预览 · {{ roleForm.name || "角色名" }}
                </span>
              </div>
            </div>

            <div class="field span-2">
              <label>角色说明</label>
              <textarea
                v-model="roleForm.description"
                rows="4"
                placeholder="说明这个角色负责什么、在工作流的哪个位置。"
              ></textarea>
            </div>
          </div>

          <div class="form-actions">
            <button class="btn btn-ghost" @click="emptyRoleForm">重置</button>
            <button class="btn btn-primary" :disabled="saving" @click="submitRole">
              {{ saving ? "保存中..." : editingRoleId ? "保存角色" : "创建角色" }}
            </button>
          </div>
        </section>
      </div>
    </div>
  </div>
</template>
