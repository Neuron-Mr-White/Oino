/**
 * The browser beside the chat follows the chat's agent profile.
 *
 *   browser-profile        the PC answers `session_profile: 'ace-jobs'` (PC default: `default`)
 *   browser-profile-old    an older PC: no `session_profile`, today's per-PC choice
 *
 * Desktop: the side pane opens on the Browser tab. Phone: open ⋯ → Browser.
 */
import type { MockScenario } from '../host';
import { genCards, genSession } from '../gen';
import { createMockBrowser } from '../browser';

const LOCAL = 'this-pc';
const CWD = '/home/you/projects/unipi';

function scenario(name: string, agentProfile?: string): MockScenario {
  return {
    name,
    hosts: [{ id: LOCAL, name: 'workstation', transport: 'local', conn: { state: 'connected' }, cards: genCards(9, { seed: 3 }), sessions: { s001: genSession('s001', { seed: 4, turns: 3, toolsPerTurn: 2, cwd: CWD }) } }],
    setup: () => {
      (window as unknown as { __UNIPI_BROWSER__?: unknown }).__UNIPI_BROWSER__ = createMockBrowser({ agentProfile });
      localStorage.setItem('unipi.shell.layout', JSON.stringify({ sidebarOpen: true, sidebarW: 264, sideOpen: true, sideW: 520, bottomOpen: false, bottomH: 220 }));
      localStorage.setItem('unipi.shell.last', JSON.stringify({ hostId: LOCAL, sessionId: 's001' }));
      localStorage.setItem(`unipi.panes:${LOCAL}|${CWD}`, JSON.stringify({ side: { tabs: [{ id: 'tb', kind: 'browser', params: { hostId: LOCAL } }], active: { s001: 'tb' }, closed: [] }, bottom: { tabs: [], active: {}, closed: [] } }));
    },
  };
}

export const browserProfile = scenario('browser-profile', 'ace-jobs');
export const browserProfileOld = scenario('browser-profile-old');
