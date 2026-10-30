/**
 * `empty-chat` (WP-A28, Z-87): the new-chat empty state on the draft route
 * (`/new/<host>?cwd=…`). A paired PC named `coffee` and a folder, so the
 * greeting, the capability hint (which names the host) and the lore line are
 * all in the frame. The state never lists recents — the sidebar is the one
 * place to resume something.
 */
import type { MockScenario } from '../host';
import { genCards, genFsTree, genSession } from '../gen';

const CWD = '/home/me/work/unipi';
const WORK = '/home/you/projects/unipi';

export const emptyChat: MockScenario = {
  name: 'empty-chat',
  hosts: [
    {
      id: 'pc-coffee',
      name: 'coffee',
      conn: { state: 'connected' },
      cards: genCards(4, { seed: 5, root: '/home/me/work' }),
      fs: genFsTree(CWD),
    },
  ],
  setup: () => {
    localStorage.setItem(
      'unipi.shell.layout',
      JSON.stringify({ sidebarOpen: true, sidebarW: 264, sideOpen: false, sideW: 480, bottomOpen: false, bottomH: 220 }),
    );
  },
};

/**
 * `motion-exits` (WP-A28, Z-91): the exit frames for the capture harness. A
 * conversation with a settled turn whose work fold is open; `setup` expands
 * the fold and slows the shared exit to 1.5 s (`--exit-ms`, `lib/motion.ts`)
 * so three timed screenshots can catch the same exit mid-flight. The same
 * slowdown shows the header menu's popover exit.
 */
export const motionExits: MockScenario = {
  name: 'motion-exits',
  hosts: [
    {
      id: 'this-pc',
      name: 'workstation',
      transport: 'local',
      conn: { state: 'connected' },
      cards: genCards(6, { seed: 8, root: '/home/you/projects/unipi' }),
      sessions: { s001: genSession('s001', { seed: 4, turns: 3, toolsPerTurn: 2, thinking: true, cwd: WORK }) },
      fs: genFsTree(WORK),
    },
  ],
  setup: () => {
    document.documentElement.style.setProperty('--exit-ms', '1500ms');
    localStorage.setItem(
      'unipi.shell.layout',
      JSON.stringify({ sidebarOpen: true, sidebarW: 264, sideOpen: false, sideW: 480, bottomOpen: false, bottomH: 220 }),
    );
    // Expand the first work fold once the timeline has painted.
    const tick = (left: number) => {
      const head = document.querySelector<HTMLElement>('[data-testid="tl-fold"] .proc-head');
      if (head) {
        head.click();
        return;
      }
      if (left > 0) setTimeout(() => tick(left - 1), 100);
    };
    setTimeout(() => tick(60), 200);
  },
};
