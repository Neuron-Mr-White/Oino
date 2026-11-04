// @vitest-environment jsdom
/**
 * Query navigator rail (WP-A7, Z-25): one bar per user query when the
 * conversation is ≥ 1100 px wide and has ≥ 2 queries; a click lands that
 * turn at the top of the viewport and leaves the timeline detached (the jump
 * pill shows). Narrow columns and single-query sessions render no rail.
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
import { commandNames, type TimelineCtx } from './context';
import { navigatorItems, TurnNavigator } from './TurnNavigator';
import { Timeline } from './Timeline';
import { installFlowLayout } from './flowLayout.testutil';
import roundtrip from '../../lib/session/fixtures/roundtrip.json';

const rt = roundtrip as unknown as { fresh: SessionSnapshot };

vi.mock('../../lib/host/factory', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../lib/host/factory')>()),
  hostClient: () => clientOf(),
}));
let clientOf: () => HostClient = () => {
  throw new Error('clientOf not set');
};

beforeEach(() => {
  localStorage.clear();
  clearScrollMemory();
  Object.defineProperty(HTMLElement.prototype, 'offsetHeight', {
    configurable: true,
    get() {
      return this.classList?.contains('tl') ? 600 : 40;
    },
  });
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

function snapshot(turns: number): SessionSnapshot {
  const base = structuredClone(rt.fresh);
  const entries: SessionEntry[] = [];
  for (let i = 0; i < turns; i++) {
    entries.push({
      type: 'message',
      id: `u${i}`,
      parentId: i === 0 ? null : `a${i - 1}`,
      timestamp: `2026-10-30T09:${String(i).padStart(2, '0')}:00Z`,
      message: { role: 'user', content: [{ type: 'text', text: `Question ${i}` }] },
    } as unknown as SessionEntry);
    entries.push({
      type: 'message',
      id: `a${i}`,
      parentId: `u${i}`,
      timestamp: `2026-10-30T09:${String(i).padStart(2, '0')}:30Z`,
      message: { role: 'assistant', content: [{ type: 'text', text: `Answer ${i}` }], stopReason: 'stop' },
    } as unknown as SessionEntry);
  }
  return { ...base, entries, seq: entries.length, leaf_id: `a${turns - 1}` } as SessionSnapshot;
}

async function open(turns: number): Promise<{ s: SessionHandle; w: MockWorld }> {
  const session: MockSession = { snapshot: snapshot(turns), events: [] };
  const scenario: MockScenario = { name: 't', hosts: [{ id: 'h1', name: 'coffee', sessions: { s1: session } }] };
  const w = createMockHost(scenario);
  clientOf = () => w.client('h1');
  const { s, dispose } = createRoot((d) => ({
    s: createSession('h1', 's1', {
      client: w.client('h1'),
      schedule: (f) => f(),
      setTimer: (ms, cb) => setTimeout(cb, ms) as unknown as number,
      clearTimer: (h) => clearTimeout(h as number),
      now: () => Date.parse('2026-10-30T10:00:00Z'),
    }),
    dispose: d,
  }));
  disposers.push(() => {
    s.close();
    dispose();
  });
  await waitFor(() => expect(s.state.conn).toBe('live'));
  await new Promise((r) => setTimeout(r, 0));
  return { s, w };
}

function ctx(s: SessionHandle, width: number): TimelineCtx {
  const cwd = () => '/proj';
  return {
    session: s,
    hostId: 'h1',
    sessionId: s.sessionId,
    cwd,
    refs: sessionRefChat(s, cwd),
    commands: () => commandNames(s.state.commands),
    columnWidth: () => width,
  };
}

const dock = <div data-testid="dock" />;

describe('navigatorItems', () => {
  it('one item per user query, with the answer excerpt', async () => {
    const { s } = await open(3);
    const items = navigatorItems(s.turns());
    expect(items.map((i) => i.query)).toEqual(['Question 0', 'Question 1', 'Question 2']);
    expect(items[1]!.excerpt).toBe('Answer 1');
    expect(items[1]!.turnId).toBe('u1');
  });
});

describe('TurnNavigator', () => {
  it('renders nothing below 560 px, nor with a single query', async () => {
    const { s } = await open(6);
    const narrow = render(() => <TurnNavigator turns={s.turns} width={() => 500} onJump={() => {}} />);
    expect(narrow.container.querySelector('.tl-nav')).toBeNull();
    cleanup();
    const single = render(() => <TurnNavigator turns={() => s.turns().slice(0, 1)} width={() => 1400} onJump={() => {}} />);
    expect(single.container.querySelector('.tl-nav')).toBeNull();
  });

  it('a click lands the turn at the top of the viewport and detaches (jump pill)', async () => {
    const L = installFlowLayout({ rowHeight: () => 100 });
    try {
      const { s } = await open(40);
      const r = render(() => <Timeline ctx={ctx(s, 1400)} dock={dock} />);
      const tl = r.getByTestId('tl');
      await L.frames();
      const nav = await waitFor(() => r.getByTestId('tl-nav'));
      expect(nav.querySelectorAll('[data-testid=tl-nav-bar]').length).toBe(40);
      expect(nav.querySelector('[data-testid=tl-nav-bar]')!.getAttribute('aria-label')).toContain('Question 0');
      expect(tl.scrollTop).toBe(tl.scrollHeight - tl.clientHeight);
      fireEvent.click(nav.querySelector('[data-nav-turn=u10]') as HTMLElement);
      await L.frames();
      // The row (not mounted before: far above the bottom window) is mounted and sits at the top of the viewport.
      const row = L.rows(tl).find((x) => x.id === 'u10');
      expect(row).toBeTruthy();
      expect(Math.abs(row!.top)).toBeLessThanOrEqual(0.5);
      expect(tl.scrollTop).toBeGreaterThan(0);
      await waitFor(() => expect(r.getByTestId('tl-jump')).toBeTruthy());
    } finally {
      L.restore();
    }
  });

  it('hover opens the one-line list; a row click jumps', async () => {
    const { s } = await open(12);
    const jumped: string[] = [];
    const r = render(() => <TurnNavigator turns={s.turns} width={() => 1400} activeId={() => 'u3'} onJump={(id) => jumped.push(id)} />);
    const nav = r.getByTestId('tl-nav');
    expect(r.queryByTestId('tl-nav-panel')).toBeNull();
    fireEvent.pointerEnter(nav);
    const rows = await waitFor(() => r.getAllByTestId('tl-nav-row'));
    expect(rows.length).toBe(12);
    expect(rows[3]!.hasAttribute('data-active')).toBe(true);
    fireEvent.click(rows[5]!);
    fireEvent.click(r.getAllByTestId('tl-nav-bar')[7]!);
    expect(jumped).toEqual(['u5', 'u7']);
  });

  it('compresses 300 turns into a bounded tick count', async () => {
    const { s } = await open(300);
    const r = render(() => <TurnNavigator turns={s.turns} width={() => 1400} onJump={() => {}} />);
    const n = r.getAllByTestId('tl-nav-bar').length;
    expect(n).toBeLessThanOrEqual(140);
    expect(n).toBeGreaterThan(20);
  });
});
