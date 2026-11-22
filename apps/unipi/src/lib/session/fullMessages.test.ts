import { createRoot } from 'solid-js';
import { createStore, produce } from 'solid-js/store';
import { describe, expect, it, vi } from 'vitest';
import { createFullMessages, FETCH_TIMEOUT_MS, isStub, needsFullMessage, stripTruncationNote, STUB_RETRY_MS, TRUNCATION_NOTE } from './fullMessages';
import type { SessionEntry } from './types';

const cut = (id: string, role = 'assistant'): SessionEntry => ({
  type: 'message',
  id,
  parentId: null,
  message: { role, content: [{ type: 'text', text: `head of ${id}${TRUNCATION_NOTE}` }], truncated: true },
});
const whole = (id: string, role = 'assistant'): SessionEntry => ({
  type: 'message',
  id,
  parentId: null,
  message: { role, content: [{ type: 'text', text: `whole ${id}` }] },
});

describe('stripTruncationNote', () => {
  it('removes only the host note at the end, keeping the head (even mid inline-code)', () => {
    expect(stripTruncationNote(`use \`foo${TRUNCATION_NOTE}`)).toBe('use `foo');
    expect(stripTruncationNote('plain text')).toBe('plain text');
    expect(stripTruncationNote('a […] b]')).toBe('a […] b]');
  });
});

describe('needsFullMessage', () => {
  it('is true for cut prose and custom messages, false for tool output, bash, provisional and whole entries', () => {
    expect(needsFullMessage({ id: 'stub', truncated: true } as unknown as SessionEntry)).toBe(true);
    expect(needsFullMessage(cut('a'))).toBe(true);
    expect(needsFullMessage(cut('u', 'user'))).toBe(true);
    expect(needsFullMessage({ type: 'custom_message', id: 'c', truncated: true })).toBe(true);
    expect(needsFullMessage(cut('t', 'toolResult'))).toBe(false);
    expect(needsFullMessage(cut('b', 'bashExecution'))).toBe(false);
    expect(needsFullMessage(cut('live:1'))).toBe(false);
    expect(needsFullMessage(whole('w'))).toBe(false);
    expect(needsFullMessage(undefined)).toBe(false);
  });
});

function harness(entries: SessionEntry[], getEntries: (ids: readonly string[]) => Promise<SessionEntry[] | { entries: SessionEntry[]; errors: Record<string, string> }>) {
  const [state, setState] = createStore<{ entries: SessionEntry[] }>({ entries });
  const api = createRoot(() =>
    createFullMessages({
      entry: (id) => state.entries.find((e) => e.id === id),
      getEntries,
      apply: (full) =>
        setState(
          produce((s) => {
            for (const e of full) {
              const at = s.entries.findIndex((x) => x.id === e.id);
              if (at >= 0) s.entries[at] = e;
            }
          }),
        ),
    }),
  );
  return { state, api };
}

describe('createFullMessages', () => {
  it('batches ids asked in one tick into one request and swaps in the full entries', async () => {
    const getEntries = vi.fn(async (ids: readonly string[]) => ids.map((id) => whole(id)));
    const { state, api } = harness([cut('a'), cut('b'), cut('c')], getEntries);
    api.ensure(['a']);
    api.ensure(['b', 'c']);
    api.ensure(['a']); // already asked
    expect(api.status('a')).toBe('loading');
    await vi.waitFor(() => expect(api.status('a')).toBeUndefined());
    expect(getEntries).toHaveBeenCalledTimes(1);
    expect(getEntries).toHaveBeenCalledWith(['a', 'b', 'c']);
    expect(state.entries.map((e) => needsFullMessage(e))).toEqual([false, false, false]);
    expect(state.entries[1]!.message!.content).toEqual([{ type: 'text', text: 'whole b' }]);
  });

  it('never asks for tool output or an entry that is already whole', async () => {
    const getEntries = vi.fn(async () => []);
    const { api } = harness([cut('t', 'toolResult'), whole('w')], getEntries);
    api.ensure(['t', 'w', 'missing']);
    await Promise.resolve();
    await Promise.resolve();
    expect(getEntries).not.toHaveBeenCalled();
  });

  it('flags a failure as error and retries only on request', async () => {
    const getEntries = vi.fn<(ids: readonly string[]) => Promise<SessionEntry[]>>().mockRejectedValueOnce(new Error('offline'));
    const { state, api } = harness([cut('a')], getEntries);
    api.ensure(['a']);
    await vi.waitFor(() => expect(api.status('a')).toBe('error'));
    api.ensure(['a']); // a re-render does not hammer a failing host
    await Promise.resolve();
    expect(getEntries).toHaveBeenCalledTimes(1);
    getEntries.mockResolvedValueOnce([whole('a')]);
    api.retry(['a']);
    expect(api.status('a')).toBe('loading');
    await vi.waitFor(() => expect(api.status('a')).toBeUndefined());
    expect(getEntries).toHaveBeenCalledTimes(2);
    expect(needsFullMessage(state.entries[0])).toBe(false);
  });

  it('re-applies a remembered full entry when a reconcile brings the cut copy back, without asking again', async () => {
    const getEntries = vi.fn(async (ids: readonly string[]) => ids.map((id) => whole(id)));
    const { state, api } = harness([cut('a')], getEntries);
    api.ensure(['a']);
    await vi.waitFor(() => expect(api.status('a')).toBeUndefined());
    // The cut copy returns (page reload).
    state.entries[0] = cut('a');
    api.ensure(['a']);
    expect(needsFullMessage(state.entries[0])).toBe(false);
    expect(getEntries).toHaveBeenCalledTimes(1);
  });
});

