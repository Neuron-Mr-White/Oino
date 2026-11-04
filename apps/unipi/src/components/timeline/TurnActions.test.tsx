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

  it('Fork on an answer keeps the turn: before the next message, a clone on the last turn (chat-tree)', async () => {
    const { s, w } = await open();
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const forks = await waitFor(() => {
      const all = r.getAllByTestId('tl-fork');
      expect(all).toHaveLength(2);
      return all;
    });
    fireEvent.click(forks[0]!);
    await waitFor(() => expect(w.log.filter((e) => e.op === 'session_fork')).toHaveLength(1));
    expect(w.log.find((e) => e.op === 'session_fork')!.args).toMatchObject({ kind: 'fork', session: 's1', fromEntry: 'u2' });
    fireEvent.click(forks[1]!);
    await waitFor(() => expect(w.log.filter((e) => e.op === 'session_fork')).toHaveLength(2));
    const clone = w.log.filter((e) => e.op === 'session_fork')[1]!.args as Record<string, unknown>;
    expect(clone).toMatchObject({ kind: 'fork', session: 's1' });
    expect(clone.fromEntry).toBeUndefined();
  });

  it('retries ANY turn through navigate_tree + prompt, not only the last (Z-49, chat-tree)', async () => {
    const { s, w } = await open();
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const retries = await waitFor(() => {
      const all = r.getAllByTestId('tl-retry');
      expect(all).toHaveLength(2);
      return all;
    });
    fireEvent.click(retries[0]!);
    await waitFor(() => expect(w.log.some((e) => e.op === '_oino/navigate_tree')).toBe(true));
    expect(w.log.find((e) => e.op === '_oino/navigate_tree')!.args).toMatchObject({ type: '_oino/navigate_tree', targetId: 'u1' });
    await waitFor(() => expect(w.log.some((e) => e.op === 'prompt')).toBe(true));
    expect(w.log.find((e) => e.op === 'prompt')!.args).toMatchObject({ type: 'prompt', message: 'Question one' });
  });

  it('retries the last turn through navigate_tree + prompt (Z-49)', async () => {
    const { s, w } = await open();
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const retry = await waitFor(() => r.getAllByTestId('tl-retry')[1]!);
    fireEvent.click(retry);
    await waitFor(() => expect(w.log.some((e) => e.op === '_oino/navigate_tree')).toBe(true));
    expect(w.log.find((e) => e.op === '_oino/navigate_tree')!.args).toMatchObject({ type: '_oino/navigate_tree', targetId: 'u2' });
    await waitFor(() => expect(w.log.some((e) => e.op === 'prompt')).toBe(true));
    expect(w.log.find((e) => e.op === 'prompt')!.args).toMatchObject({ type: 'prompt', message: 'Question two' });
    await waitFor(() => expect(s.state.outbox.some((o) => o.text === 'Question two')).toBe(true));
  });

  it('Return here on an earlier turn moves the leaf to its answer; the last turn has none (chat-tree)', async () => {
    const { s, w } = await open();
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const back = await waitFor(() => {
      const all = r.getAllByTestId('tl-return');
      // The last turn already is the leaf: only the first turn offers it.
      expect(all).toHaveLength(1);
      return all[0]!;
    });
    fireEvent.click(back);
    await waitFor(() => expect(w.log.some((e) => e.op === '_oino/navigate_tree')).toBe(true));
    expect(w.log.find((e) => e.op === '_oino/navigate_tree')!.args).toEqual({ type: '_oino/navigate_tree', targetId: 'a1' });
    // The new branch is reloaded as one tail page.
    await waitFor(() => expect(w.log.some((e) => e.op === 'get_entries' && (e.args as { limit?: number }).limit === 80)).toBe(true));
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
    await waitFor(() => expect(composerDraft(s.key).text).toMatch(/^Question one \[image]@img:\S+ $/));
  });

  it('edits a user message: tree jump + text and images back in the composer (Z-28)', async () => {
    const { s, w } = await open();
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const edit = await waitFor(() => r.getAllByTestId('tl-user-edit')[0]!);
    fireEvent.click(edit);
    await waitFor(() => expect(w.log.some((e) => e.op === '_oino/navigate_tree')).toBe(true));
    expect(w.log.find((e) => e.op === '_oino/navigate_tree')!.args).toMatchObject({ targetId: 'u1' });
    await waitFor(() => expect(composerDraft(s.key).text).toMatch(/^Question one \[image]@img:\S+ $/));
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

  it('right-click on an answer of an earlier turn offers Return here and Retry (chat-tree)', async () => {
    const { s, w } = await open();
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const prose = await waitFor(() => r.container.querySelector('.tl-prose')!);
    fireEvent.contextMenu(prose, { clientX: 20, clientY: 20 });
    const menu = await waitFor(() => {
      const el = document.querySelector('[role=menu][aria-label="Message actions"]');
      expect(el).toBeTruthy();
      return el as HTMLElement;
    });
    const labels = [...menu.querySelectorAll('[role=menuitem]')].map((el) => el.textContent ?? '');
    expect(labels).toEqual(expect.arrayContaining(['Copy', 'Quote', 'Fork from here', 'Return here', 'Retry', 'Show in tree']));
    fireEvent.click([...menu.querySelectorAll('[role=menuitem]')].find((el) => el.textContent === 'Return here')!);
    await waitFor(() => expect(w.log.some((e) => e.op === '_oino/navigate_tree')).toBe(true));
    expect(w.log.find((e) => e.op === '_oino/navigate_tree')!.args).toMatchObject({ targetId: 'a1' });
  });
});

