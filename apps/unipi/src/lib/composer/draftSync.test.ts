import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { clearHostDraft, loadHostDraft, resetDraftSync, scheduleDraftSync } from './draftSync';
import { composerDraft, resetComposerDrafts, setComposerText } from './drafts';

const request = vi.hoisted(() => vi.fn());
vi.mock('../host/factory', () => ({ hostClient: () => ({ request }), setHostClientFactory: vi.fn() }));
const writes = () => request.mock.calls.filter(([op]) => op === 'draft_set');

beforeEach(() => {
  vi.useFakeTimers(); resetDraftSync(); resetComposerDrafts();
  request.mockReset(); request.mockResolvedValue({ draft: null });
});
afterEach(() => { resetDraftSync(); vi.useRealTimers(); });

describe('draft coalescing', () => {
  it('waits one second after the last change and skips identical documents', async () => {
    await loadHostDraft('h/s');
    setComposerText('h/s', 'one'); scheduleDraftSync('h/s');
    await vi.advanceTimersByTimeAsync(900);
    expect(writes()).toHaveLength(0);
    setComposerText('h/s', 'two'); scheduleDraftSync('h/s');
    await vi.advanceTimersByTimeAsync(999);
    expect(writes()).toHaveLength(0);
    await vi.advanceTimersByTimeAsync(1);
    expect(writes()).toHaveLength(1);
    expect(writes()[0]![1].draft.text).toBe('two');
    scheduleDraftSync('h/s'); await vi.advanceTimersByTimeAsync(1000);
    expect(writes()).toHaveLength(1);
  });
  it('coalesces independently per session', async () => {
    await loadHostDraft('h/a'); await loadHostDraft('h/b');
    setComposerText('h/a', 'a'); scheduleDraftSync('h/a');
    setComposerText('h/b', 'b'); scheduleDraftSync('h/b');
    await vi.advanceTimersByTimeAsync(1000);
    expect(writes()).toHaveLength(2);
  });
  it('orders a clear after an in-flight write and cancels pending saves', async () => {
    await loadHostDraft('h/s');
    let finish!: () => void;
    request.mockImplementation((op) => op === 'draft_set' ? new Promise<void>((r) => { finish = r; }) : Promise.resolve({}));
    setComposerText('h/s', 'one'); scheduleDraftSync('h/s');
    await vi.advanceTimersByTimeAsync(1000);
    clearHostDraft('h/s');
    expect(writes()).toHaveLength(1);
    request.mockResolvedValue({}); finish();
    await vi.advanceTimersByTimeAsync(0);
    expect(writes()).toHaveLength(2);
    expect(writes()[1]![1].draft).toBeNull();
    clearHostDraft('h/s'); await vi.advanceTimersByTimeAsync(1000);
    expect(writes()).toHaveLength(2);
  });
});

describe('opening a chat while typing (r3 D13)', () => {
  it('text typed before the host\'s draft arrives wins: the draft is not applied over it (no "cold-typedcold-typed")', async () => {
    let answer!: (v: unknown) => void;
    request.mockImplementation((op) => op === 'draft_get' ? new Promise((r) => { answer = r; }) : Promise.resolve({}));
    const loading = loadHostDraft('h/s');
    setComposerText('h/s', 'cold-typed'); // typed while the chat is still opening
    answer({ draft: { v: 1, text: 'cold-typed', quotes: [], chips: [], attachments: [] } });
    await loading;
    expect(composerDraft('h/s').text).toBe('cold-typed');
    // …and it is saved up, replacing the host's older copy.
    await vi.advanceTimersByTimeAsync(1000);
    expect(writes()).toHaveLength(1);
    expect(writes()[0]![1].draft.text).toBe('cold-typed');
  });
  it('a draft that arrives first is the field\'s content; what is typed after it extends it', async () => {
    request.mockResolvedValue({ draft: { v: 1, text: 'saved', quotes: [], chips: [], attachments: [] } });
    await loadHostDraft('h/s');
    expect(composerDraft('h/s').text).toBe('saved');
  });
});
