/**
 * Shell scenarios (WP-A3): This computer + a paired PC with sessions in a
 * few projects; layout / pane state seeded through `setup`.
 *
 *   shell-desktop        sidebar + conversation (capture 1440×900, 1280×820)
 *   shell-narrow         side pane wanted open: overlay below 1280 px (1000×800)
 *   shell-sidepane-tabs  side pane with tabs (Git active) + the bottom panel
 *   shell-fullview       the same tabs; press Ctrl+Shift+F for full view
 *
 * Files / Git / Tree views are other WPs (A10, A15, A20): `setup` plugs in
 * **capture stand-ins** so the container can be judged with real-looking
 * content. Never part of the app.
 */
import { For } from 'solid-js';
import { FileText, Folder } from 'lucide-solid';
import type { MockScenario } from '../host';
import { genCards, genFsTree, genSession } from '../gen';
import { setPaneComponent } from '../../../lib/panes/kinds';

const LOCAL = 'this-pc';
const CWD = '/home/you/projects/unipi';

const cards = genCards(9, { seed: 3 });
const base = genSession('s001', { seed: 4, turns: 4, toolsPerTurn: 2, thinking: true, cwd: CWD });

const ports = () => ({
  ports: [
    { port: 5173, bind: '127.0.0.1', process: 'vite', http: true, allowed: true },
    { port: 1450, bind: '127.0.0.1', process: 'node', http: true, allowed: true },
    { port: 5432, bind: '127.0.0.1', process: 'postgres', http: false, allowed: false },
  ],
  any: true,
  allowed: [],
});

function hosts(): MockScenario['hosts'] {
  return [
    { id: LOCAL, name: 'workstation', transport: 'local', conn: { state: 'connected' }, cards, fs: genFsTree(CWD), sessions: { s001: base }, ops: { ports_list: ports } },
    { id: 'pc-coffee', name: 'coffee', conn: { state: 'connected' }, cards: genCards(3, { seed: 9, root: '/home/me/work' }).map((c) => ({ ...c, session_id: `c${c.session_id}` })) },
  ];
}

const row = (icon: 'dir' | 'file', name: string, depth = 0, hint?: string) => ({ icon, name, depth, hint });

function StandIn(props: { rows: ReturnType<typeof row>[] }) {
  return (
    <div style={{ padding: '10px 8px', 'font-size': '13px' }}>
      <For each={props.rows}>
        {(r) => (
          <div style={{ display: 'flex', 'align-items': 'center', gap: '8px', height: '30px', 'padding-left': `${10 + r.depth * 16}px`, color: 'var(--color-text-muted)' }}>
            {r.icon === 'dir' ? <Folder size={15} /> : <FileText size={15} />}
            <span style={{ flex: 1, color: 'var(--color-text)' }}>{r.name}</span>
            {r.hint ? <span style={{ 'font-family': 'var(--font-mono)', 'font-size': '12px', color: 'var(--color-wm-green)' }}>{r.hint}</span> : null}
          </div>
        )}
      </For>
    </div>
  );
}

function plugStandIns(): void {
  setPaneComponent('files', () => <StandIn rows={[row('dir', 'src'), row('file', 'math.ts', 1), row('file', 'cli.ts', 1), row('file', 'README.md'), row('file', 'package.json')]} />);
  setPaneComponent('git', () => <StandIn rows={[row('file', 'src/math.ts', 0, '+12 −3'), row('file', 'src/cli.ts', 0, '+40'), row('file', 'README.md', 0, '+2 −1')]} />);
  setPaneComponent('tree', () => <StandIn rows={[row('file', 'Fix the failing test'), row('file', 'Run the suite', 1), row('file', 'Steer: also check cli', 1)]} />);
}

function seed(layout: Record<string, unknown>, tabs?: unknown): void {
  localStorage.setItem('unipi.shell.layout', JSON.stringify({ sidebarOpen: true, sidebarW: 264, sideOpen: false, sideW: 480, bottomOpen: false, bottomH: 220, ...layout }));
  localStorage.setItem('unipi.shell.last', JSON.stringify({ hostId: LOCAL, sessionId: 's001' }));
  if (tabs) localStorage.setItem(`unipi.panes:${LOCAL}|${CWD}`, JSON.stringify(tabs));
}

const TABS = {
  side: {
    tabs: [
      { id: 'tf', kind: 'files' },
      { id: 'tg', kind: 'git' },
      { id: 'tt', kind: 'tree', sessionId: 's001' },
      { id: 'tp', kind: 'forward', params: { hostId: LOCAL, port: 5173, local: true } },
    ],
    active: { s001: 'tg' },
    closed: [{ id: 'tb', kind: 'btw', sessionId: 's001', title: 'Why is the build slow?' }],
  },
  bottom: { tabs: [], active: {}, closed: [] },
};

export const shellDesktop: MockScenario = { name: 'shell-desktop', hosts: hosts(), setup: () => seed({}) };
export const shellNarrow: MockScenario = { name: 'shell-narrow', hosts: hosts(), setup: () => (plugStandIns(), seed({ sideOpen: true }, TABS)) };
export const shellSidepaneTabs: MockScenario = { name: 'shell-sidepane-tabs', hosts: hosts(), setup: () => (plugStandIns(), seed({ sideOpen: true, bottomOpen: true }, TABS)) };
export const shellFullview: MockScenario = { name: 'shell-fullview', hosts: hosts(), setup: () => (plugStandIns(), seed({ sideOpen: true }, TABS)) };
