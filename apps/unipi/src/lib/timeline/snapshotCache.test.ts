// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { buildTurns } from '../session/turns';
import type { SessionEntry } from '../session/types';
import { MEM_BYTE_LIMIT, clearTimelineSnapshots, cutAtBoundary, readTimelineSnapshot, snapshotCacheStats, writeTimelineSnapshot } from './snapshotCache';

beforeEach(() => {
  localStorage.clear();
  clearTimelineSnapshots();
  vi.useFakeTimers();
});

afterEach(() => {
  clearTimelineSnapshots();
  vi.useRealTimers();
});

function sampleTurns(label: string) {
  const entries: SessionEntry[] = [
    {
      type: 'message',
      id: 'u1',
      parentId: null,
      timestamp: '2026-10-30T09:00:00Z',
      message: { role: 'user', content: [{ type: 'text', text: `Hello ${label}` }], timestamp: 1000 },
    },
    {
      type: 'message',
      id: 'a1',
      parentId: 'u1',
      timestamp: '2026-10-30T09:00:02Z',
      message: { role: 'assistant', content: [{ type: 'text', text: `Reply ${label}` }], stopReason: 'stop', timestamp: 3000 },
    },
  ] as unknown as SessionEntry[];
  return buildTurns({ entries, leafId: 'a1', entriesRev: 1, liveTools: {}, run: { running: false } });
}

describe('snapshotCache', () => {
  it('serves cached turns immediately from memory and survives reload via localStorage', () => {
    const turns = sampleTurns('alpha');
    writeTimelineSnapshot('h1/s1', turns);
    expect(readTimelineSnapshot('h1/s1')?.[0]?.segments[0]?.answer?.text).toBe('Reply alpha');

    // Flush the debounced disk persist, then simulate a cold reload by clearing memory via a new key check
    vi.advanceTimersByTime(300);
    const raw = localStorage.getItem('unipi.tl.snap.item.v2:h1/s1');
    expect(raw).toBeTruthy();
    // Re-hydrate after memory drop:
    clearTimelineSnapshots();
    localStorage.setItem('unipi.tl.snap.item.v2:h1/s1', raw!);
    localStorage.setItem('unipi.tl.snap.index.v2', JSON.stringify(['h1/s1']));
    const cold = readTimelineSnapshot('h1/s1');
    expect(cold).toHaveLength(1);
    expect(cold![0]!.segments[0]!.answer?.text).toBe('Reply alpha');
  });

  it('bounds persisted sessions to the ring limit', () => {
    for (let i = 0; i < 25; i++) {
      writeTimelineSnapshot(`h1/s${i}`, sampleTurns(String(i)));
    }
    vi.advanceTimersByTime(300);
    expect(localStorage.getItem('unipi.tl.snap.item.v2:h1/s0')).toBeNull();
    expect(localStorage.getItem('unipi.tl.snap.item.v2:h1/s24')).toBeTruthy();
  });

  it('bounds the in-memory cache by bytes, not by session count (E-25)', () => {
    const big = (label: string) => {
      const turns = sampleTurns(label);
      // ~1.5 MB of answer text per session (slimmed to the 24k answer cap, so use many turns).
      const many = Array.from({ length: 24 }, (_, i) => ({ ...turns[0]!, turnId: `${label}-${i}` }));
      for (const t of many) t.segments = t.segments.map((seg) => ({ ...seg, answer: { ...seg.answer!, text: 'x'.repeat(24_000) } }));
      return many;
    };
    for (let i = 0; i < 200; i++) {
      writeTimelineSnapshot(`h/s${i}`, big(`s${i}`));
      vi.advanceTimersByTime(300);
    }
    const stats = snapshotCacheStats();
    expect(stats.bytes).toBeLessThanOrEqual(MEM_BYTE_LIMIT);
    expect(stats.sessions).toBeLessThan(200);
    // The newest survives, the oldest was evicted (and is still on disk only if within the ring).
    expect(readTimelineSnapshot('h/s199')).toBeTruthy();
  });

  it('keeps no inline image data in the in-memory copy once settled', () => {
    const turns = sampleTurns('img');
    const user = turns[0]!.user!;
    (user.message as { content: unknown }).content = [{ type: 'image', data: 'A'.repeat(100_000), mimeType: 'image/png' }];
    writeTimelineSnapshot('h/img', turns);
    vi.advanceTimersByTime(300);
    expect(JSON.stringify(readTimelineSnapshot('h/img'))).not.toContain('AAAAAAAAAA');
  });
});

