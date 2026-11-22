// @vitest-environment jsdom
/**
 * w7-ui G3-1: reference-only stubs (`{id, truncated:true}`: a huge tool CALL, a huge tool RESULT, a huge ANSWER)
 * must render again after an app relaunch. The persisted timeline snapshot is only ever taken from hydrated
 * content, stub hydration runs for the reopened chat, a hydrated entry whose `role` the host dropped is repaired,
 * a failing fetch is retried and shown, and one failing id never fails the batch.
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
import { clearTimelineSnapshots } from '../../lib/timeline/snapshotCache';
import { commandNames, type TimelineCtx } from './context';
import { Timeline } from './Timeline';
import { clearSessionHeights } from '../../lib/timeline/rowModel';
import { installFlowLayout, type FlowLayout } from './flowLayout.testutil';
import { resetFileSummaries } from '../../lib/fileSummary';
import roundtrip from '../../lib/session/fixtures/roundtrip.json';

const rt = roundtrip as unknown as { fresh: SessionSnapshot };

vi.mock('../../lib/host/factory', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../lib/host/factory')>()),
  hostClient: () => clientOf(),
}));
let clientOf: () => HostClient = () => {
  throw new Error('clientOf not set');
};

let L: FlowLayout;
beforeEach(() => {
  localStorage.clear();
  clearScrollMemory();
  clearTimelineSnapshots();
  clearSessionHeights();
  resetFileSummaries();
  L = installFlowLayout({ rowHeight: () => 100, tailHeight: () => 0 });
});
let disposers: Array<() => void> = [];
afterEach(() => {
  disposers.forEach((d) => d());
  disposers = [];
  cleanup();
  L.restore();
});

const HUGE_CMD = `echo start\n${'echo "arg line lorem ipsum dolor sit amet"\n'.repeat(40_000)}HUGEARGS-END`;
const HUGE_OUT = `${'out line the quick brown fox\n'.repeat(40_000)}HUGEOUT-END`;
const HUGE_ANSWER = `${Array.from({ length: 3_000 }, (_, i) => `Paragraph ${i + 1} of the report.`).join('\n\n')}\n\nHUGE-ANSWER-END`;

const msg = (n: number, message: Record<string, unknown>): SessionEntry =>
  ({ type: 'message', id: `c${n}`, parentId: n > 1 ? `c${n - 1}` : null, timestamp: new Date(Date.parse('2026-11-14T09:00:00Z') + n * 3000).toISOString(), message: { ...message, timestamp: n * 1000 } }) as unknown as SessionEntry;
const asst = (content: unknown[], stopReason: string) => ({ role: 'assistant', content, provider: 'mock', model: 'm', stopReason });
const ENTRIES: SessionEntry[] = [
  msg(1, { role: 'user', content: [{ type: 'text', text: 'run the big script' }] }),
  msg(2, asst([{ type: 'text', text: 'Running the big script.' }, { type: 'toolCall', id: 'big1', name: 'bash', arguments: { command: HUGE_CMD } }], 'toolUse')),
  msg(3, { role: 'toolResult', toolCallId: 'big1', toolName: 'bash', content: [{ type: 'text', text: HUGE_OUT }], isError: false }),
  msg(4, asst([{ type: 'text', text: 'HUGE-TOOL-AFTER: the script finished.' }], 'stop')),
  msg(5, { role: 'user', content: [{ type: 'text', text: 'now write the long report' }] }),
  msg(6, asst([{ type: 'text', text: HUGE_ANSWER }], 'stop')),
];
const STUBS = new Set(['c2', 'c3', 'c6']);

type Host = { dropRole: boolean; fail: Set<string>; gate?: Promise<void>; asked: string[] };
function session(host: Host): MockSession {
  const base = structuredClone(rt.fresh);
  return {
    snapshot: { ...base, entries: ENTRIES.map((e) => (STUBS.has(e.id) ? { id: e.id, truncated: true } : e)), seq: ENTRIES.length, leaf_id: 'c6' } as unknown as SessionSnapshot,
    events: [],
    entries: { entries: [], leafId: 'c6' },
    cmd: {
      get_entries: async (cmd) => {
        const ask = cmd as { ids?: string[]; chunkOffset?: number };
        if (!ask.ids) return { entries: [], leafId: 'c6' };
        await host.gate;
        if (ask.chunkOffset === undefined) return { entries: ask.ids.filter((id) => STUBS.has(id)).map((id) => ({ id, truncated: true })).concat(ask.ids.filter((id) => !STUBS.has(id)).map((id) => ENTRIES.find((e) => e.id === id) as never)), leafId: 'c6' };
        const id = ask.ids[0]!;
        host.asked.push(id);
        if (host.fail.has(id)) throw { code: 'E_INTERNAL', message: `no chunk for ${id}`, retryable: true };
        let entry = structuredClone(ENTRIES.find((e) => e.id === id)!) as unknown as { message?: { role?: string } };
        if (host.dropRole && entry.message) delete entry.message.role; // the host's role-less typed serialisation
        const text = JSON.stringify(entry);
        const start = ask.chunkOffset;
        const end = Math.min(start + 64 * 1024, text.length);
        return { json: text.slice(start, end), offset: start, nextOffset: end < text.length ? end : null };
      },
    },
  };
}

async function launch(host: Host): Promise<{ s: SessionHandle; w: MockWorld }> {
  const scenario: MockScenario = { name: 't', hosts: [{ id: 'h1', name: 'coffee', sessions: { s1: session(host) } }] };
  const w = createMockHost(scenario);
  clientOf = () => w.client('h1');
  const { s, dispose } = createRoot((d) => ({
    s: createSession('h1', 's1', { client: w.client('h1'), schedule: (f) => f(), setTimer: (ms, cb) => setTimeout(cb, ms) as unknown as number, clearTimer: (h) => clearTimeout(h as number), now: () => Date.now() }),
    dispose: d,
  }));
  disposers.push(() => (s.close(), dispose()));
  await waitFor(() => expect(s.state.conn).toBe('live'));
  return { s, w };
}
const ctx = (s: SessionHandle): TimelineCtx => {
  const cwd = () => '/proj';
  return { session: s, hostId: 'h1', sessionId: s.sessionId, cwd, refs: sessionRefChat(s, cwd), commands: () => commandNames(s.state.commands), columnWidth: () => 720 };
};
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const SNAP = 'unipi.tl.snap.item.v2:h1/s1';

/** What a relaunch keeps: localStorage only (the in-memory snapshot LRU and every session handle are gone). */
function relaunch(): void {
  const keep = Object.entries(localStorage);
  disposers.forEach((d) => d());
  disposers = [];
  cleanup();
  clearTimelineSnapshots();
  keep.forEach(([k, v]) => localStorage.setItem(k, v));
  clearSessionHeights();
}

