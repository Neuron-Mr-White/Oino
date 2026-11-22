/**
 * Reserved-id routing scenarios (WP-U12, ui-protocol §8): the app renders its
 * own components for the reserved ids, so a capture proves the *routing*, not
 * a node tree.
 *
 *   u12-native      a host-owned `host/git` entry is open → the native Git pane
 *                   tab is focused (no `ext` tab) and the rail shows Board +
 *                   Cron while their owners are enabled
 *   u12-fallback    a `host/terminal` entry with no terminal component yet →
 *                   its owner `fallback` tree fills the bottom panel (§8 rule 3)
 *   u12-owner-off   the same session with `unipi.kanboard`/`unipi.cron` disabled
 *                   on the host → the nav items are gone, live (§8 rule 2)
 *   u12-phone       `host/files` opened with `focus` on the phone → the Files
 *                   layer (§3.9/§13.6) with the native Files tree
 *   u12-nav         the phone conversation with every owner enabled: the bottom
 *                   nav keeps its Board tab
 *   u12-phone-off   the same on a host whose `unipi.kanboard`/`unipi.cron` are
 *                   disabled: the bottom nav loses its Board tab, live
 */
import type { MockScenario, MockSession, MockStep } from '../host';
import type { SessionSnapshot } from '../../../lib/host/client';
import { genCards, genFsTree, genSession } from '../gen';

type SnapshotUi = NonNullable<SessionSnapshot['ui']>;

const HOST = 'pc-coffee';
const CWD = '/home/you/projects/unipi';
const cards = genCards(6, { seed: 5 });

/** The host's `_oino/extensions` answer: every one of them, `off` ones disabled. */
const extensions = (off: string[]) =>
  ['unipi.kanboard', 'unipi.cron', 'unipi.info-screen', 'unipi.long-horizon', 'unipi.browser', 'unipi.file-summary'].map((id) => ({
    id,
    enabled: !off.includes(id),
  }));

/** A reserved entry (§8); `component` is the wire name of the native component. */
function reserved(id: string, component: string, slot: string, title: string, icon: string, extra: Record<string, unknown> = {}) {
  return {
    id,
    extensionId: 'host',
    scope: 'host',
    slot,
    order: 0,
    seq: 1,
    rev: 1,
    title,
    icon,
    view: { kind: 'reserved', component, ...((extra.fallback as object | undefined) ? { fallback: extra.fallback } : {}) },
    ...(extra.focus ? { focus: true } : {}),
  };
}

/** The board/terminal fallbacks the owner installs (`updatePanel{fallback}`). */
const TERMINAL_FALLBACK = {
  t: 'Log',
  lines: [
    '$ unipi-host --status',
    'host   pc-coffee · proto 2.0.0 · pid 4112',
    'term   1 session (shell) · 0 clients',
    'note   the terminal tab arrives with A29; this is the owner fallback',
  ],
  maxLines: 200,
};

function ui(entries: ReturnType<typeof reserved>[]): SnapshotUi {
  return { scope: 'session', sessionId: 's001', rev: 1, dialogs: [], meta: {}, entries: entries as SnapshotUi['entries'] };
}

function session(uiBlock: SnapshotUi, events: MockStep[] = []): MockSession {
  const base = genSession('s001', { seed: 9, turns: 4, toolsPerTurn: 2, thinking: true, cwd: CWD });
  return { ...base, snapshot: { ...base.snapshot, ui: uiBlock }, events };
}

function variant(name: string, opts: { entries: ReturnType<typeof reserved>[]; off?: string[]; layout?: Record<string, unknown> }): MockScenario {
  return {
    name,
    hosts: [
      {
        id: HOST,
        name: 'coffee',
        transport: 'local',
        conn: { state: 'connected' },
        cards,
        fs: genFsTree(CWD),
        settings: { extensions: extensions(opts.off ?? []) },
        sessions: { s001: session(ui(opts.entries)) },
      },
    ],
    speed: 1,
    setup: () => {
      localStorage.setItem(
        'unipi.shell.layout',
        JSON.stringify({ sidebarOpen: true, sidebarW: 264, sideOpen: false, sideW: 460, bottomOpen: false, bottomH: 220, ...opts.layout }),
      );
      localStorage.setItem('unipi.shell.last', JSON.stringify({ hostId: HOST, sessionId: 's001' }));
    },
  };
}

/** The native Git pane focused by a `host/git` reserved entry. */
export const u12Native = variant('u12-native', {
  entries: [reserved('host/git', 'git', 'sidePane', 'Git', 'branch', { focus: true })],
  layout: { sideOpen: true },
});

/** The `host/terminal` owner fallback (the terminal component is wave 3). */
export const u12Fallback = variant('u12-fallback', {
  entries: [reserved('host/terminal', 'terminal', 'bottomPane', 'Terminal', 'terminal', { fallback: TERMINAL_FALLBACK })],
  layout: { bottomOpen: true },
});

/** The owners switched off on the host: the rail items go with them. */
export const u12OwnerOff = variant('u12-owner-off', {
  entries: [reserved('host/terminal', 'terminal', 'bottomPane', 'Terminal', 'terminal', { fallback: TERMINAL_FALLBACK })],
  off: ['unipi.kanboard', 'unipi.cron'],
  layout: { bottomOpen: true },
});

/** Phone: `host/files` opened with focus → the Files layer. */
export const u12Phone = variant('u12-phone', {
  entries: [reserved('host/files', 'files', 'sidePane', 'Files', 'folder', { focus: true })],
});

/** Phone conversation, every owner enabled (the bottom nav keeps Board). */
export const u12Nav = variant('u12-nav', { entries: [] });

/** Phone, Board owner off: no Board tab in the bottom nav (§8 rule 2). */
export const u12PhoneOff = variant('u12-phone-off', { entries: [], off: ['unipi.kanboard', 'unipi.cron'] });
