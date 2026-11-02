/**
 * UI renderer gallery (ui-protocol §13.7, WP-U9): every node fixture of
 * `crates/oino-ui/fixtures/nodes/*.json` rendered by the app's `NodeView`,
 * plus the perf pages, on port 1442:
 *
 *   npx vite --config vite.config.ui-capture.ts
 *   http://127.0.0.1:1442/__ui-gallery?shell=desktop          (all kinds)
 *   http://127.0.0.1:1442/__ui-gallery?shell=phone&kind=List  (one kind)
 *   http://127.0.0.1:1442/__ui-gallery?page=perf              (§13.5 rows)
 *
 * Screenshots: `docs/dev/v2/design/shots/app/u9-*.png` (desktop 1280×820,
 * phone 412×915). No host, no sockets: actions are logged on
 * `window.__UI_ACTIONS__`. Never used by tests or production builds.
 */
import { defineConfig } from 'vitest/config';
import solid from 'vite-plugin-solid';
import tailwind from '@tailwindcss/vite';
import { fileURLToPath } from 'node:url';

const fixtures = fileURLToPath(new URL('../../crates/oino-ui/fixtures', import.meta.url));
const here = fileURLToPath(new URL('.', import.meta.url));

/**
 * Served from here, not from an `.html` file in the tree: other vite configs
 * scan every `.html` under the root for dependency pre-bundling.
 */
const GALLERY_HTML = `<!doctype html><html lang="en"><head><meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<meta name="theme-color" content="#0A0908"><title>Unipi · UI renderer gallery</title>
<style>html,body{background:#0A0908;margin:0}</style></head>
<body><div id="root"></div><script type="module" src="/src/ui/gallery/main.tsx"></script></body></html>`;

type Req = { url?: string };
type Res = { setHeader(k: string, v: string): void; end(body: string): void };
type Server = {
  middlewares: { use(fn: (req: Req, res: Res, next: () => void) => void): void };
  transformIndexHtml(url: string, html: string): Promise<string>;
};

const galleryRoute = {
  name: 'unipi-ui-gallery-route',
  configureServer(server: Server) {
    server.middlewares.use((req, res, next) => {
      if (!req.url?.startsWith('/__ui-gallery')) return next();
      void server.transformIndexHtml(req.url, GALLERY_HTML).then((html) => {
        res.setHeader('Content-Type', 'text/html');
        res.end(html);
      }, next);
    });
  },
};

export default defineConfig({
  plugins: [solid(), tailwind(), galleryRoute],
  clearScreen: false,
  server: { port: 1442, strictPort: true, host: '127.0.0.1', fs: { allow: [here, fixtures] } },
  optimizeDeps: { entries: ['src/ui/gallery/main.tsx'] },
});
