/**
 * Dev-only vite config for the UNI-159 composer design captures
 * (docs/m6/capture-composer.py): identical to vite.config.chat-capture.ts
 * but on port 1433, so it never collides with the nav capture server
 * (1432) if both happen to be up at once. Never used by tests or
 * production builds.
 */
import { defineConfig } from 'vitest/config';
import solid from 'vite-plugin-solid';
import tailwind from '@tailwindcss/vite';

const mocks = {
  name: 'unipi-chat-capture-composer-mocks',
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
    port: 1433,
    strictPort: true,
    host: '127.0.0.1',
  },
});
