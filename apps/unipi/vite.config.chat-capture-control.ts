/**
 * Dev-only vite config for the UNI-160 session-control-centre design
 * captures (docs/m6/capture-control.py): identical to
 * vite.config.chat-capture.ts but on port 1435, so it never collides with
 * the other capture servers (1432, 1433) if several happen to be up at
 * once. Never used by tests or production builds.
 */
import { defineConfig } from 'vitest/config';
import solid from 'vite-plugin-solid';
import tailwind from '@tailwindcss/vite';

const mocks = {
  name: 'unipi-chat-capture-control-mocks',
  transform(code: string, id: string) {
    if (id.endsWith('/src/lib/ipc.ts')) {
      return code.replace(
        'await invoke(cmd, args)',
        'await ((typeof window !== "undefined" && window.__UNIPI_MOCK__) ? window.__UNIPI_MOCK__(cmd, args) : invoke(cmd, args))',
      );
    }
    return null;
  },
};

export default defineConfig({
  plugins: [solid(), tailwind(), mocks],
  clearScreen: false,
  server: {
    port: 1435,
    strictPort: true,
    host: '127.0.0.1',
  },
});
