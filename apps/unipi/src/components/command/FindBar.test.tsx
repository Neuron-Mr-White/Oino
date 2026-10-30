// @vitest-environment jsdom
/**
 * FindBar + Timeline integration (WP-A18, Z-79): the bar counts loaded
 * matches, highlights them in the virtual timeline, and a match in an
 * *unmounted* turn scrolls that turn into view and then highlights it
 * (the acceptance case). "Search whole session" falls back to the host
 * `search` command (one surface).
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
import { commandNames, type TimelineCtx } from '../timeline/context';
import { Timeline } from '../timeline/Timeline';
import { closeFind, findState, gotoFindMatch, openFind, setFindQuery } from '../../lib/find';
import roundtrip from '../../lib/session/fixtures/roundtrip.json';

const rt = roundtrip as unknown as { fresh: SessionSnapshot };

vi.mock('../../lib/host/factory', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../lib/host/factory')>()),
  hostClient: () => clientOf(),
}));
let clientOf: () => HostClient = () => {
  throw new Error('clientOf not set');
};

/* ── geometry stubs like Timeline.test ───────────────────────────── */
beforeEach(() => {
  localStorage.clear();
  clearScrollMemory();
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
  closeFind();
  disposers.forEach((d) => d());
  disposers = [];
  cleanup();
});

