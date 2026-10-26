# AgentCrew（Tauri + Vue3 + SQLite）

AgentCrew 是一个**本地优先、轻量、串行**的多 Agent 代码开发编排桌面工具。

当前版本内置两类基础角色，后续可扩展更多角色：

- **评审者 Reviewer**：负责拆解需求、代码 diff 评审、摘要校验、全局收口。
- **编码者 Coder**：通过 **Harness CLI（`dsh`）** 调用其自带的 `read_file / write / bash / git` 工具，在真实项目目录中写代码。

应用本身只负责：配置管理、任务拆解、状态机调度、prompt 组装、diff 捕获、评审 JSON 解析、SQLite 归档、人工兜底。**不实现**内置编辑器/终端、MCP 插件市场、自动 git 冲突合并、复杂依赖拓扑、多用户、RAG。

---

## 1. 功能总览

| Tab | 功能 |
| --- | --- |
| Agent 配置 | 新增/编辑/删除/导入/导出 Agent；绑定角色、服务商、model、baseUrl、apiKey、systemPrompt、temperature |
| 任务列表 | 历史任务卡片、新建任务弹窗、打开详情、删除任务 |
| 任务工作台 | 左：子任务队列 + 每轮历史 + 待审批方案；中：设计审批/当前 diff/评审/结论 + 运行中配置 + 人工消息；右：技术栈/全局契约 + 摘要 + **实时执行输出** + 系统日志 |
| 设置 | 默认工作目录、LLM 超时、dsh 超时、dsh 可执行文件/profile、Agent 配置导入导出 |

核心流程：

```
拆解顶层需求 -> 技术总监输出【技术栈 + 全局契约 + 有序子任务 + 验收标准】
  ├─（人工审核开启）等待人工审批方案，通过后才创建子任务
  └─（人工审核关闭）直接生效
  ↓
按队列串行取当前子任务
  ↓
技术总监生成当前模块详细设计文档
  ├─（人工审核开启）等待人工逐一审批设计，通过后才允许 Coder 开工
  └─（人工审核关闭）自动通过
  ↓
Coder（dsh）写代码  -> 实时流式输出 + 捕获本轮 git diff
  ↓
Reviewer 评审【契约 + 已完成模块摘要 + 当前子任务 + 本轮 diff】
  ├─ 不通过 -> 轮次+1 -> 同一 Coder 修改（直到子任务最大轮次 -> 人工介入）
  └─ 通过   -> Coder 生成固定模板 summary.md -> Reviewer 单独校验摘要
                ├─ 不合格 -> 打回摘要（计轮次）
                └─ 合格   -> 落盘摘要 + 归档全部轮次 -> 子任务完成 -> 下一个子任务
  ↓
全部子任务完成 -> 全局评审【契约 + 所有摘要 + 全部模块 diff 汇总】
  ├─ 通过 -> 任务完成
  └─ 不通过 -> 问题归属到模块 -> 模块轮次重置为 1 -> 回到该模块增量评审 -> 再次全局评审
              （全局轮次耗尽 -> 人工介入）
```

### 1.1 运行中控制与人工介入（新增）

- **运行中改配置**：工作台顶部「⚙ 运行中配置」可随时修改：
  - 单个子任务最大轮次（立即同步到所有子任务）
  - 全局评审最大轮次
  - 是否开启人工审核（方案 + 每个模块设计）
- **方案审批**：开启人工审核后，技术总监拆解完会停在「方案审批」，左栏展示待审批模块，中栏展示技术栈/契约/模块验收标准；你可同意或填写意见驳回重拆。
- **逐模块设计审批**：每个模块 Coder 开工前，技术总监先生成详细设计文档；人工审核开启时必须逐一同意（或驳回重设计），通过后该模块才会派发 Coder。
- **随时终止**：顶部「终止执行」会立即停止 dsh（含工具子进程）并中止当前 LLM 请求；任务进入「已暂停，可恢复」，所有数据/记录保留，之后点「恢复调度」继续。
- **随时输入人工消息**：中栏底部输入框随时可发送，消息进入待注入队列，在下一个 Agent 步骤（拆解/设计/编码/评审/全局评审）注入，并实时显示。
- **实时执行输出**：右栏「实时执行输出」实时展示 LLM 请求/响应、dsh stdout（text/tool_call/tool_result/final）、stderr、git diff、模块设计、摘要文件、人工消息等；“展开全部”可查看完整历史。

