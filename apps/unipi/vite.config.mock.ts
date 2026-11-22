/**
 * Dev-only vite config for the §7 design captures: identical to
 * vite.config.ts but with a plugin that lets a page-level
 * `window.__UNIPI_MOCK__(cmd, args)` answer IPC commands, so the real
 * app runs in a plain browser with realistic mocked data. Never used
 * by tests or production builds.
 */
import { defineConfig } from 'vitest/config';
import solid from 'vite-plugin-solid';
import tailwind from '@tailwindcss/vite';

const mockIpc = {
  name: 'unipi-mock-ipc',
  transform(code: string, id: string) {
    if (!id.endsWith('/src/lib/ipc.ts')) return null;
    return code.replace(
      'await invoke(cmd, args)',
      'await ((typeof window !== "undefined" && window.__UNIPI_MOCK__) ? window.__UNIPI_MOCK__(cmd, args) : invoke(cmd, args))',
    );
  },
};

export default defineConfig({
  plugins: [solid(), tailwind(), mockIpc],
  clearScreen: false,
  server: { port: 1421, strictPort: true, host: '127.0.0.1' },
});
