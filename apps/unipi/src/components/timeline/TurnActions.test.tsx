// @vitest-environment jsdom
/**
 * Turn actions (WP-A7) over a real `createSession` + MockHost: Copy (whole
 * turn, glyph→check), Quote (composer chip), Fork (`session_fork`), Retry
 * (`_oino/navigate_tree` + `prompt` on the last turn), Edit (tree jump that
 * restores text + images into the composer) and the per-message row menu
 * (right-click).
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
import { clearComposer, composerDraft } from '../../lib/timeline/actions';
import { commandNames, type TimelineCtx } from './context';
import { Timeline } from './Timeline';
import { setDesktopOverride } from '../../lib/layout';
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
  // The desktop turn tail (the phone's is covered in TurnActions.phone.test.tsx).
  setDesktopOverride(true);
  localStorage.clear();
  clearScrollMemory();
  clearComposer('h1/s1');
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
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: vi.fn(async (_text: string) => {}) } });
});

let disposers: Array<() => void> = [];
afterEach(() => {
  setDesktopOverride(undefined);
  disposers.forEach((d) => d());
  disposers = [];
  cleanup();
  vi.useRealTimers();
});

/** Two settled turns; the first user message carries an image (Edit restore). */
function snapshot(): SessionSnapshot {
  const base = structuredClone(rt.fresh);
  const entries: SessionEntry[] = [
    {
      type: 'message',
      id: 'u1',
      parentId: null,
      timestamp: '2026-10-30T09:00:00Z',
      message: { role: 'user', content: [{ type: 'text', text: 'Question one' }, { type: 'image', mimeType: 'image/png', data: 'AAAA' }], timestamp: Date.parse('2026-10-30T09:00:00Z') },
    },
    {
      type: 'message',
      id: 'a1',
      parentId: 'u1',
      timestamp: '2026-10-30T09:01:00Z',
      message: { role: 'assistant', content: [{ type: 'text', text: 'Answer one' }], stopReason: 'stop', usage: { cost: { total: 0.0021 } }, timestamp: Date.parse('2026-10-30T09:01:00Z') },
    },
    {
      type: 'message',
      id: 'u2',
      parentId: 'a1',
      timestamp: '2026-10-30T09:02:00Z',
      message: { role: 'user', content: [{ type: 'text', text: 'Question two' }], timestamp: Date.parse('2026-10-30T09:02:00Z') },
    },
    {
      type: 'message',
      id: 'a2',
      parentId: 'u2',
      timestamp: '2026-10-30T09:03:00Z',
      message: { role: 'assistant', content: [{ type: 'text', text: 'Answer two' }], stopReason: 'stop', timestamp: Date.parse('2026-10-30T09:03:00Z') },
    },
  ] as unknown as SessionEntry[];
  return { ...base, entries, seq: entries.length, leaf_id: 'a2' } as SessionSnapshot;
}

function world(): MockWorld {
  const session: MockSession = { snapshot: snapshot(), events: [] };
  const scenario: MockScenario = { name: 't', hosts: [{ id: 'h1', name: 'coffee', sessions: { s1: session } }] };
  return createMockHost(scenario);
}