> 已确认的业务约定（代码即按此实现）：
> 1. 新建任务可多选 Coder，但串行版**所有子任务固定交给可选列表中的第一个 Coder**。
> 2. 全局评审驳回后，被返工模块的 **current_round 重置为 1**，重新获得完整 `module_max_round` 预算。
> 3. 摘要校验的上下文 = 全局契约 + 当前子任务验收标准 + **已通过的代码 diff** + summary 内容。
> 4. 轮次语义：Coder 首次实现为第 1 轮；代码返工、摘要返工、人工驳回都会使轮次 +1；达到 `module_max_round` 仍不通过则标记人工介入。全局轮次按“全局评审次数”计。模块设计生成/人工审批不计入代码轮次。

---

## 2. 目录结构

```
AgentCrew/
├─ index.html
├─ package.json
├─ vite.config.ts
├─ tsconfig.json
├─ app-icon.png
├─ README.md
├─ docs/
│  └─ schema.sql                 # SQLite 建表 SQL（与 Rust 内 SCHEMA 一致）
├─ src/                          # Vue 3 前端
│  ├─ main.ts
│  ├─ App.vue                    # 四大 Tab 外壳
│  ├─ style.css                  # 全局深色 UI
│  ├─ types.ts                   # 与 Rust serde 对齐的类型
│  ├─ api.ts                     # 全部 Tauri invoke 封装
│  ├─ toast.ts
│  ├─ components/
│  │  ├─ NewTaskModal.vue
│  │  └─ SubtaskCard.vue         # 子任务卡片 + 历史轮次折叠
│  └─ views/
│     ├─ AgentsView.vue
│     ├─ TasksView.vue
│     ├─ WorkbenchView.vue       # 三栏工作台（核心）
│     └─ SettingsView.vue
└─ src-tauri/
   ├─ Cargo.toml
   ├─ build.rs
   ├─ tauri.conf.json
   ├─ capabilities/default.json
   ├─ icons/
   └─ src/
      ├─ main.rs
      ├─ lib.rs                 # Tauri 启动、DB 初始化、状态
      ├─ commands.rs            # 全部前端 command
      ├─ db.rs                  # SQLite CRUD / 表结构 / 归档
      ├─ models.rs              # 数据模型
      ├─ llm.rs                 # Reviewer/Planner 直连 LLM（强制 JSON）
      ├─ prompts.rs             # 全部 prompt 模板
      ├─ dsh.rs                 # dsh CLI 调用 + dsh patch + git tree diff
      └─ orchestrator.rs        # 串行编排状态机（核心）
```

---

## 3. 环境要求

- macOS / Windows / Linux（当前开发环境 macOS）
- Node.js ≥ 20
- Rust stable ≥ 1.77
- Xcode Command Line Tools（macOS）
- Git
- **Harness CLI `dsh`** 已安装并在 PATH 中（或稍后在设置里填绝对路径，例如 `/opt/homebrew/bin/dsh`）
- 可用的模型 API Key

首次安装：

```bash
cd AgentCrew
npm install
```

启动开发模式：

```bash
npm run tauri dev
```

构建安装包：

```bash
npm run tauri build -- --bundles app   # 推荐：只构建 .app，最快
# 或完整构建：npm run tauri build
```

`.app` 产物在 `src-tauri/target/release/bundle/macos/AgentCrew.app`。
macOS 上如果没有「自动化/屏幕录制」权限，DMG 打包脚本可能卡在 AppleScript；使用 `--bundles app` 可绕过。

> 前端单独构建：`npm run build`；Rust 检查：`cd src-tauri && cargo check`；端到端测试：`cd src-tauri && cargo test`。

> 如果新开终端找不到 `cargo`，先执行 `source "$HOME/.cargo/env"`。

---

## 4. 配置与使用

### 4.1 配置 Agent

进入 **Agent 配置**：

