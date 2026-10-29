/**
 * Dev-only vite config for the Board design captures (docs/m4/
 * capture-board.py): vite.config.ts (+ the `/kb` daemon proxy) with
 *  - mocked IPC (`window.__UNIPI_MOCK__(cmd, args)`, like
 *    vite.config.mock.ts) so several "paired PCs" exist in a browser;
 *  - per-PC board availability overrides
 *    (`window.__UNIPI_BOARD_AVAIL__[hostId]`) so offline /
 *    not-installed / error PCs can be shown. Unlisted PCs hit the real
 *    daemon through the proxy.
 * Never used by tests or production builds.
 */
import { defineConfig } from 'vitest/config';
import solid from 'vite-plugin-solid';
import tailwind from '@tailwindcss/vite';

const mocks = {
  name: 'unipi-board-capture-mocks',
  transform(code: string, id: string) {
    if (id.endsWith('/src/lib/ipc.ts')) {
      return code.replace(
        'await invoke(cmd, args)',
        'await ((typeof window !== "undefined" && window.__UNIPI_MOCK__) ? window.__UNIPI_MOCK__(cmd, args) : invoke(cmd, args))',
      );
    }
    if (id.endsWith('/src/lib/board/transport.ts')) {
      return code.replace(
        'async availability() {',
        'async availability(hostId) { const forced = typeof window !== "undefined" && window.__UNIPI_BOARD_AVAIL__ && window.__UNIPI_BOARD_AVAIL__[hostId]; if (forced) { await new Promise((r) => setTimeout(r, 300)); return forced; }',
      );
    }
    if (id.endsWith('/src/lib/blob.ts')) {
      // UNI-212 captures: `window.__UNIPI_BLOB__` stands in for the blob channel.
      return code.replace(/return override \?\? \(/, 'return override ?? (typeof window !== "undefined" && window.__UNIPI_BLOB__) ?? (');
    }
    return null;
  },
};

export default defineConfig({
  plugins: [solid(), tailwind(), mocks],
  clearScreen: false,
  server: {
    port: 1422,
    strictPort: true,
    host: '127.0.0.1',
    proxy: {
      '/kb': {
        target: process.env.UNIPI_KB_DEV_URL ?? 'http://127.0.0.1:7788',
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/kb/, ''),
        configure: (proxy) => proxy.on('proxyReq', (req) => req.removeHeader('origin')),
      },
    },
  },
});