describe('w5-ui D10: a cached long answer is a clean excerpt', () => {
  const paragraphs = Array.from({ length: 400 }, (_, i) => `Paragraph ${i + 1}. Seeded prose for the warm-launch excerpt, long enough to wrap.`).join('\n\n');

  it('cuts at a paragraph boundary, else a line, else a word — never mid-word', () => {
    const cut = cutAtBoundary(paragraphs, 24_000);
    expect(cut.length).toBeLessThanOrEqual(24_000);
    expect(paragraphs.startsWith(cut)).toBe(true);
    expect(paragraphs.slice(cut.length, cut.length + 2)).toBe('\n\n'); // ends right before a paragraph break
    const words = 'lorem ipsum dolor '.repeat(2000);
    const w = cutAtBoundary(words, 1000);
    expect(w.endsWith('lorem') || w.endsWith('ipsum') || w.endsWith('dolor')).toBe(true);
    expect(cutAtBoundary('short', 100)).toBe('short');
    expect(cutAtBoundary('x'.repeat(500), 100)).toBe('x'.repeat(100)); // no boundary at all: the old hard cut
  });

  it('the stored answer is the clean excerpt and says it is cut; a short answer is whole and unflagged', () => {
    const turns = sampleTurns('long');
    turns[0]!.segments[0]!.answer!.text = paragraphs;
    writeTimelineSnapshot('h/long', turns);
    vi.advanceTimersByTime(300);
    const answer = readTimelineSnapshot('h/long')![0]!.segments[0]!.answer!;
    expect(answer.cut).toBe(true);
    expect(answer.text.length).toBeLessThanOrEqual(24_000);
    expect(/Paragraph \d+\.[^\n]*$/.test(answer.text) && answer.text.endsWith('wrap.')).toBe(true);
    writeTimelineSnapshot('h/short', sampleTurns('s'));
    vi.advanceTimersByTime(300);
    expect(readTimelineSnapshot('h/short')![0]!.segments[0]!.answer!.cut).toBeUndefined();
  });
});

