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
