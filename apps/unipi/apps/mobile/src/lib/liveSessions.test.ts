import { describe, it, expect, beforeEach } from 'vitest';
import {
  MAX_LIVE,
  activeLive,
  closeLive,
  isLive,
  liveByRecency,
  liveEntries,
  liveKey,
  liveStatus,
  openLive,
  resetLiveSessions,
  setLiveStatus,
} from './liveSessions';
import { getTermTarget, setTermTarget } from './termTarget';

const t = (session: string) => ({
  hostId: 'h',
  target: { mux: 'tmux' as const, session },
  label: session,
  sublabel: 'tmux',
});

beforeEach(() => resetLiveSessions());

describe('live sessions store', () => {
  it('opening adds an entry and makes it active; reopening resumes the same entry', () => {
    const first = openLive(t('a'));
    const entry = liveEntries()[0];
    expect(activeLive()?.key).toBe(first.key);
    openLive(t('b'));
    expect(activeLive()?.target.label).toBe('b');
    openLive(t('a'));
    expect(liveEntries()).toHaveLength(2);
    // Same object: the terminal layer never remounts a resumed screen.
    expect(liveEntries()[0]).toBe(entry);
    expect(activeLive()?.key).toBe(first.key);
  });

  it('Ongoing order is most recently used first', () => {
    openLive(t('a'));
    openLive(t('b'));
    openLive(t('a'));
    expect(liveByRecency().map((e) => e.target.label)).toEqual(['a', 'b']);
  });

  it(`caps at ${MAX_LIVE}: the least recently used background connection is closed`, () => {
    for (const name of ['a', 'b', 'c', 'd']) openLive(t(name));
    openLive(t('a')); // a is now the most recent
    const result = openLive(t('e'));
    expect(result.evicted).toEqual([liveKey(t('b'))]);
    expect(liveEntries().map((e) => e.target.label).sort()).toEqual(['a', 'c', 'd', 'e']);
  });

  it('closeLive removes the entry, clears active and status', () => {
    const { key } = openLive(t('a'));
    setLiveStatus(key, 'live');
    expect(liveStatus(key)).toBe('live');
    closeLive(key);
    expect(isLive(key)).toBe(false);
    expect(activeLive()).toBeUndefined();
    expect(liveStatus(key)).toBe('connecting');
  });

  it('termTarget is a view over the active live entry', () => {
    setTermTarget(t('a'));
    expect(getTermTarget()?.label).toBe('a');
    setTermTarget(null);
    expect(getTermTarget()).toBeNull();
    // Clearing the active target never closes the connection.
    expect(isLive(liveKey(t('a')))).toBe(true);
  });
});
