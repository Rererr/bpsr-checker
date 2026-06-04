import { defineConfig } from "vite";
import solidPlugin from "vite-plugin-solid";
import path from "path";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [solidPlugin()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? { protocol: "ws", host, port: 1421 }
      : undefined,
    // Rust のビルド成果物を vite の監視対象から除外（target 内 DLL ロックによる EBUSY 回避）
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    rollupOptions: {
      input: {
        main: path.resolve(__dirname, "index.html"),
        buffs: path.resolve(__dirname, "buffs.html"),
        self_status: path.resolve(__dirname, "self_status.html"),
      },
    },
  },
});
