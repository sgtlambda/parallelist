import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

// The app's own frontend, built for a web page: every Tauri module it imports
// is swapped for a stand-in in `shims/`, and what the Rust side would answer
// is answered by `backend/`. Output goes into `site/demo`, which the landing
// page embeds.
const here = (path: string) => decodeURIComponent(new URL(path, import.meta.url).pathname)

export default defineConfig({
  root: here('.'),
  base: './',
  plugins: [vue()],
  resolve: {
    alias: {
      '@tauri-apps/api/core': here('./shims/core.ts'),
      '@tauri-apps/api/event': here('./shims/event.ts'),
      '@tauri-apps/api/path': here('./shims/path.ts'),
      '@tauri-apps/api/dpi': here('./shims/dpi.ts'),
      '@tauri-apps/api/menu': here('./shims/menu.ts'),
      '@tauri-apps/api/webview': here('./shims/webview.ts'),
      '@tauri-apps/plugin-dialog': here('./shims/dialog.ts'),
      '@tauri-apps/plugin-opener': here('./shims/opener.ts'),
    },
  },
  build: {
    outDir: here('../site/demo'),
    emptyOutDir: true,
  },
})
