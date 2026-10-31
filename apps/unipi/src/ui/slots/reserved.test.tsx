// @vitest-environment jsdom
/**
 * WP-U12 reserved routing (ui-protocol §8; app.md §3.7/§3.8/§3.10 acceptance):
 * every row of the reserved table, its live owner gate, the nav item/tab that
 * follows it, `openPanel` on a reserved id focusing the native view, and the
 * owner's fallback tree everywhere the native component is not shown (§8
 * rules 2 and 3).
 */
import { cleanup, render, screen, waitFor } from '@solidjs/testing-library';
import { createSignal, Show } from 'solid-js';
import { Route, Router, useNavigate } from '@solidjs/router';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createMockHost, type MockWorld } from '../../dev/mock/host';
import { setHostClientFactory } from '../../lib/host/factory';
import type { SessionSnapshot } from '../../lib/host/client';
import { resetLayout, setDesktopOverride } from '../../lib/layout';
import { launchableKinds, paneKind, setPaneComponent } from '../../lib/panes/kinds';
import { panes, resetPanes } from '../../lib/panes';
import { setPlatformOverride } from '../../lib/platform';
import { resetSessions, sessions, type SessionHandle } from '../../lib/session/store';
import { gitTarget, closeGit } from '../../lib/git';
import { railItems } from '../../components/shell/Rail';
import { sessionModal, setSessionModalTarget } from '../../components/session/modalTarget';
import roundtrip from '../../lib/session/fixtures/roundtrip.json';
import { resetLocal } from '../localState';
import { installUiSinks, resetUiRegistries, sessionUi } from '../registries';
import { resetSessionRenderCaches } from '../../lib/session/renderCache';
import { setUiTransport } from '../actions';
import { SlotModal, resetOverlayDismissals } from './SlotModal';
import { TurnFooters } from './TurnFooters';
import { reconcilePaneTabs, resetPaneClosures } from './paneTabs';
import {
  focusReservedEntry,
  installReservedRouting,
  isReservedPaneEntry,
  resetReservedHost,
} from './ReservedHost';
import { RESERVED_ROWS, reservedBodyFor, reservedGate, resetReservedGates, setHostTools, setOwnerGates } from './reserved';
import type { TimelineCtx } from '../../components/timeline/context';
import type { Turn } from '../../lib/session/turns';
import type { EntryView, UiEntry } from '../types';

const HOST = 'h1';
const SCOPE = `${HOST}/s1`;
const TARGET = { scope: 'session' as const, hostId: HOST, sessionId: 's1' };
const SCOPE_KEY = { hostId: HOST, projectKey: '~' };
const VIEW = { sessionId: 's1' };

const rt = roundtrip as unknown as { fresh: SessionSnapshot };
const baseSnapshot = (): SessionSnapshot => ({ ...rt.fresh, seq: 0, entries: [], ui: undefined });
const tree = (text: string): EntryView => ({ kind: 'tree', root: { t: 'Paragraph', text } });
const entry = (partial: Partial<UiEntry> & Pick<UiEntry, 'id' | 'slot' | 'view'>): UiEntry => ({
  extensionId: 'ext',
  scope: 'session',
  order: 0,
  seq: 1,
  rev: 1,
  ...partial,
});
const reserved = (id: string, component: string, slot: UiEntry['slot'], fallback?: string): UiEntry =>
  entry({
    id,
    slot,
    title: id,
    view: { kind: 'reserved', component, ...(fallback ? { fallback: { t: 'Paragraph', text: fallback } } : {}) },
  });

let world_: MockWorld;
let session: SessionHandle | undefined;
let sent: Array<{ entryId: string; actionId: string }> = [];
let undo: Array<() => void> = [];