function snapshot(turns: number, mark = (i: number) => `Question ${i}`): SessionSnapshot {
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
      message: { role: 'user', content: [{ type: 'text', text: mark(i) }], timestamp: t },
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

async function mount(n = 60, session: Partial<MockSession> = {}): Promise<{ s: SessionHandle; w: MockWorld }> {
  const scenario: MockScenario = { name: 't', hosts: [{ id: 'h1', name: 'coffee', sessions: { s1: { snapshot: snapshot(n), events: [], entries: { entries: [], leafId: `a${n - 1}` }, ...session } } }] };
  const w = createMockHost(scenario);
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
  await new Promise((r) => setTimeout(r, 0));
  await new Promise((r) => setTimeout(r, 0));
  const cwd = () => '/proj';
  const ctx: TimelineCtx = { session: s, hostId: 'h1', sessionId: s.sessionId, cwd, refs: sessionRefChat(s, cwd), commands: () => commandNames(s.state.commands), columnWidth: () => 720 };
  render(() => <Timeline ctx={ctx} />);
  await new Promise((r) => setTimeout(r, 0));
  return { s, w };
}

describe('FindBar (Z-79)', () => {
  it('counts loaded matches and highlights them in place', async () => {
    await mount(6, { snapshot: snapshot(6, (i) => (i % 2 ? `tally row ${i}` : `other row ${i}`)) });
    openFind();
    await waitFor(() => expect(document.querySelector('[data-testid="find-bar"]')).toBeInTheDocument());
    const input = document.querySelector<HTMLInputElement>('.cfind-input')!;
    fireEvent.input(input, { target: { value: 'tally' } });
    await waitFor(() => expect(document.querySelector('[data-testid="find-count"]')!.textContent).toBe('1/3'));
    await waitFor(() => expect(document.querySelectorAll('mark.find-hit').length).toBe(3));
  });

  it('a match in an unmounted turn scrolls into view, then highlights (acceptance)', async () => {
    // 60 turns, only the first fit the stubbed window; the match lives in turn 50.
    await mount(60);
    openFind();
    await waitFor(() => expect(document.querySelector('[data-testid="find-bar"]')).toBeInTheDocument());
    const mounted = document.querySelectorAll('[data-turn]').length;
    expect(mounted).toBeGreaterThan(0);
    expect(document.querySelector('[data-turn="u50"]')).toBeNull(); // not mounted
    fireEvent.input(document.querySelector<HTMLInputElement>('.cfind-input')!, { target: { value: 'Question 50' } });
    await waitFor(() => expect(document.querySelector('[data-testid="find-count"]')!.textContent).toBe('1/1'));
    await waitFor(
      () => {
        const row = document.querySelector('[data-turn="u50"]');
        if (!row) throw new Error('row not mounted');
        if (!row.querySelector('mark.find-hit[data-active]')) throw new Error(`no active mark; html=${row.innerHTML.slice(0, 160)}`);
      },
      { timeout: 3000 },
    );
  });

  it('Enter / Shift+Enter wrap; Esc closes and clears the marks', async () => {
    await mount(4, { snapshot: snapshot(4, (i) => (i % 2 ? `tally row ${i}` : `other row ${i}`)) });
    openFind();
    const input = () => document.querySelector<HTMLInputElement>('.cfind-input')!;
    await waitFor(() => expect(input()).toBeTruthy());
    fireEvent.input(input(), { target: { value: 'tally' } });
    await waitFor(() => expect(document.querySelector('[data-testid="find-count"]')!.textContent).toBe('1/2'));
    fireEvent.keyDown(input(), { key: 'Enter' });
    await waitFor(() => expect(document.querySelector('[data-testid="find-count"]')!.textContent).toBe('2/2'));
    fireEvent.keyDown(input(), { key: 'Enter' }); // wraps
    await waitFor(() => expect(document.querySelector('[data-testid="find-count"]')!.textContent).toBe('1/2'));
    fireEvent.keyDown(input(), { key: 'Enter', shiftKey: true }); // back
    await waitFor(() => expect(document.querySelector('[data-testid="find-count"]')!.textContent).toBe('2/2'));
    fireEvent.keyDown(input(), { key: 'Escape' });
    await waitFor(() => expect(document.querySelector('[data-testid="find-bar"]')).toBeNull());
    expect(document.querySelectorAll('mark.find-hit').length).toBe(0);
    expect(findState().open).toBe(false);
  });

  it('"Search whole session" falls back to the host search command (one surface)', async () => {
    const { w } = await mount(4);
    openFind();
    const input = () => document.querySelector<HTMLInputElement>('.cfind-input')!;
    await waitFor(() => expect(input()).toBeTruthy());
    fireEvent.input(input(), { target: { value: 'Answer 2' } });
    await waitFor(() => expect(document.querySelector('[data-testid="find-count"]')!.textContent).toBe('1/1'));
    fireEvent.click(document.querySelector<HTMLButtonElement>('.cfind-whole')!);
    await waitFor(() => expect(w.log.some((e) => e.kind === 'cmd' && e.op === 'search')).toBe(true));
    await waitFor(() => expect(document.querySelector('[data-testid="find-whole"]')).toBeInTheDocument());
    await waitFor(() => expect(document.querySelectorAll('.cfind-hit').length).toBe(1));
    // picking a hit of the current branch jumps to its turn (no tree op needed)
    fireEvent.click(document.querySelector<HTMLElement>('.cfind-hit')!);
    await waitFor(() => expect(w.log.some((e) => e.kind === 'cmd' && e.op === '_oino/navigate_tree')).toBe(false));
    await waitFor(() => expect(document.querySelector('[data-turn="u2"] mark.find-hit')).not.toBeNull());
  });

  it('a hit on another branch tree-jumps first (then lands on its turn)', async () => {
    // the host search answers with an entry of an older branch (`u99`)
    const { w } = await mount(4, {
      cmd: { search: () => ({ hits: [{ id: 'u99', role: 'user', snippet: 'the old branch question', marks: [[4, 15]], score: 1 }] }) },
    });
    openFind();
    const input = () => document.querySelector<HTMLInputElement>('.cfind-input')!;
    await waitFor(() => expect(input()).toBeTruthy());
    fireEvent.input(input(), { target: { value: 'branch' } });
    fireEvent.click(document.querySelector<HTMLButtonElement>('.cfind-whole')!);
    await waitFor(() => expect(document.querySelectorAll('.cfind-hit').length).toBeGreaterThan(0));
    fireEvent.click(document.querySelector<HTMLElement>('.cfind-hit')!);
    await waitFor(() => expect(w.log.some((e) => e.kind === 'cmd' && e.op === '_oino/navigate_tree')).toBe(true));
    void gotoFindMatch;
    void setFindQuery;
  });
});
