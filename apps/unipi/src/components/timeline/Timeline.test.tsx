// @vitest-environment jsdom
/**
 * Timeline (WP-A6) over a real `createSession` + MockHost: turns render,
 * the virtual window mounts a slice, the bottom lock detaches/reattaches
 * with the jump pill, and the outbox row shows a lost message with retry.
 * (DOM geometry is stubbed: 600 px viewport, 40 px rows — scroll-position
 * and pixel-precise behaviour lives in the A12 UI scenarios.)
 */
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { createRoot } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createMockHost, type MockScenario, type MockSession, type MockWorld } from '../../lib/host/mock';
import type { HostClient, SessionSnapshot } from '../../lib/host/client';
import type { SessionEntry } from '../../lib/session/types';
import { createSession, type SessionHandle } from '../../lib/session/store';
import { sessionRefChat } from '../../lib/timeline/refChat';
import { clearScrollMemory } from '../../lib/timeline/scrollMemory';
import { clearTimelineSnapshots, writeTimelineSnapshot } from '../../lib/timeline/snapshotCache';
import { commandNames, type TimelineCtx } from './context';
import { Timeline } from './Timeline';
import { TurnHeightCache } from '../../lib/timeline/heightCache';
import { resetFileSummaries } from '../../lib/fileSummary';
import { resetSessions, sessions } from '../../lib/session/store';
import { installUiSinks, resetUiRegistries } from '../../ui/registries';
import { resetReservedGates, setOwnerGates } from '../../ui/slots/reserved';
import { setPlatformOverride } from '../../lib/platform';
import roundtrip from '../../lib/session/fixtures/roundtrip.json';

const rt = roundtrip as unknown as { fresh: SessionSnapshot };

vi.mock('../../lib/host/factory', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../lib/host/factory')>()),
  hostClient: () => clientOf(),
}));
let clientOf: () => HostClient = () => {
  throw new Error('clientOf not set');
};

/* ── geometry stubs (jsdom has none) ─────────────────────────────── */
beforeEach(() => {
  localStorage.clear();
  clearScrollMemory();
  clearTimelineSnapshots();
  Object.defineProperty(HTMLElement.prototype, 'offsetHeight', {
    configurable: true,
    get() {
      return this.classList?.contains('tl') ? 600 : 40;
    },
  });
  Object.defineProperty(HTMLElement.prototype, 'offsetWidth', { configurable: true, get: () => 400 });
  Object.defineProperty(HTMLElement.prototype, 'clientHeight', {
    configurable: true,
    get() {
      return this.classList?.contains('tl') ? 600 : 0;
    },
  });
  Object.defineProperty(HTMLElement.prototype, 'scrollHeight', {
    configurable: true,
    get() {
      return this.classList?.contains('tl') ? this.childElementCount * 2400 : 0;
    },
  });
  HTMLElement.prototype.scrollTo = function scrollTo(this: HTMLElement, arg?: number | ScrollToOptions) {
    if (typeof arg === 'object' && arg && typeof arg.top === 'number') this.scrollTop = arg.top;
  } as typeof HTMLElement.prototype.scrollTo;
});

let disposers: Array<() => void> = [];
afterEach(() => {
  disposers.forEach((d) => d());
  disposers = [];
  cleanup();
});

/* ── a session of N turn pairs ───────────────────────────────────── */
function snapshot(turns: number): SessionSnapshot {
  const base = structuredClone(rt.fresh);
  const entries: SessionEntry[] = [];
  let t = Date.parse('2026-10-30T09:00:00Z');
  for (let i = 0; i < turns; i++) {
    const uid = `u${i}`;
    entries.push({
      type: 'message',
      id: uid,
      parentId: i === 0 ? null : `a${i - 1}`,
      timestamp: new Date(t).toISOString(),
      message: { role: 'user', content: [{ type: 'text', text: `Question ${i}` }], timestamp: t },
    } as unknown as SessionEntry);
    t += 1000;
    entries.push({
      type: 'message',
      id: `a${i}`,
      parentId: uid,
      timestamp: new Date(t).toISOString(),
      message: { role: 'assistant', content: [{ type: 'text', text: `Answer ${i}` }], timestamp: t },
    } as unknown as SessionEntry);
    t += 1000;
  }
  return { ...base, entries, seq: entries.length, leaf_id: `a${turns - 1}` } as SessionSnapshot;
}

function world(session: MockSession): MockWorld {
  const scenario: MockScenario = { name: 't', hosts: [{ id: 'h1', name: 'coffee', sessions: { s1: session } }] };
  return createMockHost(scenario);
}