async function open(): Promise<{ s: SessionHandle; w: MockWorld }> {
  const w = world();
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

function ctx(s: SessionHandle, openSession = vi.fn()): TimelineCtx {
  const cwd = () => '/proj';
  return {
    session: s,
    hostId: 'h1',
    sessionId: s.sessionId,
    cwd,
    refs: sessionRefChat(s, cwd),
    commands: () => commandNames(s.state.commands),
    columnWidth: () => 720,
    openSession,
  };
}

const dock = <div data-testid="dock" />;

describe('turn tail', () => {
  it('copies the whole turn and flips the glyph to a check for 1.2 s (Z-93)', async () => {
    const { s } = await open();
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const copy = await waitFor(() => r.getAllByTestId('tl-copy')[1]!);
    vi.useFakeTimers();
    fireEvent.click(copy);
    await vi.waitFor(() => expect(copy.getAttribute('aria-label')).toBe('Copied'));
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith('Question two\n\nAnswer two');
    vi.advanceTimersByTime(1_300);
    expect(copy.getAttribute('aria-label')).toBe('Copy turn');
  });

  it('shows the done/cost line and the timestamp on the same line', async () => {
    const { s } = await open();
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const tail = await waitFor(() => r.getAllByTestId('tl-turn-tail')[0]!);
    expect(tail.textContent).toContain('Done in 1m');
    expect(tail.textContent).toContain('$0.002');
    expect(tail.querySelector('[data-testid=tl-time]')).toBeTruthy();
  });

  it('quotes into the composer as a chip', async () => {
    const { s } = await open();
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const quote = await waitFor(() => r.getAllByTestId('tl-quote')[0]!);
    fireEvent.click(quote);
    expect(composerDraft(s.key).quotes.map((q) => q.text)).toEqual(['Answer one']);
  });

  it('forks the session with session_fork from the turn query (Z-17)', async () => {
    const { s, w } = await open();
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const fork = await waitFor(() => r.getAllByTestId('tl-fork')[1]!);
    fireEvent.click(fork);
    await waitFor(() => expect(w.log.some((e) => e.op === 'session_fork')).toBe(true));
    const call = w.log.find((e) => e.op === 'session_fork')!;
    expect(call.args).toMatchObject({ kind: 'fork', session: 's1', fromEntry: 'u2' });
  });

  it('retries the last turn through navigate_tree + prompt (Z-49)', async () => {
    const { s, w } = await open();
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const retry = await waitFor(() => r.getAllByTestId('tl-retry')[0]!);
    fireEvent.click(retry);
    await waitFor(() => expect(w.log.some((e) => e.op === '_oino/navigate_tree')).toBe(true));
    expect(w.log.find((e) => e.op === '_oino/navigate_tree')!.args).toMatchObject({ type: '_oino/navigate_tree', targetId: 'u2' });
    await waitFor(() => expect(w.log.some((e) => e.op === 'prompt')).toBe(true));
    expect(w.log.find((e) => e.op === 'prompt')!.args).toMatchObject({ type: 'prompt', message: 'Question two' });
    await waitFor(() => expect(s.state.outbox.some((o) => o.text === 'Question two')).toBe(true));
  });

  it('Edit offers "Also undo file changes" when the file summary can undo the turn (Z-28)', async () => {
    const { s, w } = await open();
    const undone: string[] = [];
    const base = ctx(s);
    const c: TimelineCtx = { ...base, undoFiles: { canUndo: (turnId) => turnId === 'u1', undoFrom: async (turnId) => void undone.push(turnId) } };
    const r = render(() => <Timeline ctx={c} dock={dock} />);
    const edit = await waitFor(() => r.getAllByTestId('tl-user-edit')[0]!);
    fireEvent.click(edit);
    const menu = await waitFor(() => {
      const el = document.querySelector('[role=menu][aria-label="Edit message"]');
      expect(el).toBeTruthy();
      return el as HTMLElement;
    });
    const items = () => [...menu.querySelectorAll('[role=menuitem]')];
    expect(items().some((el) => el.textContent?.includes('Also undo file changes'))).toBe(true);
    fireEvent.click(items().find((el) => el.textContent?.includes('Also undo file changes'))!);
    fireEvent.click(items().find((el) => el.textContent?.includes('Edit message'))!);
    await waitFor(() => expect(undone).toEqual(['u1']));
    await waitFor(() => expect(w.log.some((e) => e.op === '_oino/navigate_tree')).toBe(true));
    await waitFor(() => expect(composerDraft(s.key).text).toBe('Question one'));
  });

  it('edits a user message: tree jump + text and images back in the composer (Z-28)', async () => {
    const { s, w } = await open();
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const edit = await waitFor(() => r.getAllByTestId('tl-user-edit')[0]!);
    fireEvent.click(edit);
    await waitFor(() => expect(w.log.some((e) => e.op === '_oino/navigate_tree')).toBe(true));
    expect(w.log.find((e) => e.op === '_oino/navigate_tree')!.args).toMatchObject({ targetId: 'u1' });
    await waitFor(() => expect(composerDraft(s.key).text).toBe('Question one'));
    expect(composerDraft(s.key).images).toEqual([{ mimeType: 'image/png', data: 'AAAA' }]);
  });
});

describe('per-message row menu', () => {
  it('right-click on an answer offers Copy / Quote / Fork, not Edit', async () => {
    const { s } = await open();
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const prose = await waitFor(() => r.container.querySelector('.tl-prose')!);
    fireEvent.contextMenu(prose, { clientX: 20, clientY: 20 });
    const menu = await waitFor(() => {
      const el = document.querySelector('[role=menu][aria-label="Message actions"]');
      expect(el).toBeTruthy();
      return el as HTMLElement;
    });
    const labels = [...menu.querySelectorAll('[role=menuitem]')].map((el) => el.textContent);
    expect(labels.some((l) => l?.includes('Copy'))).toBe(true);
    expect(labels.some((l) => l?.includes('Quote'))).toBe(true);
    expect(labels.some((l) => l?.includes('Fork from here'))).toBe(true);
    expect(labels.some((l) => l?.includes('Edit message'))).toBe(false);
  });

  it('right-click on a user bubble offers Edit (and forks from that message)', async () => {
    const { s, w } = await open();
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const bubble = await waitFor(() => r.container.querySelector('.tl-user .chat-bubble')!);
    fireEvent.contextMenu(bubble, { clientX: 20, clientY: 20 });
    const menu = await waitFor(() => {
      const el = document.querySelector('[role=menu][aria-label="Message actions"]');
      expect(el).toBeTruthy();
      return el as HTMLElement;
    });
    const fork = [...menu.querySelectorAll('[role=menuitem]')].find((el) => el.textContent?.includes('Fork from here'))!;
    fireEvent.click(fork);
    await waitFor(() => expect(w.log.some((e) => e.op === 'session_fork')).toBe(true));
    expect(w.log.find((e) => e.op === 'session_fork')!.args).toMatchObject({ fromEntry: 'u1' });
  });
});
