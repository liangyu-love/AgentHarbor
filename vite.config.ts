import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "path";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [react()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  clearScreen: false,
  build: {
    rollupOptions: {
      output: {
        // Separate the stable React runtime for caching; keep feature libraries
        // with their lazy routes rather than eagerly loading all dependencies.
        manualChunks(id) {
          if (/node_modules\/(?:\.pnpm\/[^/]+\/node_modules\/)?(?:react|react-dom|scheduler)\//.test(id.replace(/\\/g, "/"))) {
            return "react-runtime";
          }
        },
      },
    },
  },
  server: {
    // Dev-only port — keep in sync with tauri.conf.json `devUrl`.
    port: 3417,
    strictPort: true,
    // Bind IPv4 loopback explicitly: on this machine `localhost` resolves to ::1
    // first and node binds IPv6-only, which WebView2 clients reaching 127.0.0.1
    // cannot connect to (blank window in `tauri dev`).
    host: host || "127.0.0.1",
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 3418,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
}));