const expectWhole = (r: ReturnType<typeof render>) => {
  const text = () => r.container.textContent ?? '';
  return waitFor(() => {
    expect(text()).toContain('HUGE-TOOL-AFTER');
    expect(text()).toContain('ran 1 command');
    expect(text()).toContain('HUGE-ANSWER-END');
  }, { timeout: 5000 });
};

describe('G3-1: stub entries after a relaunch', () => {
  it('renders the huge call, result and answer in the first session and again after a relaunch', async () => {
    const first = await launch({ dropRole: false, fail: new Set(), asked: [] });
    let r = render(() => <Timeline ctx={ctx(first.s)} />);
    await L.frames();
    await expectWhole(r);
    await sleep(400); // the debounced snapshot write
    const persisted = localStorage.getItem(SNAP);
    expect(persisted).toBeTruthy();
    expect(persisted!.length).toBeLessThan(900_000); // bounded: the 1.4 MB command is cut

    relaunch();
    const host: Host = { dropRole: true, fail: new Set(), asked: [] };
    const second = await launch(host);
    r = render(() => <Timeline ctx={ctx(second.s)} />);
    await L.frames();
    await expectWhole(r);
    // the host answered the call and the result role-less: both were repaired, the group is there
    expect(host.asked).toEqual(expect.arrayContaining(['c2', 'c3', 'c6']));
    expect(second.s.state.entries.find((e) => e.id === 'c2')?.message?.role).toBe('assistant');
    expect(second.s.state.entries.find((e) => e.id === 'c3')?.message?.role).toBe('toolResult');
  });

  it('never rewrites the persisted snapshot from turns that still miss their stubs', async () => {
    const first = await launch({ dropRole: false, fail: new Set(), asked: [] });
    let r = render(() => <Timeline ctx={ctx(first.s)} />);
    await L.frames();
    await expectWhole(r);
    await sleep(400);
    const before = localStorage.getItem(SNAP)!;
    expect(before).toContain('Running the big script');

    relaunch();
    let release!: () => void;
    const gate = new Promise<void>((res) => (release = res));
    const second = await launch({ dropRole: false, fail: new Set(), gate, asked: [] });
    r = render(() => <Timeline ctx={ctx(second.s)} />);
    await L.frames();
    await sleep(500); // stubs are unhydrated: the live turns lack the call
    expect(second.s.state.entries.some((e) => !e.type && e.truncated)).toBe(true);
    expect(localStorage.getItem(SNAP)).toBe(before); // untouched: the stub-less turns were not written
    expect(r.getByTestId('stubs-loading').textContent).toContain('Loading the rest');
    release();
    await expectWhole(r);
  });

  it('a failing id is shown with a Retry, retried by itself first, and never fails the other ids', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'], shouldAdvanceTime: true });
    try {
      const host: Host = { dropRole: false, fail: new Set(['c2']), asked: [] };
      const { s } = await launch(host);
      const r = render(() => <Timeline ctx={ctx(s)} />);
      await L.frames();
      // c3 and c6 arrive although c2 fails: per-id errors
      await waitFor(() => expect(s.state.entries.find((e) => e.id === 'c6')?.message?.role).toBe('assistant'), { timeout: 5000 });
      expect(s.state.entries.find((e) => e.id === 'c3')?.message?.role).toBe('toolResult');
      await waitFor(() => expect(s.fullMessages!.status('c2')).toBe('error'), { timeout: 5000 });
      expect(s.fullMessages!.error('c2')).toContain('no chunk for c2');
      // its own retries run (1.5 s, 4 s, 10 s), then the notice offers Retry
      await vi.advanceTimersByTimeAsync(20_000);
      await waitFor(() => expect(r.getByTestId('stubs-failed').textContent).toContain('could not be loaded'));
      const askedBefore = host.asked.filter((id) => id === 'c2').length;
      expect(askedBefore).toBeGreaterThanOrEqual(4);
      host.fail.clear();
      fireEvent.click(r.getByTestId('stubs-retry'));
      await vi.advanceTimersByTimeAsync(50);
      await waitFor(() => expect(r.queryByTestId('stubs-failed')).toBeNull(), { timeout: 5000 });
      expect(r.container.textContent).toContain('ran 1 command');
    } finally {
      vi.useRealTimers();
    }
  });
});
