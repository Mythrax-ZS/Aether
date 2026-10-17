/// <reference types="vitest/config" />
import { fileURLToPath, URL } from 'node:url';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vite';

// Tauri expects a fixed dev port, and the app is only ever shown inside WebView2 (Chromium).
export default defineConfig({
  plugins: [
    svelte({
      // Runes everywhere in our code; leave third-party components alone.
      dynamicCompileOptions({ filename }) {
        if (!filename.includes('node_modules')) return { runes: true };
      },
    }),
  ],
  resolve: {
    alias: { $lib: fileURLToPath(new URL('./src/lib', import.meta.url)) },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: '127.0.0.1',
    watch: { ignored: ['**/src-tauri/**', '**/target/**', '**/.scratch/**'] },
  },
  build: {
    target: 'es2022',
    chunkSizeWarningLimit: 2000,
  },
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts'],
  },
});