describe('w7-ui G3-1: a snapshot is only ever taken from hydrated content', () => {
  const toolTurns = (command: string) => {
    const entries = [
      { type: 'message', id: 'u1', parentId: null, timestamp: '2026-10-30T09:00:00Z', message: { role: 'user', content: [{ type: 'text', text: 'run it' }], timestamp: 1000 } },
      { type: 'message', id: 'a1', parentId: 'u1', timestamp: '2026-10-30T09:00:02Z', message: { role: 'assistant', content: [{ type: 'text', text: 'Running.' }, { type: 'toolCall', id: 'c1', name: 'bash', arguments: { command } }], stopReason: 'toolUse', timestamp: 3000 } },
      { type: 'message', id: 'r1', parentId: 'a1', timestamp: '2026-10-30T09:00:03Z', message: { role: 'toolResult', toolCallId: 'c1', toolName: 'bash', content: [{ type: 'text', text: 'ok' }], isError: false, timestamp: 4000 } },
      { type: 'message', id: 'a2', parentId: 'r1', timestamp: '2026-10-30T09:00:04Z', message: { role: 'assistant', content: [{ type: 'text', text: 'Done.' }], stopReason: 'stop', timestamp: 5000 } },
    ] as unknown as SessionEntry[];
    return buildTurns({ entries, leafId: 'a2', entriesRev: 1, liveTools: {}, run: { running: false } });
  };

  it('records nothing while stubs are unhydrated (an incomplete turn list never restores as the chat)', () => {
    writeTimelineSnapshot('h1/stub', toolTurns('echo hi'), false);
    vi.advanceTimersByTime(300);
    expect(readTimelineSnapshot('h1/stub')).toBeUndefined();
    expect(localStorage.getItem('unipi.tl.snap.item.v2:h1/stub')).toBeNull();
    writeTimelineSnapshot('h1/stub', toolTurns('echo hi'), true);
    vi.advanceTimersByTime(300);
    expect(localStorage.getItem('unipi.tl.snap.item.v2:h1/stub')).toBeTruthy();
  });

  it('survives a simulated relaunch with the tool call of a huge command, cut to a bounded excerpt', () => {
    writeTimelineSnapshot('h1/huge', toolTurns(`echo start\n${'echo "arg line lorem ipsum"\n'.repeat(60_000)}`));
    vi.advanceTimersByTime(300);
    const raw = localStorage.getItem('unipi.tl.snap.item.v2:h1/huge')!;
    expect(raw.length).toBeLessThan(100_000);
    clearTimelineSnapshots();
    localStorage.setItem('unipi.tl.snap.item.v2:h1/huge', raw);
    const seg = readTimelineSnapshot('h1/huge')![0]!.segments[0]!;
    const tool = [...seg.work, ...(seg.after ?? [])].find((w) => w.kind === 'tool') as unknown as { kind: string; tool: { args: { command: string } } };
    expect(tool.kind).toBe('tool');
    expect(tool.tool.args.command.length).toBeLessThan(8_000);
  });

  it('drops the old persisted copy when a newer one cannot be stored (quota), instead of restoring a stale one', () => {
    writeTimelineSnapshot('h1/q', sampleTurns('old'));
    vi.advanceTimersByTime(300);
    expect(localStorage.getItem('unipi.tl.snap.item.v2:h1/q')).toBeTruthy();
    const set = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new DOMException('full', 'QuotaExceededError');
    });
    writeTimelineSnapshot('h1/q', sampleTurns('new'));
    vi.advanceTimersByTime(300);
    set.mockRestore();
    expect(localStorage.getItem('unipi.tl.snap.item.v2:h1/q')).toBeNull();
  });

  it('never persists an oversized snapshot and removes the older copy of that chat', () => {
    writeTimelineSnapshot('h1/big', sampleTurns('old'));
    vi.advanceTimersByTime(300);
    const entries = Array.from({ length: 40 }, (_, i) => ({ type: 'message', id: `u${i}`, parentId: i ? `a${i - 1}` : null, timestamp: '2026-10-30T09:00:00Z', message: { role: 'user', content: Array.from({ length: 900 }, (_, k) => ({ type: 'text', text: `block ${i}/${k}` })), timestamp: 1000 + i } }))
      .flatMap((u, i) => [u, { type: 'message', id: `a${i}`, parentId: u.id, timestamp: '2026-10-30T09:00:02Z', message: { role: 'assistant', content: [{ type: 'text', text: 'x' }], stopReason: 'stop', timestamp: 2000 + i } }]) as unknown as SessionEntry[];
    writeTimelineSnapshot('h1/big', buildTurns({ entries, leafId: 'a39', entriesRev: 1, liveTools: {}, run: { running: false } }));
    vi.advanceTimersByTime(300);
    expect(localStorage.getItem('unipi.tl.snap.item.v2:h1/big')).toBeNull();
  });

  it('removes the v1 snapshots (taken while stubs were unhydrated) instead of restoring them', async () => {
    localStorage.setItem('unipi.tl.snap.item.v1:h1/old', JSON.stringify(sampleTurns('stale')));
    localStorage.setItem('unipi.tl.snap.index.v1', JSON.stringify(['h1/old']));
    // the purge runs once per page load: reload the module
    vi.resetModules();
    const fresh = await import('./snapshotCache');
    expect(fresh.readTimelineSnapshot('h1/old')).toBeUndefined();
    expect(localStorage.getItem('unipi.tl.snap.item.v1:h1/old')).toBeNull();
    expect(localStorage.getItem('unipi.tl.snap.index.v1')).toBeNull();
  });
});
