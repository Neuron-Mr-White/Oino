import { defineConfig, type Plugin } from 'vitest/config';
import solid from 'vite-plugin-solid';
import tailwind from '@tailwindcss/vite';
import { fileURLToPath } from 'node:url';
import { readFileSync } from 'node:fs';

/**
 * `npm run build:web` (`vite build --mode web`): the browser build the
 * kanboard daemon embeds and serves at `/` (DESIGN.md §5.7 "Web build").
 * Only the Board works in a browser, so the app-only screens/layers are
 * swapped for an empty stub (their routes show the "open the UniPi app"
 * notice through `<Requires>` anyway) — that keeps xterm, CodeMirror,
 * pdf.js and the chat stack out of the binary. Absolute `/assets/…` URLs
 * (base `/`): the daemon serves the SPA shell for every client path.
 */
const WEB_STUBS: Record<string, string> = {
  './screens/Sessions': 'AppOnly.tsx',
  './screens/Terminal': 'AppOnly.tsx',
  './screens/Chat': 'AppOnly.tsx',
  './screens/Files': 'AppOnly.tsx',
  './screens/Editor': 'AppOnly.tsx',
  './components/chat/ChatView': 'AppOnly.tsx',
  './components/chat/ReviewViewer': 'AppOnly.tsx',
  './components/media/Previewer': 'AppOnly.tsx',
  '@xterm/xterm/css/xterm.css': 'empty.css',
};

export function webStubs(): Plugin {
  const dir = fileURLToPath(new URL('./src/web/', import.meta.url));
  return {
    name: 'unipi-web-stubs',
    enforce: 'pre',
    resolveId(source, importer) {
      if (!importer || !/[\\/]src[\\/]main\.tsx$/.test(importer)) return null;
      const stub = WEB_STUBS[source];
      return stub ? dir + stub : null;
    },
    // A stamp the kanboard build reads (which app version it embeds).
    generateBundle() {
      let version: string | null = null;
      try {
        version = (JSON.parse(readFileSync(new URL('./src-tauri/tauri.conf.json', import.meta.url), 'utf8')) as { version?: string }).version ?? null;
      } catch {
        /* no version */
      }
      this.emitFile({ type: 'asset', fileName: 'unipi-web.json', source: `${JSON.stringify({ app: 'unipi', kind: 'web', version }, null, 2)}\n` });
    },
    // The terminal's Nerd Font symbols face (1.2 MB) is only used by xterm.
    transform(code, id) {
      if (!/[\\/]src[\\/]style\.css$/.test(id)) return null;
      return code.replace(/@font-face\s*{[^}]*Symbols Nerd Font Mono[^}]*}/, '');
    },
  };
}

export default defineConfig(({ mode }) => ({
  plugins: [...(mode === 'web' ? [webStubs()] : []), solid(), tailwind()],
  base: mode === 'web' ? '/' : undefined,
  // The app build has a second page: the desktop mascot window (UNI-200, mascot.html).
  build: mode === 'web' ? { outDir: 'dist-web', emptyOutDir: true, sourcemap: false } : { rollupOptions: { input: { main: fileURLToPath(new URL('./index.html', import.meta.url)), mascot: fileURLToPath(new URL('./mascot.html', import.meta.url)) } } },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    proxy: {
      '/kb': {
        target: process.env.UNIPI_KB_DEV_URL ?? 'http://127.0.0.1:7788',
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/kb/, ''),
        configure: (proxy) => proxy.on('proxyReq', (req) => req.removeHeader('origin')),
      },
    },
  },
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test-setup.ts'],
    // The app's own tests only — `packages/protocol` (a workspace member)
    // runs its suite in its own package context (`npm test --workspaces`).
    include: ['src/**/*.test.{ts,tsx}'],
  },
}));
