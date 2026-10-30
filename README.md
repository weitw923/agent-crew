# AgentCrew


AgentCrew 是一个**本地优先**的多 Agent 代码开发编排桌面应用（Tauri 2 + Vue 3 + Rust + SQLite），当前版本 **v2.5.0**。

## 背景

在本地项目里让 AI 连续写一整个功能时，常见问题是：需求没人拆、改了哪几个文件没人管、每轮改完没人验收，最后只能自己回头逐个 diff 看。

AgentCrew 把这些环节串成一条**固定顺序的流水线**，全程在你自己机器上跑：

```
需求拆解 → 技术栈 / 全局契约 / 子任务清单 → 逐模块设计 → 编码 → 代码评审 → 摘要校验 → 下一个模块 → 全局评审
```

- **多个 Agent 分工**：每个 Agent 绑定一个角色类型（内置「评审者」「编码者」，可自行新增「产品经理」等任意角色），角色用能力标签描述职责，新建任务时拖拽成一条线性工作流。
- **代码真的写进你的项目**：编码者（Coder）通过 Harness CLI（`dsh`）在指定工作目录里读写文件、执行命令；应用用独立临时 git index 捕获每轮 diff，不改你的真实 index、不自动 commit。
- **评审不只是聊天**：评审者拿到「契约 + 已完成模块摘要 + 当前子任务 + 本轮 diff」，返回结构化 JSON 结论；不通过就打回同一个 Coder 重做，直到轮次上限才转为人工介入。
- **人工可随时接管**：支持方案 / 逐模块设计审批、暂停与恢复、运行中修改需求、强制通过 / 驳回、追加轮次；任务完成后还可以「追加需求」，只对增量拆解，不重跑已完成模块。
- **数据不出本机**：任务、日志、模块设计 / diff / 摘要快照都存在本地 SQLite 与本地目录（默认 `~/Library/Application Support/com.local.agent-orchestrator/`，可在设置里改），没有任何云端存储。

## 现成安装包

[软件安装包下载链接](https://gitee.com/weitw/agent-crew/releases/)

- DMG 未做 Apple 签名 / 公证，首次打开如果提示「无法验证开发者」，在 **系统设置 → 隐私与安全性** 里点「仍要打开」，或终端执行 `xattr -dr com.apple.quarantine /Applications/AgentCrew.app`。
- 目前没有 Windows / Linux 的现成包。应用本身基于 Tauri，在对应平台按下面的源码步骤执行 `npm run tauri build` 即可打包出 `.msi` / `.exe` / `.deb` / `.AppImage`，是否顺畅运行尚未在真实机器验证。

## 依赖

**使用现成包**：macOS Apple Silicon、Git、Harness CLI `dsh`（`dsh` 需先在 PATH 中，或稍后在设置里填绝对路径）、可用的模型 API Key。

**从源码构建**：

| 依赖 | 版本 / 说明 |
| --- | --- |
| Node.js | ≥ 20，附 npm |
| Rust | stable ≥ 1.77（含 `cargo`） |
| Xcode Command Line Tools | macOS 必需（编译、打包） |
| 系统 WebView | macOS 自带；Linux 需 `webkit2gtk` 等 Tauri 系统依赖 |
| npm 依赖 | Vue 3、Vite 6、TypeScript、markdown-it、`@tauri-apps/api`、`@tauri-apps/cli`，见 [`package.json`](package.json) |
| Rust 依赖 | `tauri` 2、`rusqlite`（内置 SQLite）、`reqwest`、`serde`、`chrono`，见 [`src-tauri/Cargo.toml`](src-tauri/Cargo.toml) |
| Harness CLI `dsh` | 运行任务时必需，用于驱动编码 Agent |
| 模型 API | OpenAI 兼容（DeepSeek / OpenAI / 兼容网关）或 Anthropic Messages，需要 API Key |
| Git | 用于捕获每轮 diff |

## 从源码运行

```bash
cd AgentCrew
npm install
npm run tauri dev          # 开发模式启动
```

打包：

```bash
npm run tauri build -- --bundles app   # 只构建 .app（最快）
npm run tauri build                    # 完整构建，含 DMG / 安装包
```

产物位于 `src-tauri/target/release/bundle/`。首次拉代码后不需要额外准备数据库，应用启动时会自动建表并把旧版本数据迁移到最新结构。

若终端提示找不到 `cargo`，先执行 `source "$HOME/.cargo/env"`。

## 首次使用要做的事

安装或启动后，还需要自己完成两件事，应用才能跑任务：

1. **Agent 配置**：至少建一个具备「方案设计 / 评审验收」能力的评审者、一个具备「编码实现」能力的编码者，填好服务商、`baseUrl`（如 `https://api.deepseek.com/v1`）、`model`（填服务商的 model ID）、`apiKey`。
2. **设置**：若应用问不到 `dsh`，把「dsh 可执行文件」填写成绝对路径（`which dsh` 的结果，例如 `/opt/homebrew/bin/dsh`）。

然后在「任务列表 → 新建任务」里填需求、验收标准、工作流、轮次上限和工作目录（会真实写代码，建议是空目录或已有项目）即可。API Key 只以明文保存在本机 SQLite，请不要把导出的 Agent 配置 JSON 或装有数据库的目录推到公开仓库。

## 文档

完整功能说明、配置细节、dsh 对接、SQLite 表结构、diff 捕获机制与手工验收清单见 [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md)；建表 SQL 见 [docs/schema.sql](docs/schema.sql)。
