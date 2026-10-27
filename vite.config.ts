import { defineConfig, type Plugin } from "vite";
import vue from "@vitejs/plugin-vue";
import { fileURLToPath } from "node:url";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;
// run.sh picks a free port per run so several worktrees can run at once, and
// hands it to tauri's devUrl as well.
// @ts-expect-error process is a nodejs global
const port = Number(process.env.PARALLELIST_DEV_PORT) || 1420;

// `vite --mode sandbox` (pnpm sandbox) serves the component sandbox: the app's
// own components in a browser, with every Tauri module swapped for a stub in
// src/sandbox/tauri. None of this applies to `tauri dev` or `tauri build`.
const stubs: Record<string, string> = {
  "@tauri-apps/api/core": "core.ts",
  "@tauri-apps/api/event": "event.ts",
  "@tauri-apps/api/menu": "menu.ts",
  "@tauri-apps/api/dpi": "dpi.ts",
  "@tauri-apps/api/path": "path.ts",
  "@tauri-apps/api/webview": "webview.ts",
  "@tauri-apps/plugin-dialog": "dialog.ts",
  "@tauri-apps/plugin-opener": "opener.ts",
};

/** The sandbox is what `/` shows, rather than the app, which would run
 *  without a backend. */
const sandboxIndex: Plugin = {
  name: "sandbox-index",
  configureServer(server) {
    server.middlewares.use((req, _res, next) => {
      if (req.url === "/") req.url = "/sandbox.html";
      next();
    });
  },
};

// https://vite.dev/config/
export default defineConfig(async ({ mode }) => ({
  plugins: mode === "sandbox" ? [vue(), sandboxIndex] : [vue()],

  resolve: {
    alias:
      mode === "sandbox"
        ? Object.entries(stubs).map(([find, file]) => ({
            find,
            replacement: fileURLToPath(new URL(`./src/sandbox/tauri/${file}`, import.meta.url)),
          }))
        : [],
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a known port, fail if that port is not available
  server: {
    // The sandbox takes its own port, so it can run beside `tauri dev`.
    // @ts-expect-error process is a nodejs global
    port: mode === "sandbox" ? Number(process.env.PARALLELIST_SANDBOX_PORT) || 1430 : port,
    strictPort: mode !== "sandbox",
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
