/** OIN-38 capture: replay the updater's UI registry entry and persist its skip action. */
import { SCENARIOS } from './scenarios';
import { genCards, genFsTree, genSession } from './gen';

const id = 's001';
const skipped = () => localStorage.getItem('oin38.skip') === '1';
const session = genSession(id, { seed: 9, turns: 1, toolsPerTurn: 0, cwd: '/home/you/projects/unipi' });
const root = {
  t: 'Layout', direction: 'Vertical', spacing: 1, children: [
    { t: 'Paragraph', text: 'Oino 1.0.2 is available (installed 1.0.1).' },
    { t: 'Paragraph', text: 'Installed     1.0.1' },
    { t: 'Paragraph', text: 'Latest          1.0.2' },
    { t: 'Paragraph', text: 'Channel       latest' },
    { t: 'Paragraph', text: "What's new" },
    { t: 'Paragraph', text: 'No changelog for this release.' },
    { t: 'Layout', direction: 'Horizontal', spacing: 1, children: [
      { t: 'Button', label: 'How to update', action: 'how', variant: 'Primary' },
      { t: 'Button', label: 'Release page', action: 'open' },
      { t: 'Button', label: 'Skip this version', action: 'skip' },
      { t: 'Button', label: 'Later', action: 'close' },
    ] },
  ],
};
const entry = { id: 'unipi.updater/update', extensionId: 'unipi.updater', scope: 'session', slot: 'overlay', order: 0, seq: 1, rev: 1, title: 'Update available', view: { kind: 'tree', root } };
SCENARIOS['oin38'] = {
  name: 'oin38',
  appOps: {
    app_info: () => ({ version: '1.0.2', platform: 'linux', proto: '1' }),
    about_components: () => ({ app: '1.0.2', host: { version: '1.0.2' }, oino: { version: '1.0.2' }, extensions: { count: 1 }, problems: [] }),
  },
  hosts: [{ id: 'pc-coffee', name: 'coffee', conn: { state: 'connected' }, cards: genCards(1, { seed: 5 }), fs: genFsTree(), sessions: {
    [id]: { ...session, events: skipped() ? [] : [{ at: 200, dto: { session: id, seq: 1, event: { type: 'oino_ui_patch', scope: 'session', rev: 1, ops: [{ op: 'put', entry }] } } }],
      cmd: { '_oino/ui_action': (cmd) => { if (cmd.actionId === 'skip') { localStorage.setItem('oin38.skip', '1'); location.reload(); } return {}; } } },
  } }],
  setup: () => {
    localStorage.setItem('unipi.shell.last', JSON.stringify({ hostId: 'pc-coffee', sessionId: id }));
    (window as any).__UNIPI_LOCAL_ADMIN__ = { call: async (req: { op: string }) => req.op === 'host_info' ? { version: '1.0.1', proto: '1', mode: 'app', pid: 1, started_at: 1, host_id: 'pc-coffee', host_name: 'coffee', data_dir: '/home/you/.oino', tools: { oino: true, unipi_kanboard: true, browser_engines: [], cloak: false }, devices: 0 } : { devices: [] } };
  },
};
