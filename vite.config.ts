import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [ vue() ],
  clearScreen: false,
  build: {
    // preserve the webview targets used before the Vite 8 upgrade
    target: [ 'chrome107', 'edge107', 'firefox104', 'safari16' ]
  },
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
        protocol: 'ws',
        host,
        port: 1421
      }
      : undefined,
    watch: {
      ignored: [ '**/src-tauri/**' ]
    }
  }
});