- 至少创建 1 个 `Reviewer`、1 个 `Coder`。
- 服务商/协议支持：
  - OpenAI 兼容（DeepSeek / OpenAI / 其他兼容网关）
  - Anthropic Messages
- `baseUrl` 填到版本前缀即可，例如：
  - `https://api.deepseek.com/v1`
  - `https://api.openai.com/v1`
  - `https://api.anthropic.com`
- `apiKey` 仅保存于本机 SQLite 明文（导入导出 JSON 也是明文，请妥善保管）。
- Reviewer 直连 HTTP；Coder 的配置会被写入临时 dsh provider patch。

### 4.2 新建任务

进入 **任务列表 -> 新建任务**：

- 任务名称
- 顶层业务需求（大输入框）
- 项目验收标准
- 选择 1 个 Reviewer（只展示 reviewer 角色）
- 多选 Coder（串行版固定使用第一个）
- 单个子任务最大迭代轮次
- 全局评审最大轮次
- 项目本地工作目录（会真实写代码，建议是空目录或已有项目）

启动后自动进入工作台。任务启动时会：

1. 自动创建目录（如不存在）；
2. 若还不是 git 仓库则 `git init`；
3. 用临时 git index 对工作目录做 tree 快照，作为 diff 基线（不改动用户真实 index）。

### 4.3 Workbench 操作

- 左侧点击子任务卡片查看；“历史”按钮展开该子任务**全部轮次归档**（评审意见 / diff / 结论）。
- 方案审批阶段左侧展示待审批模块；设计审批阶段中栏展示技术总监的模块详细设计。
- 中间顶部固定操作台：方案/设计审批、人工强制通过/驳回、全局人工兜底会根据状态自动切换，不会被长内容顶出视野。
- 中间底部是人工消息输入框（Ctrl+Enter 发送）与待注入消息列表。
- 右侧可折叠查看技术栈、全局契约、已完成模块摘要、实时执行输出、系统日志。
- 顶部「运行中配置」「终止执行」「恢复调度」可随时使用。
- 进入全局评审后：顶部标签切换，左侧队列冻结，中间切换全局评审会话视图。
- 人工兜底：
  - **人工强制标记通过**：代码评审阶段会继续生成摘要；摘要校验阶段则直接完成模块。
  - **人工驳回，手动输入修改意见**：把意见交给同一 Coder 重做，轮次 +1。
  - 全局轮次耗尽时，可“人工强制通过全局评审”或“人工指定模块返工”。
  - 应用重启/线程中断后，可点击“恢复调度”从 SQLite 持久化状态继续。

---

## 5. Harness CLI（dsh）对接说明

### 5.1 调用入口

每次 Coder 执行，应用调用：

```bash
dsh --profile headless \
    --patch <APP_DATA>/runtime/coder-<agentId>-patch.yml \
    --json -
```

- prompt 通过 **stdin** 传入（避免超长命令行参数）。
- 工作目录 = 任务选择的项目本地工作目录。
- stdout 是 NDJSON 事件流，应用从中读取 `{"type":"final","text":...}`。
- 执行前设置环境变量：
  - `DSH_AGENT_API_KEY=<该 Coder 的 apiKey>`
  - `DSH_PERMISSION_MODE=danger-full-access`（headless 无人值守，允许写文件/执行 bash；仅在你选择的项目目录内运行）
  - `DSH_PROFILE=headless`

### 5.2 按 Coder Agent 生成 dsh patch

应用会为每个 Coder 生成临时 patch，把该 Agent 的模型配置注入 dsh：

```yaml
- id: llm-pi-ai
  name: "@deepseek-ai/dsh-llm-pi-ai"
  config:
    providers:
      orchestrator-coder:
        displayName: "Coder 名称"
        apiKeyEnv: DSH_AGENT_API_KEY
        api: openai-completions        # anthropic 时自动切换 anthropic-messages
        baseURL: "https://api.deepseek.com/v1"
        models:
          - id: "deepseek-chat"
            name: "deepseek-chat"
            contextWindow: 200000
- id: agent-default-model
  name: "@deepseek-ai/dsh-agent-default-model"
  config:
    provider: orchestrator-coder
    model: "deepseek-chat"
```

