/**
 * Capture harness (app spec §3.15, WP-A1): the real app on port 1450 with the
 * mock host (`src/dev/mock/`) answering every host op when the URL carries
 * `scenario=`:
 *
 *   npx vite --config vite.config.capture.ts
 *   http://127.0.0.1:1450/#/chat?scenario=empty&shell=desktop
 *
 * A plugin prepends `import '/src/dev/mock/boot.ts'` to `lib/host/factory.ts`,
 * so the scenario's world is on `window.__UNIPI_HOST__` before the first
 * request. Requests/commands are recorded on `window.__UNIPI_MOCK_LOG__`.
 * Screenshots go to `docs/dev/v2/design/shots/app/<wp>-<name>-<shell>.png`.
 * `QA_VITE_CACHE` (optional) moves Vite's dependency cache: a checkout whose `node_modules` is a
 * link to another tree would otherwise write the cache into that tree.
 * Never used by tests or production builds.
 */
import { defineConfig } from 'vitest/config';
import solid from 'vite-plugin-solid';
import tailwind from '@tailwindcss/vite';

const mockHost = {
  name: 'unipi-capture-mock-host',
  enforce: 'pre' as const,
  transform(code: string, id: string) {
    if (!id.endsWith('/src/lib/host/factory.ts')) return null;
    return `import '/src/dev/mock/boot.ts';\n${code}`;
  },
};

export default defineConfig({
  plugins: [solid(), tailwind(), mockHost],
  clearScreen: false,
  ...(process.env.QA_VITE_CACHE ? { cacheDir: process.env.QA_VITE_CACHE } : {}),
  server: { port: Number(process.env.QA_PORT ?? 1450), strictPort: true, host: '127.0.0.1', watch: { ignored: ['**/target/**', '**/dist/**'] } },
});