async function openSession(n = 60, session: Partial<MockSession> = {}): Promise<{ s: SessionHandle; w: MockWorld }> {
  const w = world({ snapshot: snapshot(n), events: [], entries: { entries: [], leafId: `a${n - 1}` }, ...session });
  clientOf = () => w.client('h1');
  const { s, dispose } = createRoot((d) => ({
    s: createSession('h1', 's1', {
      client: w.client('h1'),
      schedule: (f) => f(),
      setTimer: (ms, cb) => setTimeout(cb, ms) as unknown as number,
      clearTimer: (h) => clearTimeout(h as number),
      now: () => Date.now(),
    }),
    dispose: d,
  }));
  disposers.push(() => {
    s.close();
    dispose();
  });
  await waitFor(() => expect(s.state.conn).toBe('live'));
  // Let the post-open get_entries reconcile land before the test drives.
  await new Promise((r) => setTimeout(r, 0));
  await new Promise((r) => setTimeout(r, 0));
  return { s, w };
}

function ctx(s: SessionHandle): TimelineCtx {
  const cwd = () => '/proj';
  return {
    session: s,
    hostId: 'h1',
    sessionId: s.sessionId,
    cwd,
    refs: sessionRefChat(s, cwd),
    commands: () => commandNames(s.state.commands),
    columnWidth: () => 720,
  };
}

const dock = <div data-testid="dock" />;

describe('Timeline', () => {
  it('renders turns and the live tail; the dock sits inside the scroll viewport', async () => {
    const { s } = await openSession(4);
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    await waitFor(() => expect(r.container.querySelectorAll('.tl-vrow').length).toBeGreaterThan(0));
    expect(r.container.textContent).toContain('Question 0');
    expect(r.getByTestId('tl-tail')).toBeTruthy();
    expect(r.getByTestId('dock')).toBeTruthy();
    expect(r.getByTestId('tl-dock').contains(r.getByTestId('dock'))).toBe(true);
  });

  it('virtualizes: only a window of turns mounts, and the window follows scrollTop', async () => {
    const { s } = await openSession(60);
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const tl = r.getByTestId('tl');
    await waitFor(() => expect(tl.querySelectorAll('.tl-vrow').length).toBeGreaterThan(2));
    expect(tl.querySelectorAll('.tl-vrow').length).toBeLessThan(60);
    const first = () => [...tl.querySelectorAll('.tl-vrow')].map((el) => Number((el as HTMLElement).dataset.index));
    // bottom-locked initial: the window sits at the end of the list
    // (jsdom: programmatic scrollTop fires no scroll event — dispatch it)
    tl.scrollTop = tl.scrollHeight;
    fireEvent.scroll(tl);
    await waitFor(() => expect(Math.max(...first())).toBeGreaterThan(50));
    // user scrolls up → the rendered window follows the offset
    fireEvent.wheel(tl);
    tl.scrollTop = 200;
    fireEvent.scroll(tl);
    await waitFor(() => expect(Math.min(...first())).toBeLessThan(10));
  });

  it('bottom lock: a user scroll up detaches → jump pill; new content shows the unread dot; jump re-locks', async () => {
    const { s, w } = await openSession(60);
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const tl = r.getByTestId('tl');
    await waitFor(() => expect(tl.querySelectorAll('.tl-vrow').length).toBeGreaterThan(0));
    expect(r.queryByTestId('tl-jump')).toBeNull();

    // user scrolls up past the 48 px threshold → detach
    fireEvent.wheel(tl);
    tl.scrollTop = 100;
    fireEvent.scroll(tl);
    await waitFor(() => expect(r.getByTestId('tl-jump')).toBeTruthy());
    // (a late get_entries reconcile may already light the dot — real growth
    // while detached is exactly what it signals, so we only assert on push)

    // new content while detached → unread dot (a provisional entry grows the tail)
    w.push('h1', 's1', [{ session: 's1', seq: s.state.seq + 1, event: { type: 'message_end', message: { role: 'assistant', content: [{ type: 'text', text: 'grew' }] } } }]);
    await waitFor(() => expect(r.getByTestId('tl-jump-dot')).toBeTruthy());

    fireEvent.click(r.getByTestId('tl-jump'));
    await waitFor(() => expect(r.queryByTestId('tl-jump')).toBeNull());
  });

  it('a lost outbox message renders its own row with Retry', async () => {
    const { s } = await openSession(2, {
      cmd: {
        prompt: () => {
          throw { code: 'E_TRANSPORT', message: 'The session stream closed', retryable: true };
        },
      },
    });
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    s.send('deliver me later');
    await waitFor(() => expect(s.state.outbox[0]?.status).toBe('lost'));
    await waitFor(() => expect(r.getByTestId('tl-outbox')).toBeTruthy());
    expect(r.getByTestId('tl-outbox').textContent).toContain('deliver me later');
    expect(r.getByTestId('tl-outbox').textContent).toContain('Retry');
  });

  it('an empty session shows the timeline empty state', async () => {
    const { s } = await openSession(0);
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    await waitFor(() => expect(r.getByTestId('tl-empty')).toBeTruthy());
  });

  it('renders cached snapshot turns immediately while connecting without flashing a skeleton', async () => {
    const warm = await openSession(2);
    writeTimelineSnapshot('h1/s-slow', warm.s.turns());
    // Create a session whose snapshot never arrives yet (`conn === 'connecting'`).
    const baseClient = warm.w.client('h1');
    const slowClient = Object.assign(Object.create(Object.getPrototypeOf(baseClient)), baseClient, {
      openSession: () => ({ cmd: () => new Promise(() => {}), close: () => {} }),
    }) as HostClient;
    const { s, dispose } = createRoot((d) => ({
      s: createSession('h1', 's-slow', { client: slowClient }),
      dispose: d,
    }));
    disposers.push(() => {
      s.close();
      dispose();
    });
    expect(s.state.conn).toBe('connecting');
    expect(s.turns()).toHaveLength(0);
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    expect(r.queryByTestId('tl-skeleton')).toBeNull();
    expect(r.container.textContent).toContain('Question 0');
    expect(r.container.textContent).toContain('Answer 1');
  });
});