> ⚠️ **temperature**：Reviewer 直连请求会使用配置值；dsh 当前不暴露 temperature 透传入口，因此 Coder 的 temperature 由 dsh/模型侧默认值决定。这是 Harness CLI 侧限制，不是业务漏实现。

### 5.3 diff 捕获

- 每轮 Coder 前后，用 **独立临时 git index**（`GIT_INDEX_FILE`）执行 `read-tree --empty + add -A + write-tree`，得到 tree hash。
- `git diff <beforeTree> <afterTree>` 得到本轮真实 diff（含新增文件），**不修改用户真实 index、不自动 commit、不做冲突合并**。
- 全局评审使用任务启动基线 tree 到当前 tree 的累计 diff。
- diff 传给 LLM 前会截断（90k / 140k 字符），避免上下文爆炸。

### 5.4 summary.md

- Coder 被要求写入：`<work_dir>/summaries/<NN>-<module-slug>.md`
- 固定模板（六段，必须完全一致）：

```markdown
# 模块名称
1. 对外接口列表：接口路径、请求方式、入参、返回字段
2. 关联数据库表
3. 核心业务逻辑简述
4. 对外依赖（依赖哪些其他模块能力）
5. 本模块对外提供的能力（哪些其他模块会调用我）
6. 已知限制、边界、未实现能力
```

- Reviewer 单独校验摘要并返回 JSON；通过后应用兜底落盘，写入 DB，归档全部轮次。
- 摘要只作为后续评审上下文，**历史完整对话/旧代码永远不会进入 Reviewer 上下文**。
- 应用会先做一次确定性模板校验（六个标题、长度、禁止代码围栏），再做 LLM 摘要准确性校验。

---

## 6. 数据存储

- SQLite 路径（macOS）：`~/Library/Application Support/com.local.agent-orchestrator/orchestrator.db`
- Windows：`%APPDATA%/com.local.agent-orchestrator/orchestrator.db`
- 应用显示名已改为 **AgentCrew**；bundle identifier 保留 `com.local.agent-orchestrator`，用于兼容升级前已有的本地数据库。
- 所有数据本地存储，不上云。
- 建表 SQL 见 [`docs/schema.sql`](docs/schema.sql)。

表：

| 表 | 用途 |
| --- | --- |
| `agent_config` | Agent 配置 |
| `task_main` | 任务主表（含技术栈、全局契约、阶段、状态、人工审核开关、待审批方案、git 基线 tree） |
| `subtask` | 子任务 / 模块（含详细设计、summary、全部轮次 JSON 归档） |
| `task_log` | 高层系统事件日志 |
| `task_event` | **全量执行输出**（LLM 请求/响应、dsh 流、diff 等，页面实时展示） |
| `human_message` | 人工随时输入的消息队列（pending / consumed） |
| `app_setting` | 极简设置键值 |

> 旧版本数据库启动时会自动执行 `ALTER TABLE` 补齐新列并创建新表，**不会删除或重建已有 Agent、任务、子任务数据**。

`iteration_record` 按需求内嵌在 `subtask.iteration_records` JSON 中，不单独分表。

---

## 7. LLM 结构化输出

- Planner / Reviewer / 全局评审全部要求 **只返回 JSON**。
- Coder 不解析自由文本：代码结果通过 git tree diff 获取，摘要通过固定文件路径读取，因此不存在“解析 Coder 回复失败”的问题。
- OpenAI 兼容请求带 `response_format: {"type":"json_object"}`；若服务端不支持会自动降级重试。
- 解析层会：
  - 提取 Markdown 围栏内 JSON；
  - 处理前后解释文字；
  - 首次解析失败自动发起一次“JSON 修复”请求；
  - 结构/语义不合法（如 `passed=false` 但无 feedback）会重试一次。
- 仍未通过则任务标记 `failed`，日志写明原因，可人工恢复。

主要 JSON 约定：

```json
// Planner
{ "global_contract": "markdown", "subtasks": [ { "name": "...", "accept_criteria": "..." } ] }

// 单次评审
{ "passed": true, "feedback": "..." }

// 全局评审
{ "passed": false, "feedback": "...", "issues": [
  { "subtask_id": 3, "subtask_name": "用户鉴权模块", "feedback": "..." }
] }
```

