import { ref } from "vue";

export type Theme = "dark" | "light";

const STORAGE_KEY = "agentcrew-theme";

function initialTheme(): Theme {
  const saved = localStorage.getItem(STORAGE_KEY);
  if (saved === "light" || saved === "dark") return saved;
  return window.matchMedia?.("(prefers-color-scheme: light)").matches ? "light" : "dark";
}

export const theme = ref<Theme>(initialTheme());

export function applyTheme(value: Theme) {
  document.documentElement.dataset.theme = value;
  document.documentElement.style.colorScheme = value;
}

export function setTheme(value: Theme) {
  theme.value = value;
  localStorage.setItem(STORAGE_KEY, value);
  applyTheme(value);
}

export function toggleTheme() {
  setTheme(theme.value === "dark" ? "light" : "dark");
}

export function initTheme() {
  applyTheme(theme.value);
}

// 模块加载即应用，减少启动时的主题闪烁。
if (typeof document !== "undefined") {
  applyTheme(theme.value);
}
