import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

// Tauri 固定前端开发端口，strictPort 保证 dsh webview 能稳定连上。
export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"]
    }
  }
});
