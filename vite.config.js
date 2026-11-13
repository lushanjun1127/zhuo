import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    host: "0.0.0.0",
    port: 3000,
    strictPort: true,
    hmr: {
      port: 3000,
    },
    proxy: {
      // 后端 zhuo-server（axum，SSE 流式对话 + 记忆库）
      "/api": {
        target: "http://127.0.0.1:1127",
        changeOrigin: true,
      },
    },
  },
});