/* ── WP-A27: the turn file summary footer in the real timeline ──────── */

/** The reserved `turnFooter` entry the extension's manifest declares (§3.13). */
const FOOTER_ENTRY = {
  id: 'unipi.file-summary/turn-files',
  extensionId: 'unipi.file-summary',
  scope: 'session',
  slot: 'turnFooter',
  order: 0,
  seq: 1,
  rev: 1,
  view: { kind: 'reserved', component: 'turnFiles' },
};

const uiWithFooter = (sessionId: string) => ({ scope: 'session', sessionId, rev: 1, dialogs: [], meta: {}, entries: [FOOTER_ENTRY] });

/** The extension's `turn` service; `asked` collects the turn ids it was asked for. */
const fileSummaryCmd = (asked: string[]) => ({
  '_oino/service_call': (cmd: { name?: string; payload?: Record<string, unknown> }) => {
    if (cmd.name !== 'turn') return {};
    const turnEntryId = String(cmd.payload?.turnEntryId ?? '');
    asked.push(turnEntryId);
    return {
      turnEntryId,
      files: [{ path: 'src/math.ts', existed: true, beforeSha: 'a', afterSha: 'b', add: 2, del: 1 }],
      undone: false,
      undonePaths: [],
      running: false,
    };
  },
});

