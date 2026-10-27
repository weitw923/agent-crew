// 仅用于本地 UI 回归（浅色/深色主题、三栏布局）的浏览器 Mock 入口，不参与 Tauri 打包。
const params = new URLSearchParams(location.search);
const theme = params.get("theme");
if (theme) localStorage.setItem("agentcrew-theme", theme);

const { installMock } = await import("./tauriMock");
installMock();

await import("../style.css");
const [{ createApp }, { default: App }] = await Promise.all([
  import("vue"),
  import("../App.vue")
]);
createApp(App).mount("#app");

// 通过 URL 参数驱动截图场景：?scene=tasks|iterations|workbench|config&theme=light
const scene = params.get("scene");

function clickByText(selector: string, text: string) {
  const nodes = Array.from(document.querySelectorAll<HTMLElement>(selector));
  const target = nodes.find((n) => (n.textContent ?? "").includes(text));
  target?.click();
  return !!target;
}

async function runScene() {
  await new Promise((r) => setTimeout(r, 600));
  if (scene === "agents" || scene === "settings") {
    clickByText(".tab-btn", scene === "agents" ? "Agent 配置" : "设置");
    await new Promise((r) => setTimeout(r, 700));
    return;
  }
  if (scene === "newtask") {
    clickByText(".page-head .btn", "新建任务");
    await new Promise((r) => setTimeout(r, 500));
    return;
  }
  if (!scene || scene === "tasks") return;
  clickByText(".task-card .btn", "打开详情");
  await new Promise((r) => setTimeout(r, 800));
  if (scene === "iterations") return;
  clickByText(".iteration-card .btn", "进入迭代");
  await new Promise((r) => setTimeout(r, 800));
  if (scene === "workbench") return;
  if (scene === "completed") {
    // 已完成任务的「追加需求，继续迭代」固定面板
    return;
  }
  if (scene === "design") {
    clickByText(".tree-subtask-actions .link-btn", "设计");
    await new Promise((r) => setTimeout(r, 400));
    return;
  }
  if (scene === "st-view") {
    clickByText(".tree-subtask-actions .link-btn", "查看");
    await new Promise((r) => setTimeout(r, 400));
    return;
  }
  if (scene === "plan") {
    clickByText(".iteration-card .btn", "进入迭代");
    await new Promise((r) => setTimeout(r, 600));
    return;
  }
  if (scene === "md-tech") {
    clickByText(".contract-bar .chip", "技术栈");
    await new Promise((r) => setTimeout(r, 400));
    return;
  }
  if (scene === "search" || scene === "collapse" || scene === "jump") {
    await new Promise((r) => setTimeout(r, 400));
    if (scene === "search") {
      const input = document.querySelector<HTMLInputElement>(".timeline-search input");
      if (input) {
        input.value = "导出";
        input.dispatchEvent(new Event("input", { bubbles: true }));
      }
      await new Promise((r) => setTimeout(r, 400));
      return;
    }
    if (scene === "collapse") {
      clickByText(".conv-toolbar .chip", "折叠全部");
      await new Promise((r) => setTimeout(r, 400));
      return;
    }
    if (scene === "jump") {
      // 切到有轮次记录的 dev-1.0 迭代，再跳到第 1 轮评审
      clickByText(".iteration-node-head .link-btn", "切换");
      await new Promise((r) => setTimeout(r, 1200));
      clickByText(".round-chip", "第1轮");
      await new Promise((r) => setTimeout(r, 1200));
      return;
    }
  }
  if (scene === "history") {
    // 展开历史迭代 dev-1.0，再点其中带轮次记录的模块「历史」
    const nodes = Array.from(document.querySelectorAll<HTMLElement>(".iteration-node"));
    const node = nodes.find((n) => (n.textContent ?? "").includes("dev-1.0"));
    clickByText(".iteration-node-head", "dev-1.0");
    await new Promise((r) => setTimeout(r, 300));
    void node;
    const btn = Array.from(
      node?.querySelectorAll<HTMLElement>(".tree-subtask-actions .link-btn") ?? []
    ).find((b) => (b.textContent ?? "").includes("历史") && !b.hasAttribute("disabled"));
    btn?.click();
    await new Promise((r) => setTimeout(r, 700));
    return;
  }
  if (scene === "tools") {
    clickByText(".conv-toolbar .chip", "工具调用");
    await new Promise((r) => setTimeout(r, 400));
    clickByText(".timeline-meta .link-btn", "在右侧查看详情");
    await new Promise((r) => setTimeout(r, 400));
    return;
  }
  if (scene === "config") {
    clickByText(".workbench-top .btn", "配置");
    await new Promise((r) => setTimeout(r, 300));
  }
}

void runScene();
export {};
