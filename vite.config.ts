import { readFileSync } from 'node:fs'
import { fileURLToPath, URL } from 'node:url'

import type { Plugin } from 'vite'
import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

/**
 * The development Content-Security-Policy, injected by `vite dev` and by nothing else.
 *
 * A release build's policy is `app.security.csp` in tauri.conf.json, which Tauri injects itself.
 * Development needs a looser `style-src` — Vite injects CSS as <style> elements — and that policy
 * used to sit in index.html, where it was shipped in every release build as a second meta tag:
 * harmless, since the stricter of two policies wins, but the weaker one was in the product. Read
 * from `devCsp`, so the two policies are kept side by side in one file.
 */
function devCsp(): Plugin {
  const conf = JSON.parse(
    readFileSync(new URL('./src-tauri/tauri.conf.json', import.meta.url), 'utf8'),
  ) as { app: { security: { devCsp: string } } }
  return {
    name: 'pipdock-dev-csp',
    apply: 'serve',
    transformIndexHtml: () => [
      {
        tag: 'meta',
        attrs: { 'http-equiv': 'Content-Security-Policy', content: conf.app.security.devCsp },
        // After Vite's own head-prepend scripts (React Refresh's inline preamble), as the old
        // meta in index.html was: a meta policy governs what follows it, and the preamble is
        // dev-only tooling, not the app.
        injectTo: 'head',
      },
    ],
  }
}

// The frontend lives in ui/ per ARCHITECTURE §2, but package.json sits at the repo root so the
// ops-repo `reusable-web-react.yml` — which has no working-directory input — can run `npm ci` and
// `npm run build` unmodified. Hence root: './ui' with the bundle emitted to the repo-root dist/,
// which is that workflow's default dist-dir.
export default defineConfig({
  root: './ui',
  plugins: [react(), tailwindcss(), devCsp()],
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
    // No source maps: Tauri embeds dist/ in the executable, so `sourcemap: true` shipped 2 MB of
    // them to every user, and nothing in a release build ever reads one (X51).
    sourcemap: false,
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
    // Off unless asked for (`npm run test:coverage`): sonar.yml imports the LCOV, and nothing
    // else reads it, so the plain `npm test` the ops-repo reusable runs stays as fast as it was.
    coverage: {
      provider: 'v8',
      // `lcovonly`: `lcov` adds an HTML report beside the file Sonar reads.
      reporter: ['text-summary', 'lcovonly'],
      reportsDirectory: 'coverage',
      include: ['ui/src/**/*.{ts,tsx}'],
      // Tests and their harness are not code under test. `generated.ts` is emitted by
      // `cargo run -p xtask -- bindings` and is types alone; `main.tsx` mounts <App/> into the
      // real DOM, which is what App.test.tsx renders instead.
      exclude: [
        'ui/src/**/*.test.{ts,tsx}',
        'ui/src/test/**',
        'ui/src/ipc/generated.ts',
        'ui/src/main.tsx',
      ],
    },
  },
})