beforeEach(() => {
  localStorage.clear();
  resetUiRegistries();
  resetLocal();
  resetPanes();
  resetSessionRenderCaches();
  resetOverlayDismissals();
  resetPaneClosures();
  resetReservedHost();
  resetLayout();
  closeGit();
  setSessionModalTarget(undefined);
  setDesktopOverride(true);
  setPlatformOverride('tauri-desktop');
  sent = [];
  installUiSinks();
  setUiTransport({
    action: (_t, p) => {
      sent.push({ entryId: p.entryId, actionId: p.actionId });
      return Promise.resolve({});
    },
    visible: () => Promise.resolve({}),
    resync: () => Promise.resolve({}),
  });
  // The native pane views the app ships (A20/A15); stand-ins keep the test light.
  undo = [setPaneComponent('files', () => null), setPaneComponent('git', () => null)];
  ownerState.clear();
  world_ = createMockHost({
    name: 'reserved',
    hosts: [
      {
        id: HOST,
        name: 'coffee',
        // The host's `_oino/extensions` answer: every reserved owner installed and on.
        settings: { extensions: RESERVED_ROWS.filter((r) => r.owner !== 'host').map((r) => ({ id: r.owner, enabled: true })) },
        sessions: { s1: { snapshot: baseSnapshot(), events: [] } },
      },
    ],
  });
  setHostClientFactory(world_.client);
});

afterEach(() => {
  undo.forEach((off) => off());
  undo = [];
  session = undefined;
  cleanup();
  resetSessions();
  setHostClientFactory(undefined);
  setDesktopOverride(undefined);
  setPlatformOverride(undefined);
});

// ── registry wire (same shape as the U10 slot tests) ─────────────────────

let seq = 0;
let rev = 0;
let entrySeq = 0;

const pushOps = (ops: unknown[]) =>
  world_.push(HOST, 's1', [{ session: 's1', seq: ++seq, event: { type: 'oino_ui_patch', scope: 'session', rev: ++rev, ops } }]);
const put = (e: UiEntry) => pushOps([{ op: 'put', entry: { ...e, seq: ++entrySeq } }]);

async function open(): Promise<SessionHandle> {
  const s = sessions().ensure(HOST, 's1');
  session = s;
  await waitFor(() => expect(s.state.conn).toBe('live'));
  rev = sessionUi(HOST, 's1').state.rev;
  seq = 0;
  entrySeq = 0;
  return s;
}

/** The host's `_oino/extensions` answer, merged per owner (one `host_cmd` per fetch). */
const ownerState = new Map<string, boolean>();
const owner = (id: string, enabled: boolean) => {
  ownerState.set(id, enabled);
  setOwnerGates(HOST, [...ownerState].map(([ownerId, on]) => ({ id: ownerId, enabled: on })));
};
const allOwnersOn = () => {
  for (const row of RESERVED_ROWS) if (row.owner !== 'host') ownerState.set(row.owner, true);
  setOwnerGates(HOST, [...ownerState].map(([id, enabled]) => ({ id, enabled })));
};

// ── §8 rule 2: entry present ⇔ gate true, per row ─────────────────────────

