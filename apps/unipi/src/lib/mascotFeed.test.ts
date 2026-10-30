import { afterEach, describe, expect, it, vi } from 'vitest';
import { buildFeed, overlayConfig } from './mascotFeed';
import { mascotWindowSize, setMascotWindowSize } from './mascot';
import { setOverlay } from './mascotNative';
import { overlayOn } from './mascot';
import { createMascotLiveForTest } from './mascotLive';
import { needsYou, type NeedsYouItem } from './needsYou';
import { SLEEP_AFTER_MS } from './unicrab';

const item = (over: Partial<NeedsYouItem> = {}): NeedsYouItem => ({
  id: 'h:3:agent_end:9', hostId: 'h', hostName: 'W', pid: 3, session: 's', title: 'auth', kind: 'agent_end', group: 'finished', since: 9, ...over,
});

afterEach(() => {
  delete (window as { UniPiOverlay?: unknown }).UniPiOverlay;
  localStorage.clear();
  needsYou().clearAll();
});

describe('buildFeed', () => {
  it('state + count, and a bubble with the summary and the chat deep link', () => {
    expect(buildFeed('idle', 0, undefined)).toEqual({ state: 'idle', count: 0, bubble: null, size: 96 });
    expect(buildFeed('done', 2, item({ what: 'auth: all green' }))).toEqual({
      state: 'done',
      count: 2,
      bubble: { id: 'h:3:agent_end:9', text: 'auth: all green', link: 'unipi://chat?host=h&pid=3' },
      size: 96,
    });
  });
  it('UNI-255: carries the chosen crab size for the desktop window', () => {
    setMascotWindowSize(128);
    expect(buildFeed('idle', 0, undefined).size).toBe(128);
    localStorage.setItem('unipi.mascot.windowSize', '9999');
    expect(mascotWindowSize()).toBe(96);
  });
});

describe('overlayConfig', () => {
  it('only when notifications are on with a server + topic; never carries the key', () => {
    expect(overlayConfig({ enabled: false, serverUrl: 'https://ntfy.sh', topic: 't', token: '' })).toBeNull();
    expect(overlayConfig({ enabled: true, serverUrl: 'https://ntfy.sh', topic: '', token: '' })).toBeNull();
    const cfg = overlayConfig({ enabled: true, serverUrl: 'https://ntfy.example/', topic: 't', token: 'tk' });
    expect(cfg).toEqual({ serverUrl: 'https://ntfy.example', topic: 't', token: 'tk' });
    expect(JSON.stringify(cfg)).not.toContain('key');
  });
});

describe('setOverlay (SYSTEM_ALERT_WINDOW flow)', () => {
  it('unsupported without the Android bridge', () => {
    expect(setOverlay(true)).toBe('unsupported');
  });
  it('asks for "Display over other apps" first, then starts; off stops', () => {
    const bridge = { canDraw: vi.fn(() => false), requestPermission: vi.fn(), start: vi.fn(), stop: vi.fn() };
    (window as { UniPiOverlay?: unknown }).UniPiOverlay = bridge;
    expect(setOverlay(true)).toBe('needs-permission');
    expect(bridge.requestPermission).toHaveBeenCalled();
    expect(bridge.start).not.toHaveBeenCalled();
    expect(overlayOn()).toBe(false);
    bridge.canDraw.mockReturnValue(true);
    expect(setOverlay(true)).toBe('on');
    expect(bridge.start).toHaveBeenCalled();
    expect(overlayOn()).toBe(true);
    expect(setOverlay(false)).toBe('off');
    expect(bridge.stop).toHaveBeenCalled();
    expect(overlayOn()).toBe(false);
  });
});

describe('mascotLive', () => {
  it('follows the needs-you store: needs-you > done > idle, then falls asleep', () => {
    let now = 1_000_000;
    const live = createMascotLiveForTest(() => [], () => now);
    expect(live.state()).toBe('idle');
    needsYou().upsert(item());
    expect(live.state()).toBe('done');
    expect(live.count()).toBe(1);
    needsYou().upsert(item({ id: 'w', kind: 'ask_user', group: 'waiting' }));
    expect(live.state()).toBe('needs-you');
    needsYou().markAllRead();
    expect(live.state()).toBe('idle');
    now += SLEEP_AFTER_MS + 1;
    live.poke(); // a touch wakes it (and re-evaluates)
    expect(live.state()).toBe('idle');
  });
});
