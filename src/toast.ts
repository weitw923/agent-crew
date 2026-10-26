import { reactive } from "vue";

export interface ToastItem {
  id: number;
  message: string;
  kind: "info" | "success" | "error";
}

export const toasts = reactive<ToastItem[]>([]);
let seq = 1;

/** 极简全局 toast，避免引入 UI 组件库。 */
export function showToast(message: string, kind: ToastItem["kind"] = "info") {
  const id = seq++;
  toasts.push({ id, message, kind });
  window.setTimeout(() => {
    const idx = toasts.findIndex((t) => t.id === id);
    if (idx >= 0) toasts.splice(idx, 1);
  }, 4200);
}

export function errorMessage(e: unknown): string {
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "message" in e) {
    return String((e as { message: unknown }).message);
  }
  return String(e);
}