describe('U12 reserved gates (§8 rule 2)', () => {
  it('every row follows its owner live: enabled → on, disabled → off', () => {
    // The rows this build renders natively; browser/terminal are the fallback
    // rows (rule 3) and are covered separately.
    const native = RESERVED_ROWS.filter((r) => ['board', 'cron', 'files', 'git', 'sessionInfo', 'lhStatus', 'turnFiles'].includes(r.component));
    for (const row of RESERVED_ROWS.filter((r) => r.owner !== 'host')) {
      allOwnersOn();
      owner(row.owner, false);
      expect(reservedGate(row), `${row.component} is off when ${row.owner} is disabled`).toBe(false);
      allOwnersOn();
      if (native.includes(row)) expect(reservedGate(row), `${row.component} is on when ${row.owner} is enabled`).toBe(true);
    }
  });

  it('an owner that is installed nowhere shows no surface; with no host list the platform decides', () => {
    resetReservedGates();
    setPlatformOverride('tauri-desktop');
    expect(reservedGate('unipi.kanboard/board')).toBe(true); // no host asked yet
    setOwnerGates(HOST, [{ id: 'unipi.cron', enabled: true }]);
    expect(reservedGate('unipi.kanboard/board')).toBe(false); // asked: kanboard is not installed
    expect(reservedGate('unipi.cron/jobs')).toBe(true);
  });

  it('host rows follow the host service the app talks to (fs/git/pty)', () => {
    resetReservedGates();
    expect(reservedGate('host/files')).toBe(true);
    expect(reservedGate('host/git')).toBe(true);
    expect(reservedGate('host/terminal')).toBe(true);
    setPlatformOverride('web');
    expect(reservedGate('host/files')).toBe(false);
    expect(reservedGate('host/git')).toBe(false);
    expect(reservedGate('host/terminal')).toBe(false);
    expect(reservedGate('unipi.kanboard/board')).toBe(true); // the web build is the board
  });

  it('the Board row also needs `unipi-kanboard` on the host (`host_info.tools`)', () => {
    allOwnersOn();
    setHostTools(HOST, { kanboard: false });
    expect(reservedGate('unipi.kanboard/board')).toBe(false);
    setHostTools(HOST, { kanboard: true });
    expect(reservedGate('unipi.kanboard/board')).toBe(true);
  });

  it('a row whose pane view is not in this build falls back instead of routing', () => {
    allOwnersOn();
    // Browser (B8) and terminal (A29) register their pane component later; until
    // then the entry is not routed and its owner tree renders (§8 rule 3).
    expect(paneKind('browser')?.component).toBeUndefined();
    expect(paneKind('terminal')?.component).toBeUndefined();
    expect(isReservedPaneEntry(reserved('unipi.browser/browser', 'browser', 'sidePane', 'browser fallback'))).toBe(false);
    expect(isReservedPaneEntry(reserved('host/terminal', 'terminal', 'bottomPane', 'terminal fallback'))).toBe(false);
    // The turn footer renders in place: A27 registered its native body, so the
    // row is routed (the component itself hides on a turn without files).
    expect(reservedGate('unipi.file-summary/turn-files')).toBe(true);
  });
});

// ── nav items + the + launcher follow the gate ───────────────────────────

describe('U12 nav items and tabs follow the owner', () => {
  it('Board and Cron rail items disappear when their owner is disabled, live', () => {
    allOwnersOn();
    expect(railItems().map((i) => i.id)).toEqual(expect.arrayContaining(['board', 'cron']));
    owner('unipi.kanboard', false);
    expect(railItems().map((i) => i.id)).not.toContain('board');
    expect(railItems().map((i) => i.id)).toContain('cron');
    owner('unipi.cron', false);
    expect(railItems().map((i) => i.id)).not.toContain('cron');
  });

  it('the + launcher offers Files/Git only where the host services exist', () => {
    setPlatformOverride('tauri-desktop');
    expect(launchableKinds('side', true).map((k) => k.kind)).toEqual(expect.arrayContaining(['files', 'git']));
    setPlatformOverride('web');
    expect(launchableKinds('side', true).map((k) => k.kind)).not.toContain('files');
  });
});

// ── openPanel on a reserved id focuses the native view ───────────────────

