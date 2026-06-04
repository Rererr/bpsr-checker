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