---

## 8. 常见问题

**Q: 点击开始任务后立即失败，日志提示启动 dsh 失败？**
GUI 应用的 PATH 通常不包含 Homebrew。到设置页把 `dsh 可执行文件` 填成绝对路径（`which dsh` 的结果，例如 `/opt/homebrew/bin/dsh`）。

**Q: Coder 报 401 / MISSING_CREDENTIAL？**
检查该 Coder Agent 的 apiKey / baseUrl / model；应用通过 `DSH_AGENT_API_KEY` 注入，dsh 自身的 DeepSeek 凭据不参与。

**Q: 我想让技术总监先给我审方案和设计怎么办？**
新建任务时勾选「开启人工审核模式」，或运行中在「⚙ 运行中配置」里打开。之后方案与每个模块设计都会等待你同意；关闭时会自动放行正在等待的方案/设计。

**Q: 执行过程看不到详细输出？**
右栏「实时执行输出」会展示 LLM 请求/响应和 dsh 的 text/tool_call/tool_result/final 事件；点「展开全部」可看完整历史。事件数量较多时加载会稍慢，属正常。

**Q: 拆解/评审报 `HTTP 404` 或 `HTTP 400`？**
优先检查 `model 名称` 是否填了服务商 API 的 **model ID**，而不是界面展示名。DeepSeek 官方当前支持 `deepseek-flash`、`deepseek-v4-pro`。错误日志现在会带实际请求 URL 和响应体，可据此检查 baseUrl 是否多写/少写 `/v1`。

**Q: Reviewer 报 JSON 解析失败？**
确认所用模型支持 JSON 输出；应用会自动重试一次并降级 `response_format`，仍失败请更换模型或降低 temperature。

**Q: 子任务不动了 / 应用重启后状态一直“执行中”？**
应用重启会把中断任务标记为 `failed` 并写日志；到工作台点击“恢复调度”。如果已到轮次上限，会显示“等待人工介入”，使用人工通过/驳回。

**Q: 工作目录必须是 git 仓库吗？**
不需要。应用会自动 `git init`。但建议已有项目保留自己的 `.git`；应用不会 commit、不会改你的真实 index。

**Q: 会并行开发多个模块吗？**
不会。简单版强制串行，一次只处理一个子任务。

---

## 9. 明确不做（需求硬约束）

- ❌ 自动 git 冲突合并
- ❌ MCP 插件市场 / 额外插件体系
- ❌ 内置代码编辑器 / 内置终端
- ❌ 复杂依赖拓扑编排（只有有序串行队列）
- ❌ 多用户 / 登录系统
- ❌ 向量 RAG / 自动摘要历史对话
- ❌ 并行开发多个子任务
- ❌ 任何云端存储

---

## 10. 手工验收清单

1. 创建 Reviewer + 2 个 Coder，刷新后仍存在。
2. 新建任务，选择 Reviewer / 多 Coder / 轮次 / 工作目录，启动。
3. 工作台左侧出现有序子任务；右侧契约已生成。
4. Coder 产生的代码真实出现在工作目录；中间出现 diff。
5. 评审不通过：轮次 +1，同一 Coder 收到修改意见并重做。
6. 代码通过：生成 summary.md；摘要校验通过后子任务 ✅，自动进入下一个。
7. 子任务卡片展开可看到每一轮评审意见 + diff + 结论。
8. 全部完成后自动进入全局评审；左侧冻结、中间切换视图。
9. 全局轮次耗尽显示人工介入；人工通过后任务完成。
10. Agent 配置导出再导入成功；设置保存后重启仍保留。
11. 运行中修改轮次上限，后续评审立即按新上限执行。
12. 开启人工审核：拆解后停在方案审批；同意后每个模块设计逐一等待审批，同意后才开始编码。
13. 终止执行后 dsh 立即停止，任务显示「已暂停，可恢复」，数据保留；恢复后可继续。
14. 输入人工消息后，下一个 Agent 步骤的 LLM 请求/事件流中能看到“人工消息已注入”。
15. 右栏实时执行输出持续刷新，包含 LLM 请求/响应、dsh 工具调用与 git diff。
