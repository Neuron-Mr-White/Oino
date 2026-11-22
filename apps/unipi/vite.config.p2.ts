/** Reproducible P2 idle request comparison against the dev mock.
 * P2_BASELINE=1 uses pre-P2 versions of the migrated frontend consumers.
 * Run: npx vite --config vite.config.p2.ts --port 1492
 */
import { defineConfig } from 'vitest/config';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import solid from 'vite-plugin-solid';
import tailwind from '@tailwindcss/vite';

const root = fileURLToPath(new URL('../../', import.meta.url));
const consumers = new Set([
  'src/lib/session/hub.ts', 'src/lib/composer/draftSync.ts',
  'src/components/panes/tabs/BgTaskTab.tsx', 'src/components/panes/tabs/SubagentTab.tsx',
  'src/components/superagent/FleetDetail.tsx', 'src/components/superagent/railStatus.ts',
  'src/components/superagent/state.ts', 'src/components/files/FileTree.tsx',
]);
export default defineConfig({
  cacheDir: `target/vite-p2-${process.env.P2_BASELINE === '1' ? 'before' : 'after'}`,
  optimizeDeps: { noDiscovery: true, include: [] },
  plugins: [
    { name: 'p2-baseline', enforce: 'pre', transform(code, id) {
      if (process.env.P2_BASELINE !== '1') return;
      const name = id.split('/apps/unipi/')[1];
      if (consumers.has(name)) return execFileSync('git', ['show', `48b66565c473f3e8eb4baf5cf244334f7ee17652:apps/unipi/${name}`], { cwd: root, encoding: 'utf8' });
    } },
    { name: 'p2-mock', enforce: 'pre', transform(code, id) {
      if (id.endsWith('/src/lib/host/factory.ts')) return `import '/src/dev/mock/boot.ts';\n${code}`;
    } },
    solid(), tailwind(),
  ],
  resolve: { alias: { '@unipi/protocol': fileURLToPath(new URL('./packages/protocol/src/index.ts', import.meta.url)) } },
  server: { hmr: false, host: '127.0.0.1', port: 1492, strictPort: true, watch: { ignored: ['**/target/**', '**/dist/**'] } },
});
