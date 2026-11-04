// @vitest-environment jsdom
/**
 * Timeline (WP-A6, [chat-load] redesign) over a real `createSession` + MockHost: turns
 * render, the window mounts a slice, the bottom lock detaches/reattaches with the jump
 * pill, the outbox row shows a lost message with retry. Geometry: `flowLayout.testutil`
 * (a flow layout with browser frame timing); the scroll-through-history stability harness
 * is `Timeline.stability.test.tsx`.
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
import { clearSessionHeights, SessionHeights } from '../../lib/timeline/rowModel';
import { installFlowLayout, type FlowLayout } from './flowLayout.testutil';
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

/* ── geometry: a flow layout (flowLayout.testutil) — 600 px viewport, 100 px rows by default ── */
let L: FlowLayout;
const rowH = new Map<string, number>();
beforeEach(() => {
  localStorage.clear();
  clearScrollMemory();
  clearTimelineSnapshots();
  clearSessionHeights();
  rowH.clear();
  L = installFlowLayout({ rowHeight: (id) => rowH.get(id) ?? 100, tailHeight: () => 0 });
});
afterEach(() => L.restore());

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

const viewTopOf = (tl: HTMLElement, id: string) => L.rows(tl).find((r) => r.id === id)?.top;

describe('Timeline', () => {
  it('renders turns and the live tail; the dock sits inside the scroll viewport', async () => {
    const { s } = await openSession(4);
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    await L.frames();
    await waitFor(() => expect(r.container.querySelectorAll('.tl-vrow').length).toBe(4));
    expect(r.container.textContent).toContain('Question 0');
    expect(r.getByTestId('tl-tail')).toBeTruthy();
    expect(r.getByTestId('dock')).toBeTruthy();
    expect(r.getByTestId('tl-dock').contains(r.getByTestId('dock'))).toBe(true);
  });

  it('windows: only the rows around the viewport mount, in flow, and the window follows the reader', async () => {
    const { s } = await openSession(60);
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const tl = r.getByTestId('tl');
    await L.frames();
    const ids = () => L.rows(tl).map((x) => Number(x.id.slice(1)));
    // Pinned to the bottom: the last rows, about three screens' worth.
    expect(tl.scrollTop).toBe(tl.scrollHeight - tl.clientHeight);
    expect(Math.max(...ids())).toBe(59);
    expect(ids().length).toBeLessThan(25);
    // The reader goes up: the window follows; rows are stacked, never on top of each other.
    await L.wheelTo(tl, 200);
    await L.frames();
    expect(Math.min(...ids())).toBe(0);
    const rows = L.rows(tl);
    for (let i = 1; i < rows.length; i++) expect(rows[i]!.top).toBeGreaterThanOrEqual(rows[i - 1]!.bottom - 0.5);
  });

  it('bottom lock: a user scroll up detaches → jump pill; new content shows the unread dot; jump re-locks', async () => {
    const { s, w } = await openSession(60);
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const tl = r.getByTestId('tl');
    await L.frames();
    expect(r.queryByTestId('tl-jump')).toBeNull();
    // 10 px up is a detach (no dead zone)
    await L.wheelTo(tl, tl.scrollTop - 10);
    await waitFor(() => expect(r.getByTestId('tl-jump')).toBeTruthy());
    w.push('h1', 's1', [{ session: 's1', seq: s.state.seq + 1, event: { type: 'message_end', message: { role: 'assistant', content: [{ type: 'text', text: 'grew' }] } } }]);
    await waitFor(() => expect(r.getByTestId('tl-jump-dot')).toBeTruthy());
    fireEvent.click(r.getByTestId('tl-jump'));
    await L.frames();
    await waitFor(() => expect(r.queryByTestId('tl-jump')).toBeNull());
    expect(tl.scrollTop).toBe(tl.scrollHeight - tl.clientHeight);
  });

  it('while locked, content growth keeps the bottom; detached, a row growing ABOVE the reader never moves what they read', async () => {
    const { s } = await openSession(60);
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const tl = r.getByTestId('tl');
    await L.frames();
    rowH.set('u58', 400);
    await L.frames();
    expect(tl.scrollTop).toBe(tl.scrollHeight - tl.clientHeight);
    await L.wheelTo(tl, 3000);
    await L.frames();
    const first = L.rows(tl).find((x) => x.bottom > 0)!;
    const before = viewTopOf(tl, first.id)!;
    // Every mounted row above the anchor grows (cards rendering, markdown settling).
    for (const x of L.rows(tl)) if (x.bottom <= 0) rowH.set(x.id, 260);
    await L.frames();
    expect(Math.abs(viewTopOf(tl, first.id)! - before)).toBeLessThanOrEqual(0.5);
    expect(r.queryByTestId('tl-jump-dot')).toBeNull();
  });

  it('unread dot (chat-scroll #2): scrolling up and row re-measures never set it; tail growth after the detach does', async () => {
    let tail = 40;
    L.tailHeight = () => tail;
    const { s } = await openSession(60);
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const tl = r.getByTestId('tl');
    await L.frames();
    await L.wheelTo(tl, 1000);
    await L.frames();
    await waitFor(() => expect(r.getByTestId('tl-jump')).toBeTruthy());
    for (const x of L.rows(tl)) rowH.set(x.id, 130);
    await L.frames();
    expect(r.queryByTestId('tl-jump-dot')).toBeNull();
    tail = 180;
    await L.frames();
    await waitFor(() => expect(r.getByTestId('tl-jump-dot')).toBeTruthy());
    fireEvent.click(r.getByTestId('tl-jump'));
    await L.frames();
    await waitFor(() => expect(r.queryByTestId('tl-jump')).toBeNull());
  });

  it('bottom lock (chat-scroll #1): output growing inside a height-capped tail accordion follows its bottom; a box scrolled up inside stays', async () => {
    const { s } = await openSession(6);
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const tl = r.getByTestId('tl');
    await L.frames();
    const tail = tl.querySelector('.tl-tail-wrap')!;
    /** A capped body (max-height 240) whose content height the test drives. */
    const box = (cls: string) => {
      const el = document.createElement('pre');
      el.className = cls;
      let content = 100;
      Object.defineProperty(el, 'clientHeight', { configurable: true, get: () => Math.min(content, 240) });
      Object.defineProperty(el, 'scrollHeight', { configurable: true, get: () => content });
      return { el, grow: (h: number) => ((content = h), el.append('x')) };
    };
    const term = box('tool-term-out');
    const read = box('tool-term-out');
    tail.append(term.el, read.el);
    await L.frames(); // first sight: heights recorded, nothing scrolled
    expect(term.el.scrollTop).toBe(0);
    term.grow(400);
    await L.frames();
    expect(term.el.scrollTop).toBe(400 - 240);
    term.grow(900);
    await L.frames();
    expect(term.el.scrollTop).toBe(900 - 240);
    read.grow(400);
    await L.frames();
    read.el.scrollTop = 20;
    read.grow(700);
    await L.frames();
    expect(read.el.scrollTop).toBe(20);
  });

  it('expanding an OLDER accordion keeps its header where it was and detaches without the dot', async () => {
    const { s } = await openSession(60);
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const tl = r.getByTestId('tl');
    await L.frames();
    expect(tl.scrollTop).toBe(tl.scrollHeight - tl.clientHeight);
    const row = L.rows(tl).find((x) => x.top > 100 && x.bottom < 500)!;
    const node = tl.querySelector<HTMLElement>(`.tl-vrow[data-turn="${row.id}"]`)!;
    const header = document.createElement('button');
    header.setAttribute('aria-expanded', 'false');
    node.append(header);
    const at = header.getBoundingClientRect().top;
    fireEvent.click(header);
    rowH.set(row.id, 900); // its body opens below the header
    await L.frames();
    expect(Math.abs(header.getBoundingClientRect().top - at)).toBeLessThanOrEqual(0.5);
    await waitFor(() => expect(r.getByTestId('tl-jump')).toBeTruthy());
    expect(r.queryByTestId('tl-jump-dot')).toBeNull();
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

  it('reopening a chat lands on the remembered anchor turn at the same offset', async () => {
    const { s } = await openSession(60);
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const tl = r.getByTestId('tl');
    await L.frames();
    await L.wheelTo(tl, 2345);
    await L.frames();
    const anchor = L.rows(tl).find((x) => x.bottom > 1)!;
    r.unmount();
    const r2 = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    await L.frames();
    const again = L.rows(r2.getByTestId('tl')).find((x) => x.id === anchor.id)!;
    expect(Math.abs(again.top - anchor.top)).toBeLessThanOrEqual(1);
    expect(r2.getByTestId('tl-jump')).toBeTruthy();
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

  it('asks the extension only for the turns the window mounts (visible ± one screen), never the whole list', async () => {
    const asked: string[] = [];
    const s = await openRegistered(60, asked);
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const tl = r.getByTestId('tl');
    await L.frames();
    await waitFor(() => expect(asked.length).toBeGreaterThan(0));
    const mounted = () => [...tl.querySelectorAll('.tl-vrow')].map((el) => (el as HTMLElement).dataset.turn!);
    // exactly the mounted rows — and each of them once
    await waitFor(() => expect([...new Set(asked)].sort()).toEqual([...mounted()].sort()));
    expect(asked.length).toBe(new Set(asked).size);
    expect(asked.length).toBeLessThan(30); // 60 turns exist; the footer never asks for all
    const firstRound = asked.length;
    await L.wheelTo(tl, 100);
    await L.frames();
    await waitFor(() => expect(asked.length).toBeGreaterThan(firstRound));
    expect(asked.length).toBeLessThan(60);
  });

  it('expanding one turn’s footer changes only that turn’s height', async () => {
    const asked: string[] = [];
    L.rowHeight = (id) => (document.querySelector(`.tl-vrow[data-turn="${id}"] .tl-files[data-expanded]`) ? 220 : 40);
    const set = vi.spyOn(SessionHeights.prototype, 'set');
    const s = await openRegistered(6, asked);
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    await L.frames();
    await waitFor(() => expect(asked.length).toBeGreaterThan(0));
    const tl = r.getByTestId('tl');
    await waitFor(() => expect(tl.querySelectorAll('[data-testid="turn-files-toggle"]').length).toBeGreaterThan(1));
    const rowOf = (el: Element) => el.closest('.tl-vrow') as HTMLElement;
    const toggles = [...tl.querySelectorAll('[data-testid="turn-files-toggle"]')];
    const row = rowOf(toggles[0]!);
    const other = rowOf(toggles[1]!).dataset.turn!;
    const turnId = row.dataset.turn!;
    expect(turnId).not.toBe(other);
    // every mounted row was measured on mount
    await waitFor(() => expect(set.mock.calls.some((c) => c[0].turnId === other)).toBe(true));
    set.mockClear();
    fireEvent.click(toggles[0]!);
    await waitFor(() => expect(row.querySelector('.tl-files[data-expanded]')).toBeTruthy());
    await L.frames();
    // the writes that CHANGED a height (the model is read for every mounted row, only changes count)
    const changed = set.mock.calls.filter((_, i) => set.mock.results[i]?.value === true);
    expect(changed.length).toBeGreaterThan(0);
    expect(changed.every((c) => c[0].turnId === turnId)).toBe(true);
    expect(changed.some((c) => c[1] === 220)).toBe(true);
    set.mockRestore();
  });
});

/* ── perf-open / [chat-load]: paged history — whole-turn chunks ─────────────────── */
describe('Timeline: load earlier', () => {
  /** The newest `shown` turns of a `total`-turn chat, as a paged snapshot; older pages via `get_entries{before}` (host-like). */
  async function openPaged(total: number, shown: number) {
    const all = snapshot(total).entries as SessionEntry[];
    const tail = all.slice(-shown * 2);
    const asked: Array<Record<string, unknown>> = [];
    const paged = { ...snapshot(total), entries: tail, page: { total: all.length, has_more: true, cursor: tail[0]!.id } } as SessionSnapshot;
    const opened = await openSession(total, {
      snapshot: paged,
      cmd: {
        get_entries: (cmd) => {
          asked.push(cmd);
          const before = all.findIndex((e) => e.id === cmd.before);
          const from = Math.max(0, before - Number(cmd.limit ?? 80));
          return { entries: all.slice(from, before), total: all.length, hasMore: from > 0, cursor: all[from]?.id };
        },
      },
    });
    return { ...opened, asked };
  }

  it('shows the fixed-height row above the oldest loaded turn; a click prepends the older chunk; the row goes when nothing is left', async () => {
    rowH.clear();
    // ten turns (estimated 140 px each) fill two 600 px screens: no fill on open
    const { s, asked } = await openPaged(20, 10);
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    await L.frames();
    await waitFor(() => expect(r.getByTestId('tl-earlier')).toBeTruthy());
    expect(asked).toHaveLength(0);
    expect(s.state.entries[0]!.id).toBe('u10');
    fireEvent.click(r.getByTestId('tl-earlier-load'));
    await waitFor(() => expect(s.state.entries[0]!.id).toBe('u0'));
    expect(asked[0]).toMatchObject({ type: 'get_entries', before: 'u10' });
    await L.frames();
    await waitFor(() => expect(r.queryByTestId('tl-earlier')).toBeNull());
    expect(s.state.history?.hasMore).toBe(false);
  });

  it('open fills two screens with whole turns BEFORE the first paint (one swap, no gradual fill)', async () => {
    const { s, asked } = await openPaged(40, 2);
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const tl = r.getByTestId('tl');
    // Until the fill is done the view shows the skeleton (no cached turns), never a half-filled list.
    const seen: number[] = [];
    const mo = new MutationObserver(() => seen.push(tl.querySelectorAll('.tl-vrow').length));
    mo.observe(tl, { childList: true, subtree: true });
    await L.frames();
    mo.disconnect();
    expect(tl.dataset.ready).toBe('');
    expect(asked.length).toBeGreaterThan(0);
    expect(asked.length).toBeLessThanOrEqual(3);
    // 100 px rows, 600 px viewport: at least 12 turns' worth of rows before the reveal.
    const firstShown = seen.find((n) => n > 0) ?? 0;
    expect(firstShown).toBeGreaterThanOrEqual(10);
    expect(tl.scrollTop).toBe(tl.scrollHeight - tl.clientHeight);
  });

  it('scrolling near the top loads ONE older chunk once the scroll is idle — never while bottom-locked, never mid-flick', async () => {
    L.rowHeight = () => 300;
    const { s, asked } = await openPaged(400, 5);
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const tl = r.getByTestId('tl');
    await L.frames();
    const opened = asked.length;
    // Locked at the bottom: nothing loads, whatever the layout does.
    await new Promise((res) => setTimeout(res, 250));
    await L.frames();
    expect(asked.length).toBe(opened);
    // A flick to the top: scroll events every frame — no load while they keep coming.
    for (let y = tl.scrollTop; y > 0; y -= 400) {
      await L.wheelTo(tl, Math.max(0, y - 400));
      expect(asked.length).toBe(opened);
    }
    await new Promise((res) => setTimeout(res, 250));
    await L.frames();
    await waitFor(() => expect(asked.length).toBe(opened + 1));
    // And only one: no reader input since → no second chunk by itself.
    await new Promise((res) => setTimeout(res, 250));
    await L.frames();
    expect(asked.length).toBe(opened + 1);
  });
});

describe('Timeline: never a blank chat (chat-vanish)', () => {
  it('a handle closed under the view (LRU eviction) with no turns draws the error card with Retry, not an empty pane', async () => {
    const { s } = await openSession(0);
    const heal = vi.fn();
    const r = render(() => <Timeline ctx={{ ...ctx(s), heal, hostOnline: () => true }} dock={dock} />);
    await waitFor(() => expect(r.getByTestId('tl-empty')).toBeTruthy());
    s.close(); // conn 'closed', no error — used to render nothing at all
    await waitFor(() => expect(r.getByTestId('tl-open-failed')).toBeTruthy());
    expect(r.queryByTestId('tl-empty')).toBeNull();
    fireEvent.click(r.getByTestId('tl-open-retry'));
    expect(heal).toHaveBeenCalledWith('manual');
  });

  it('self-heal: blank for 2 s while the host is live → ctx.heal(reason), bounded; offline hosts are left alone', async () => {
    const { s } = await openSession(3);
    const heal = vi.fn();
    const [online, setOnline] = [{ v: false }, (v: boolean) => (online.v = v)];
    const r = render(() => <Timeline ctx={{ ...ctx(s), heal, hostOnline: () => online.v }} dock={dock} />);
    await L.frames();
    await waitFor(() => expect(r.container.textContent).toContain('Question 0'));
    s.close();
    await new Promise((res) => setTimeout(res, 2_300));
    expect(heal).not.toHaveBeenCalled(); // host offline (verdict memo read it as offline)
    setOnline(true);
    r.unmount();
    const r2 = render(() => <Timeline ctx={{ ...ctx(s), heal, hostOnline: () => online.v }} dock={dock} />);
    await waitFor(() => expect(heal).toHaveBeenCalledWith('handle_closed'), { timeout: 3_500 });
    // The stale content stays on screen meanwhile (cached turns are never wiped by the heal).
    expect(r2.container.textContent).toContain('Question 0');
  }, 10_000);
});

/* ── the owner's shapes (render-loop 2026-11-04, chat-load 2026-11-04) ─────────── */
describe('Timeline: owner-shaped chat (one giant subagent turn + a short open-ended last turn)', () => {
  /**
   * The owner's 'Frontend Whitelabeling Strategy' shape, synthetic: 1527 entries, 11 user
   * messages, one giant turn (605‥1511) of subagent runs (run_subagent calls + 900
   * subagent-step entries for 6 agents) and a short, open-ended last turn (15 entries).
   * Paged like the host (`page_start`): the newest `limit` entries before `before`, moved
   * forward to the first user message in the page.
   */
  function ownerChat(): SessionEntry[] {
    const users = new Set([0, 5, 27, 36, 46, 54, 62, 90, 105, 141, 605, 1512]);
    const out: SessionEntry[] = [];
    let parent: string | null = null;
    let t = Date.parse('2026-11-03T03:00:00Z');
    const agents = ['67acdafd', '240ed91c', '783a1b39', 'd3874a6b', '04fc7aaf', 'd703771b'];
    for (let i = 0; i < 1527; i++) {
      const id = `e${i}`;
      const ts = new Date((t += 1000)).toISOString();
      let e: Record<string, unknown>;
      if (users.has(i)) e = { type: 'message', message: { role: 'user', content: [{ type: 'text', text: `Question ${i}` }], timestamp: t } };
      else if (i > 605 && i < 612) {
        const a = agents[i - 606]!;
        e = { type: 'message', message: { role: 'assistant', content: [{ type: 'toolCall', id: `call-${a}`, name: 'run_subagent', arguments: { title: `wl-${a}`, profile: 'subagent_general', task: 'x', ...(i > 609 ? { is_background: true } : {}) } }], stopReason: 'toolUse', timestamp: t } };
      } else if (i >= 612 && i < 1500) {
        const a = agents[i % agents.length]!;
        e = { type: 'custom', customType: 'subagent-step', data: { kind: i % 3 ? 'tool' : 'text', name: 'read', arg: `f${i}.rs`, output: 'ok', text: `step ${i}`, agentId: a, id: a, title: `wl-${a}`, profile: 'subagent_general', group: `subagent:${a}:1`, label: `subagent · wl-${a}` } };
      } else if (i >= 1500 && i < 1506) {
        const a = agents[i - 1500]!;
        e = { type: 'message', message: { role: 'toolResult', toolCallId: `call-${a}`, toolName: 'run_subagent', content: [{ type: 'text', text: i > 1503 ? 'Working in the background' : 'done' }], isError: false, timestamp: t } };
      } else if (i % 7 === 0) e = { type: 'custom', customType: 'unipi-memory-session-card', data: { memories: 3 } };
      else e = { type: 'message', message: { role: 'assistant', content: [{ type: 'text', text: `Answer ${i}` }], timestamp: t } };
      out.push({ id, parentId: parent, timestamp: ts, ...e } as unknown as SessionEntry);
      parent = id;
    }
    return out;
  }
  const isUser = (e: SessionEntry) => e.type === 'message' && (e as { message?: { role?: string } }).message?.role === 'user';
  const pageStart = (all: SessionEntry[], end: number, limit: number) => {
    let start = Math.max(0, end - limit);
    if (start > 0) for (let a = start; a < end; a++) if (isUser(all[a]!)) return a;
    return start;
  };

  async function openOwner() {
    const all = ownerChat();
    const tailFrom = pageStart(all, all.length, 120);
    const asked: Array<Record<string, unknown>> = [];
    const base = snapshot(1);
    const paged = { ...base, entries: all.slice(tailFrom), seq: 1, leaf_id: all.at(-1)!.id, page: { total: all.length, has_more: true, cursor: all[tailFrom]!.id } } as SessionSnapshot;
    const opened = await openSession(1, {
      snapshot: paged,
      cmd: {
        get_entries: (cmd) => {
          if (typeof cmd.before !== 'string') return { entries: [], leafId: all.at(-1)!.id };
          asked.push(cmd);
          const end = all.findIndex((e) => e.id === cmd.before);
          const from = pageStart(all, end, Number(cmd.limit ?? 80));
          return { entries: all.slice(from, end), total: all.length, hasMore: from > 0, cursor: all[from]?.id };
        },
      },
    });
    // Geometry like the rig: the giant turn collapsed is one modest row, the early turns are tall.
    L.rowHeight = (id) => (id === 'e605' ? 260 : id === 'e1512' ? 420 : Number(id.slice(1)) < 141 ? 900 : 200);
    return { ...opened, asked, all };
  }

  it('opens with the giant turn WHOLE (never a split "pre:" turn), in a bounded number of host pages, then stays quiet', async () => {
    const { s, asked } = await openOwner();
    expect(s.state.entries.length).toBe(15);
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const tl = r.getByTestId('tl');
    await L.frames();
    await waitFor(() => expect(tl.dataset.ready).toBe(''));
    const ids = s.turns().map((t) => t.turnId);
    expect(ids).toContain('e605');
    expect(ids.some((id) => id.startsWith('pre:'))).toBe(false);
    // the giant turn arrived in one commit: its fold summary counts every run from the first paint
    const giant = s.turns().find((t) => t.turnId === 'e605')!;
    expect(giant.entries.length).toBe(1512 - 605);
    const opened = asked.length;
    expect(opened).toBeLessThanOrEqual(3 * 10);
    // Quiet: pins / layout for a while load nothing, mount nothing new.
    let mutations = 0;
    const mo = new MutationObserver((ms) => (mutations += ms.length));
    mo.observe(tl, { childList: true, subtree: true });
    for (let i = 0; i < 20; i++) await L.frame();
    await new Promise((res) => setTimeout(res, 250));
    await L.frames();
    mo.disconnect();
    expect(asked.length).toBe(opened);
    expect(mutations).toBeLessThan(20);
  });

  it('back at the bottom after reading everything, a trim cuts at a TURN start and nothing pages back in', async () => {
    const { s, asked, all } = await openOwner();
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const tl = r.getByTestId('tl');
    await L.frames();
    for (let i = 0; i < 40 && s.state.history?.hasMore; i++) {
      await L.wheelTo(tl, 0);
      await new Promise((res) => setTimeout(res, 200));
      await L.frames();
    }
    expect(s.state.entries.length).toBe(all.length);
    const loads = asked.length;
    // The reader scrolls back down to the bottom: re-attached → one trim (above LOADED_MAX).
    for (let i = 0; i < 80 && r.queryByTestId('tl-jump'); i++) {
      await L.wheelTo(tl, tl.scrollTop + 4000);
      await L.frames();
    }
    await waitFor(() => expect(s.state.entries.length).toBeLessThan(all.length));
    expect(s.state.entries[0]!.id).toBe('e605');
    const after = s.state.entries.length;
    for (let i = 0; i < 20; i++) await L.frame();
    await new Promise((res) => setTimeout(res, 250));
    await L.frames();
    expect(s.state.entries.length).toBe(after);
    expect(asked.length).toBe(loads);
  });

  it('live pushes (subagent steps, topics) update rows in place: settled rows never remount', async () => {
    const { s, w } = await openOwner();
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const tl = r.getByTestId('tl');
    await L.frames();
    const rows = () => [...tl.querySelectorAll('.tl-vrow > *')];
    const before = rows();
    expect(before.length).toBeGreaterThan(0);
    let seq = s.state.seq;
    let parent = s.state.leafId;
    for (let i = 0; i < 30; i++) {
      const id = `live${i}`;
      w.push('h1', 's1', [
        { session: 's1', seq: ++seq, event: { type: 'topic', topic: 'unipi:subagents:list', data: { tick: i, runs: [{ id: '783a1b39', status: 'running' }] } } },
        { session: 's1', seq: ++seq, event: { type: 'entry_appended', entry: { type: 'custom', id, parentId: parent, timestamp: new Date().toISOString(), customType: 'subagent-step', data: { kind: 'text', text: `tick ${i}`, agentId: '783a1b39', id: '783a1b39', title: 'wl-783a1b39' } } } },
      ] as never);
      parent = id;
      await L.frame();
    }
    await L.frames();
    const after = rows();
    const settledBefore = before.slice(0, -1);
    expect(settledBefore.every((n) => after.includes(n))).toBe(true);
  });
});