describe('user message actions (chat-tree)', () => {
  it('every user message has Copy / Quote / Edit / Fork / Retry under it', async () => {
    const { s } = await open();
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    const rows = await waitFor(() => {
      const all = r.getAllByTestId('tl-user-actions');
      expect(all).toHaveLength(2);
      return all;
    });
    for (const row of rows) {
      const ids = [...row.querySelectorAll('button')].map((b) => b.getAttribute('data-testid'));
      expect(ids).toEqual(['tl-user-copy', 'tl-user-quote', 'tl-user-edit', 'tl-user-fork', 'tl-user-retry']);
    }
    fireEvent.click(rows[1]!.querySelector('[data-testid=tl-user-copy]')!);
    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalledWith('Question two'));
    fireEvent.click(rows[1]!.querySelector('[data-testid=tl-user-quote]')!);
    expect(composerDraft(s.key).quotes.map((q) => q.text)).toEqual(['Question two']);
  });

  it('Fork from a user message forks before it and refills the fork\'s composer (pi fork)', async () => {
    const { s, w } = await open();
    const openSession = vi.fn();
    // The host streams the fork under its new id (the mock would echo the source's).
    const base = w.client('h1');
    clientOf = () =>
      Object.assign(Object.create(base) as HostClient, {
        openSession: (req: Parameters<HostClient['openSession']>[0], sink: Parameters<HostClient['openSession']>[1]) => {
          base.openSession(req, { ...sink, snapshot: () => {}, events: () => {}, closed: () => {} }).close();
          queueMicrotask(() => sink.snapshot({ ...snapshot(), session_id: 's9' }));
          return { cmd: async () => undefined, close: () => {} } as never;
        },
      });
    const r = render(() => <Timeline ctx={ctx(s, openSession)} dock={dock} />);
    const fork = await waitFor(() => r.getAllByTestId('tl-user-fork')[1]!);
    fireEvent.click(fork);
    await waitFor(() => expect(w.log.some((e) => e.op === 'session_fork')).toBe(true));
    expect(w.log.find((e) => e.op === 'session_fork')!.args).toMatchObject({ kind: 'fork', session: 's1', fromEntry: 'u2' });
    await waitFor(() => expect(openSession).toHaveBeenCalled());
    expect(openSession).toHaveBeenCalledWith('s9');
    expect(composerDraft('h1/s9').text).toBe('Question two');
  });

  it('Retry from a user message re-sends it on a fresh branch', async () => {
    const { s, w } = await open();
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    fireEvent.click(await waitFor(() => r.getAllByTestId('tl-user-retry')[0]!));
    await waitFor(() => expect(w.log.some((e) => e.op === 'prompt')).toBe(true));
    expect(w.log.find((e) => e.op === '_oino/navigate_tree')!.args).toMatchObject({ targetId: 'u1' });
    expect(w.log.find((e) => e.op === 'prompt')!.args).toMatchObject({ message: 'Question one' });
  });
});

describe('phone: tap a message for its actions (chat-tree)', () => {
  beforeEach(() => setDesktopOverride(false));

  it('only the last turn shows its tail until another reply or message is tapped', async () => {
    const { s } = await open();
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    await waitFor(() => expect(r.getAllByTestId('tl-turn-tail')).toHaveLength(1));
    expect(r.queryAllByTestId('tl-user-actions')).toHaveLength(0);
    // Tap the first answer: its tail (with Return here) shows too.
    fireEvent.click(r.container.querySelector('.tl-prose')!);
    await waitFor(() => expect(r.getAllByTestId('tl-turn-tail')).toHaveLength(2));
    expect(r.getAllByTestId('tl-return')).toHaveLength(1);
    // Tap the first user message: its row shows instead (one message at a time).
    fireEvent.click(r.container.querySelector('.tl-user .chat-bubble')!);
    await waitFor(() => expect(r.getAllByTestId('tl-user-actions')).toHaveLength(1));
    expect(r.getAllByTestId('tl-turn-tail')).toHaveLength(1);
    // Tap it again: hidden.
    fireEvent.click(r.container.querySelector('.tl-user .chat-bubble')!);
    await waitFor(() => expect(r.queryAllByTestId('tl-user-actions')).toHaveLength(0));
  });

  it('the sheet of an earlier reply offers Fork / Return here / Retry / Show in tree', async () => {
    const { s } = await open();
    const r = render(() => <Timeline ctx={ctx(s)} dock={dock} />);
    fireEvent.click(await waitFor(() => r.container.querySelector('.tl-prose')!));
    const more = await waitFor(() => r.getAllByTestId('tl-more')[0]!);
    fireEvent.click(more);
    const sheet = await waitFor(() => {
      const el = document.querySelector('[data-testid=message-sheet]');
      expect(el).toBeTruthy();
      return el as HTMLElement;
    });
    for (const label of ['Fork from here', 'Return here', 'Retry', 'Show in tree']) expect(sheet.textContent).toContain(label);
  });
});
