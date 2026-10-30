/**
 * Dev-only vite config for the UNI-161 "needs you" design captures
 * (docs/m6/capture-needs.py): identical to vite.config.chat-capture-control.ts
 * but on port 1436, so it never collides with the other capture servers.
 * Never used by tests or production builds.
 */
import { defineConfig } from 'vitest/config';
import solid from 'vite-plugin-solid';
import tailwind from '@tailwindcss/vite';

const mocks = {
  name: 'unipi-chat-capture-needs-mocks',
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
    port: 1436,
    strictPort: true,
    host: '127.0.0.1',
  },
});
