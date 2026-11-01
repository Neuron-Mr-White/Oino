// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { buildTurns } from '../session/turns';
import type { SessionEntry } from '../session/types';
import { MEM_BYTE_LIMIT, clearTimelineSnapshots, readTimelineSnapshot, snapshotCacheStats, writeTimelineSnapshot } from './snapshotCache';

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
    const raw = localStorage.getItem('unipi.tl.snap.item.v1:h1/s1');
    expect(raw).toBeTruthy();
    // Re-hydrate after memory drop:
    clearTimelineSnapshots();
    localStorage.setItem('unipi.tl.snap.item.v1:h1/s1', raw!);
    localStorage.setItem('unipi.tl.snap.index.v1', JSON.stringify(['h1/s1']));
    const cold = readTimelineSnapshot('h1/s1');
    expect(cold).toHaveLength(1);
    expect(cold![0]!.segments[0]!.answer?.text).toBe('Reply alpha');
  });

  it('bounds persisted sessions to the ring limit', () => {
    for (let i = 0; i < 25; i++) {
      writeTimelineSnapshot(`h1/s${i}`, sampleTurns(String(i)));
    }
    vi.advanceTimersByTime(300);
    expect(localStorage.getItem('unipi.tl.snap.item.v1:h1/s0')).toBeNull();
    expect(localStorage.getItem('unipi.tl.snap.item.v1:h1/s24')).toBeTruthy();
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