describe('U12 openPanel focuses the native view', () => {
  it('a `host/files` sidePane entry opens the native Files tab, not an ext tab', async () => {
    allOwnersOn();
    await open();
    const reg = sessionUi(HOST, 's1');
    render(() => {
      reconcilePaneTabs({ hostId: HOST, sessionId: 's1', cwd: () => undefined, registry: () => reg });
      return <p>host</p>;
    });
    put(reserved('host/files', 'files', 'sidePane', 'fallback files'));
    await waitFor(() => expect(panes().visible(SCOPE_KEY, 'side', VIEW).some((t) => t.kind === 'files')).toBe(true));
    expect(panes().visible(SCOPE_KEY, 'side', VIEW).some((t) => t.kind === 'ext')).toBe(false);
  });

  it('`host/git` focuses the native Git view at its target (no ext tab)', () => {
    allOwnersOn();
    const nav: string[] = [];
    expect(focusReservedEntry(reserved('host/git', 'git', 'sidePane', 'fallback'), { hostId: HOST, cwd: '/w', navigate: (h) => nav.push(h) })).toBe(true);
    expect(gitTarget()).toMatchObject({ hostId: HOST, path: '/w' });
    // once per entry instance: a registry update never re-focuses
    expect(focusReservedEntry(reserved('host/git', 'git', 'sidePane'), { hostId: HOST, cwd: '/w', navigate: (h) => nav.push(h) })).toBe(false);
  });

  it('`unipi.long-horizon/visualize` opens the status pane at the LH section', () => {
    allOwnersOn();
    expect(focusReservedEntry(reserved('unipi.long-horizon/visualize', 'lhStatus', 'sidePane'), { hostId: HOST, cwd: '/w', navigate: () => undefined })).toBe(true);
    expect(gitTarget()).toMatchObject({ hostId: HOST, path: '/w' });
  });

  it('`unipi.kanboard/board` navigates to the Board page', () => {
    allOwnersOn();
    const nav: string[] = [];
    expect(focusReservedEntry(reserved('unipi.kanboard/board', 'board', 'fullscreen'), { hostId: HOST, navigate: (h) => nav.push(h) })).toBe(true);
    expect(nav).toEqual(['/board']);
  });

  it('`unipi.info-screen/dashboard` opens the Session modal on Info', () => {
    allOwnersOn();
    expect(focusReservedEntry(reserved('unipi.info-screen/dashboard', 'sessionInfo', 'overlay'), { hostId: HOST, sessionId: 's1', navigate: () => undefined })).toBe(true);
    expect(sessionModal()).toMatchObject({ hostId: HOST, sessionId: 's1', tab: 'info', entryId: 'unipi.info-screen/dashboard' });
  });

  it('phone: `host/files` opens the full-screen pane layer with the native body', () => {
    setDesktopOverride(false);
    setPlatformOverride('tauri-mobile');
    allOwnersOn();
    const nav: string[] = [];
    const layers: UiEntry[] = [];
    const entryIn = reserved('host/files', 'files', 'sidePane');
    expect(focusReservedEntry(entryIn, { hostId: HOST, cwd: '/w', navigate: (h) => nav.push(h), openLayer: (e) => layers.push(e) })).toBe(true);
    expect(nav).toEqual([]); // §13.6: a layer over the conversation, never a route change
    expect(layers.map((e) => e.id)).toEqual(['host/files']);
    // the layer draws the native Files tree, not the owner's fallback tree
    expect(reservedBodyFor(entryIn)).toBeTruthy();
  });

  it('a row the build does not implement is left to its fallback tree', () => {
    allOwnersOn();
    const browserRow = reserved('unipi.browser/browser', 'browser', 'sidePane', 'browser fallback');
    expect(isReservedPaneEntry(browserRow)).toBe(false);
    expect(focusReservedEntry(browserRow, { hostId: HOST, navigate: () => undefined })).toBe(false);
  });

  it('a reserved page row opened with `focus` navigates once (openPanel)', async () => {
    allOwnersOn();
    await open();
    const [path, setPath] = createSignal('/s/h1/s1');
    const nav: string[] = [];
    render(() => {
      installReservedRouting({ navigate: (h) => nav.push(h), pathname: path });
      return <p>shell</p>;
    });
    put({ ...reserved('unipi.kanboard/board', 'board', 'sidePane'), focus: true });
    await waitFor(() => expect(nav).toEqual(['/board']));
    put({ ...reserved('unipi.kanboard/board', 'board', 'sidePane'), focus: true, seq: 9 });
    await waitFor(() => expect(railItems().some((i) => i.id === 'board')).toBe(true));
    setPath('/board');
    await new Promise((r) => setTimeout(r, 20));
    expect(nav).toEqual(['/board']); // already there / same instance: no second jump
  });
});

// ── §8 rule 3: fallback trees everywhere else ───────────────────────────