describe('w7-ui G3-1: errors are per id, reasoned and retryable', () => {
  const stub = (id: string) => ({ id, truncated: true }) as unknown as SessionEntry;

  it('names the failing id and leaves the others of the batch applied', async () => {
    const getEntries = vi.fn(async () => ({ entries: [whole('b')], errors: { a: 'chunk lost' } }));
    const { state, api } = harness([cut('a'), cut('b')], getEntries);
    api.ensure(['a', 'b']);
    await vi.waitFor(() => expect(api.status('a')).toBe('error'));
    expect(api.error('a')).toBe('chunk lost');
    expect(api.status('b')).toBeUndefined();
    expect(needsFullMessage(state.entries[1])).toBe(false);
    api.retry(['a']);
    expect(api.status('a')).toBe('loading');
    expect(api.error('a')).toBeUndefined();
  });

  it('a stub asks again by itself with growing waits, then waits for the user; a cut message does not', async () => {
    vi.useFakeTimers();
    try {
      const getEntries = vi.fn<(ids: readonly string[]) => Promise<SessionEntry[]>>().mockRejectedValue(new Error('offline'));
      const { api } = harness([stub('s'), cut('m')], getEntries);
      api.ensure(['s', 'm']);
      await vi.advanceTimersByTimeAsync(0);
      expect(getEntries).toHaveBeenCalledTimes(1);
      for (const [i, wait] of STUB_RETRY_MS.entries()) {
        await vi.advanceTimersByTimeAsync(wait - 1);
        expect(getEntries).toHaveBeenCalledTimes(1 + i);
        await vi.advanceTimersByTimeAsync(1);
        expect(getEntries).toHaveBeenCalledTimes(2 + i);
        expect(getEntries).toHaveBeenLastCalledWith(['s']);
      }
      await vi.advanceTimersByTimeAsync(60_000);
      expect(getEntries).toHaveBeenCalledTimes(1 + STUB_RETRY_MS.length);
      expect(api.status('s')).toBe('error');
      expect(api.status('m')).toBe('error');
      // the user's retry starts a fresh round
      getEntries.mockResolvedValue([whole('s')]);
      api.retry(['s']);
      await vi.advanceTimersByTimeAsync(0);
      expect(api.status('s')).toBeUndefined();
    } finally {
      vi.useRealTimers();
    }
  });

  it('a request that never answers ends in error instead of loading for good', async () => {
    vi.useFakeTimers();
    try {
      const getEntries = vi.fn(() => new Promise<SessionEntry[]>(() => {}));
      const { api } = harness([cut('a')], getEntries);
      api.ensure(['a']);
      await vi.advanceTimersByTimeAsync(0);
      expect(api.status('a')).toBe('loading');
      await vi.advanceTimersByTimeAsync(FETCH_TIMEOUT_MS + 1);
      expect(api.status('a')).toBe('error');
      expect(api.error('a')).toContain('did not answer');
    } finally {
      vi.useRealTimers();
    }
  });

  it('a reply that is still a stub never replaces what is on screen', async () => {
    const getEntries = vi.fn(async () => [stub('a')]);
    const { state, api } = harness([cut('a')], getEntries);
    api.ensure(['a']);
    await vi.waitFor(() => expect(api.status('a')).toBe('error'));
    expect(state.entries[0]).toEqual(cut('a'));
    expect(isStub(state.entries[0])).toBe(false);
  });

  it('dispose stops the pending automatic retries', async () => {
    vi.useFakeTimers();
    try {
      const getEntries = vi.fn<(ids: readonly string[]) => Promise<SessionEntry[]>>().mockRejectedValue(new Error('offline'));
      const { api } = harness([stub('s')], getEntries);
      api.ensure(['s']);
      await vi.advanceTimersByTimeAsync(0);
      api.dispose();
      await vi.advanceTimersByTimeAsync(60_000);
      expect(getEntries).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });
});
