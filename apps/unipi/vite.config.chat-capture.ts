/**
 * Dev-only vite config for the UNI-158 session-navigation design captures
 * (docs/m6/capture-nav.py): vite.config.ts + mocked IPC
 * (`window.__UNIPI_MOCK__(cmd, args)`, like vite.config.mock.ts /
 * vite.config.board-capture.ts) so the app renders without a real paired
 * PC. Port 1432 so it never collides with the Board capture server (1422).
 * Never used by tests or production builds.
 */
import { defineConfig } from 'vitest/config';
import solid from 'vite-plugin-solid';
import tailwind from '@tailwindcss/vite';

const mocks = {
  name: 'unipi-chat-capture-mocks',
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
    port: 1432,
    strictPort: true,
    host: '127.0.0.1',
  },
});
