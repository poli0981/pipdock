import { fileURLToPath, URL } from 'node:url'

import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

// The frontend lives in ui/ per ARCHITECTURE §2, but package.json sits at the repo root so the
// ops-repo `reusable-web-react.yml` — which has no working-directory input — can run `npm ci` and
// `npm run build` unmodified. Hence root: './ui' with the bundle emitted to the repo-root dist/,
// which is that workflow's default dist-dir.
export default defineConfig({
  root: './ui',
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./ui/src', import.meta.url)),
    },
  },
  build: {
    outDir: '../dist',
    emptyOutDir: true,
    // Tauri ships its own WebView2; there is no legacy browser to support.
    target: 'esnext',
    sourcemap: true,
  },
  server: {
    port: 1420,
    // Fail loudly rather than silently moving ports — tauri.conf.json pins devUrl to 1420.
    strictPort: true,
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: [fileURLToPath(new URL('./ui/src/test/setup.ts', import.meta.url))],
    include: ['ui/src/**/*.test.{ts,tsx}'],
    root: fileURLToPath(new URL('.', import.meta.url)),
  },
})