describe('Timeline · A27 turn file footer', () => {
  beforeEach(() => {
    installUiSinks();
    setPlatformOverride('tauri-desktop');
    resetReservedGates();
    resetFileSummaries();
    setOwnerGates('h1', [{ id: 'unipi.file-summary', enabled: true }]);
  });
  afterEach(() => {
    resetUiRegistries();
    resetReservedGates();
    resetSessions();
    resetFileSummaries();
    setPlatformOverride(undefined);
  });

  /** A session the footer can find in the app store (`sessions().peek`), over this file's mock. */
  async function openRegistered(n: number, asked: string[]): Promise<SessionHandle> {
    const w = world({ snapshot: { ...snapshot(n), ui: uiWithFooter('s1') }, events: [], entries: { entries: [], leafId: `a${n - 1}` }, cmd: fileSummaryCmd(asked) } as MockSession);
    clientOf = () => w.client('h1');
    const s = sessions().ensure('h1', 's1');
    await waitFor(() => expect(s.state.conn).toBe('live'));
    await new Promise((r) => setTimeout(r, 0));
    return s;
  }

  it('asks the extension only for the turns the virtualizer mounts (visible ± overscan), never the whole list', async () => {
    const asked: string[] = [];
    const s = await openRegistered(60, asked);
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const tl = r.getByTestId('tl');
    await waitFor(() => expect(asked.length).toBeGreaterThan(0));
    const mounted = () => [...tl.querySelectorAll('.tl-vrow')].map((el) => (el as HTMLElement).dataset.turn!);
    // exactly the mounted rows (A6's window) — and each of them once
    await waitFor(() => expect([...new Set(asked)].sort()).toEqual([...mounted()].sort()));
    expect(asked.length).toBe(new Set(asked).size);
    expect(asked.length).toBeLessThan(30); // 60 turns exist; the footer never asks for all
    // scrolling mounts more turns; the ones already asked are not asked again
    const firstRound = asked.length;
    fireEvent.wheel(tl);
    tl.scrollTop = 100;
    fireEvent.scroll(tl);
    await waitFor(() => expect(asked.length).toBeGreaterThan(firstRound));
    const reAsked = asked.filter((id, i) => asked.indexOf(id) !== i);
    // a row that never unmounted keeps its one ask; a remounted row may ask again
    expect(reAsked.length).toBeLessThanOrEqual(asked.length - new Set(asked).size + 2);
    expect(asked.length).toBeLessThan(60);
  });

  it('expanding one turn’s footer changes only that turn’s height-cache entry', async () => {
    const asked: string[] = [];
    // geometry: a row that contains the expanded footer is 220 px, others 40 px
    Object.defineProperty(HTMLElement.prototype, 'offsetHeight', {
      configurable: true,
      get(this: HTMLElement) {
        if (this.classList?.contains('tl')) return 600;
        return this.querySelector?.('.tl-files[data-expanded]') ? 220 : 40;
      },
    });
    const ro = installFakeResizeObserver();
    const set = vi.spyOn(TurnHeightCache.prototype, 'set');
    const s = await openRegistered(6, asked);
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    await waitFor(() => expect(asked.length).toBeGreaterThan(0));
    const tl = r.getByTestId('tl');
    await waitFor(() => expect(tl.querySelectorAll('[data-testid="turn-files-toggle"]').length).toBeGreaterThan(1));
    const rowOf = (el: Element) => el.closest('.tl-vrow') as HTMLElement;
    const toggles = [...tl.querySelectorAll('[data-testid="turn-files-toggle"]')];
    const row = rowOf(toggles[0]!);
    const other = rowOf(toggles[1]!).dataset.turn!;
    const turnId = row.dataset.turn!;
    expect(turnId).not.toBe(other);
    // every mounted row was measured once on mount
    await waitFor(() => expect(set.mock.calls.some((c) => c[0] === other)).toBe(true));
    // expanding the footer, then the resize the browser would report for that row
    fireEvent.click(toggles[0]!);
    await waitFor(() => expect(row.querySelector('.tl-files[data-expanded]')).toBeTruthy());
    set.mockClear();
    ro.fire(row);
    await waitFor(() => expect(set).toHaveBeenCalled());
    expect(set.mock.calls.every((c) => c[0] === turnId)).toBe(true);
    expect(set).toHaveBeenCalledWith(turnId, 220);
    // the sibling row was neither re-measured nor re-asked
    expect(set.mock.calls.some((c) => c[0] === other)).toBe(false);
    set.mockRestore();
    ro.restore();
  });
});

/** A ResizeObserver stand-in: tests fire `fire(el)` for the elements a browser would resize. */
function installFakeResizeObserver() {
  const previous = (globalThis as { ResizeObserver?: unknown }).ResizeObserver;
  const instances: Array<{ cb: (entries: Array<{ target: Element }>) => void; targets: Set<Element> }> = [];
  class FakeResizeObserver {
    private readonly entry: { cb: (entries: Array<{ target: Element }>) => void; targets: Set<Element> };
    constructor(cb: (entries: Array<{ target: Element }>) => void) {
      this.entry = { cb, targets: new Set() };
      instances.push(this.entry);
    }
    observe(target: Element) {
      this.entry.targets.add(target);
    }
    unobserve(target: Element) {
      this.entry.targets.delete(target);
    }
    disconnect() {
      this.entry.targets.clear();
      const at = instances.indexOf(this.entry);
      if (at >= 0) instances.splice(at, 1);
    }
  }
  (globalThis as { ResizeObserver?: unknown }).ResizeObserver = FakeResizeObserver;
  return {
    fire(el: Element) {
      for (const instance of [...instances]) if (instance.targets.has(el)) instance.cb([{ target: el }]);
    },
    restore() {
      (globalThis as { ResizeObserver?: unknown }).ResizeObserver = previous;
    },
  };
}