describe('U12 fallback trees elsewhere (§8 rule 3)', () => {
  it('the Session modal is the only overlay for its reserved entry (no second modal)', () => {
    allOwnersOn();
    render(() => <SlotModal stack={[reserved('unipi.info-screen/dashboard', 'sessionInfo', 'overlay', 'dashboard fallback')]} target={TARGET} />);
    expect(screen.queryByTestId('ui-modal')).toBeNull();
  });

  it('a reserved entry in a foreign slot renders the owner tree', () => {
    allOwnersOn();
    render(() => <SlotModal stack={[reserved('host/git', 'git', 'overlay', 'git fallback rows')]} target={TARGET} />);
    expect(screen.getByText('git fallback rows')).toBeTruthy();
  });

  it('the reserved turn-footer row renders natively (never pulls a renderer) and hides on a turn without files', async () => {
    allOwnersOn();
    const s = await open();
    const ctx = { session: s, hostId: HOST, sessionId: () => 's1', columnWidth: () => 800 } as unknown as TimelineCtx;
    const turn: Turn = { turnId: 'e-user-1', kind: 'turn', entries: [], segments: [], live: false, stopped: false, source: 'user' };
    put(reserved('unipi.file-summary/turn-files', 'turnFiles', 'turnFooter', '3 files +12 −04'));
    render(() => <TurnFooters ctx={ctx} turn={turn} />);
    // A27's native body asks the extension for the turn (the mock answers `{}`):
    // no owner fallback, no renderer, and an empty summary renders nothing at all.
    await waitFor(() =>
      expect(world_.log.some((e) => e.kind === 'cmd' && e.op === '_oino/service_call' && (e.args as { name?: string })?.name === 'turn')).toBe(true),
    );
    expect(screen.queryByText('3 files +12 −04')).toBeNull();
    expect(screen.queryByTestId('turn-files')).toBeNull();
    expect(world_.log.some((e) => e.kind === 'cmd' && e.op === '_oino/render' && (e.args as { target?: string })?.target === 'turnFooter')).toBe(false);
  });

  it('a reserved turn-footer row with its owner off renders the owner fallback (rule 3)', async () => {
    allOwnersOn();
    owner('unipi.file-summary', false);
    const s = await open();
    const ctx = { session: s, hostId: HOST, sessionId: () => 's1', columnWidth: () => 800 } as unknown as TimelineCtx;
    const turn: Turn = { turnId: 'e-user-1', kind: 'turn', entries: [], segments: [], live: false, stopped: false, source: 'user' };
    put(reserved('unipi.file-summary/turn-files', 'turnFiles', 'turnFooter', '3 files +12 −04'));
    render(() => <TurnFooters ctx={ctx} turn={turn} />);
    await waitFor(() => expect(screen.getByText('3 files +12 −04')).toBeTruthy());
    expect(screen.queryByTestId('turn-files')).toBeNull();
  });
});

// ── §8 rule 2: a disabled owner closes its open page ─────────────────────

describe('U12 disabled owner closes its page', () => {
  const Harness = (props: { path: string; nav: string[] }) => {
    const navigate = useNavigate();
    installReservedRouting({ navigate: (h) => { props.nav.push(h); navigate(h); }, pathname: () => props.path });
    return <p>on {props.path}</p>;
  };

  it('leaving /board when `unipi.kanboard` is disabled falls back to the chat', async () => {
    allOwnersOn();
    const nav: string[] = [];
    render(() => (
      <Router>
        <Route path="*" component={() => <Harness path="/board" nav={nav} />} />
      </Router>
    ));
    await new Promise((r) => setTimeout(r, 20));
    expect(nav).toEqual([]);
    owner('unipi.kanboard', false);
    await waitFor(() => expect(nav.length).toBeGreaterThan(0));
    expect(nav[0]).not.toBe('/board');
  });
});

// keep the imports honest for the Show/Router helpers used by the shell test
void Show;
void SCOPE;
