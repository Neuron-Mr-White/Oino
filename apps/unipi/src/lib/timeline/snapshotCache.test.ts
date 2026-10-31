// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { buildTurns } from '../session/turns';
import type { SessionEntry } from '../session/types';
import { clearTimelineSnapshots, readTimelineSnapshot, writeTimelineSnapshot } from './snapshotCache';

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
});
