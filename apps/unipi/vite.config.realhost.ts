/**
 * Real-host QA (development only; scripts/qa/README.md): the app in a plain browser, talking to a
 * running `unipi-host` through scripts/qa/host-bridge.mjs. The boot module is injected only here,
 * so the normal app and production builds are untouched.
 *
 * `QA_VITE_PORT` (default 1451) picks the port; `QA_VITE_CACHE` the dependency cache (default a
 * temp dir: a checkout whose `node_modules` is a link to another tree must not write into it).
 */
import { defineConfig } from 'vite';
import solid from 'vite-plugin-solid';
import tailwind from '@tailwindcss/vite';
import { realpathSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('.', import.meta.url));

const realHost = {
  name: 'unipi-realhost-qa-bridge',
  enforce: 'pre' as const,
  transform(code: string, id: string) {
    if (!id.endsWith('/src/lib/host/factory.ts')) return null;
    return `import '/scripts/qa/realhost-boot.ts';\n${code}`;
  },
};

export default defineConfig({
  plugins: [solid({ hot: false }), tailwind(), realHost],
  clearScreen: false,
  cacheDir: process.env.QA_VITE_CACHE ?? `${tmpdir()}/unipi-realhost-vite`,
  // This checkout's own protocol sources, whatever `node_modules/@unipi/protocol` links to.
  resolve: { alias: { '@unipi/protocol': `${root}packages/protocol/src/index.ts` } },
  server: {
    port: Number(process.env.QA_VITE_PORT ?? 1451),
    strictPort: true,
    host: '127.0.0.1',
    watch: null,
    hmr: false,
    fs: { allow: [root, realpathSync(`${root}node_modules`)] },
  },
});
