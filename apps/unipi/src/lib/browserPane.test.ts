import { afterEach, describe, expect, it } from 'vitest';
import {
  agentStrip,
  browserPane,
  canPickProfile,
  clampPaneWidth,
  PANE_CHAT_MIN,
  PANE_DEFAULT,
  PANE_MIN,
  profileChoices,
  resetBrowserPane,
  stealthBadge,
  takeover,
  takeoverError,
  toggleBrowser,
  type TakeoverState,
} from './browserPane';

const agent = (connected: boolean, driving: boolean, taken_over = false) => ({ connected, driving, taken_over });

describe('agent strip + take over (UNI-296)', () => {
  it('shows nothing until an agent drives; taken over wins', () => {
    expect(agentStrip(undefined)).toEqual({ kind: 'none' });
    expect(agentStrip(agent(true, false))).toEqual({ kind: 'none' });
    expect(agentStrip(agent(true, true))).toEqual({ kind: 'driving' });
    expect(agentStrip(agent(true, true, true))).toEqual({ kind: 'taken' });
    expect(agentStrip(agent(false, false, true))).toEqual({ kind: 'taken' });
  });

  it('agent driving → take over → (agent waits) → give back', () => {
    let s: TakeoverState = { agent: undefined };
    s = takeover(s, { kind: 'agent', agent: agent(true, true) });
    expect(agentStrip(s.agent).kind).toBe('driving');
    s = takeover(s, { kind: 'request', on: true });
    expect(s.pending).toBe('take');
    expect(agentStrip(s.agent).kind).toBe('driving');
    s = takeover(s, { kind: 'ok', agent: agent(true, false, true) });
    expect(s).toEqual({ agent: agent(true, false, true) });
    expect(agentStrip(s.agent).kind).toBe('taken');
    // Events while held keep it taken (the PC refuses the agent's input).
    s = takeover(s, { kind: 'agent', agent: agent(true, false, true) });
    expect(agentStrip(s.agent).kind).toBe('taken');
    s = takeover(s, { kind: 'request', on: false });
    expect(s.pending).toBe('give');
    s = takeover(s, { kind: 'ok', agent: agent(true, false, false) });
    expect(agentStrip(s.agent).kind).toBe('none');
  });

  it('the PC ending a take-over by itself (10 min idle) clears the strip', () => {
    let s: TakeoverState = { agent: agent(true, false, true) };
    s = takeover(s, { kind: 'agent', agent: agent(true, true, false) });
    expect(agentStrip(s.agent).kind).toBe('driving');
  });

  it('a failed request keeps the state and says why in plain words', () => {
    let s: TakeoverState = { agent: agent(true, true) };
    s = takeover(s, { kind: 'request', on: true });
    s = takeover(s, { kind: 'failed', error: { code: 'E_SCHEMA', message: 'unknown variant', retryable: false } });
    expect(s.pending).toBeUndefined();
    expect(s.error).toBe('Update unipi-host on the PC to take over from the agent.');
    expect(agentStrip(s.agent).kind).toBe('driving');
    expect(takeoverError({ code: 'E_TRANSPORT', message: 'Could not reach coffee', retryable: true })).toBe('Could not reach coffee');
    // A new request clears the old error.
    expect(takeover(s, { kind: 'request', on: true }).error).toBeUndefined();
  });
});

describe('stealth badge (UNI-296)', () => {
  it('stealthy CloakBrowser', () => {
    expect(stealthBadge({ engine: 'cloak', stealth: true })).toMatchObject({ tone: 'ok', label: 'Stealth' });
    expect(stealthBadge({ engine: 'cloak', stealth: true })?.note).toBeUndefined();
  });
  it('Chrome fallback names the reason', () => {
    const b = stealthBadge({ engine: 'chrome', stealth: false, stealth_reason: 'CloakBrowser failed to start: exited before DevTools came up' })!;
    expect(b.tone).toBe('warn');
    expect(b.label).toBe('Not stealthy');
    expect(b.note).toBe('Not stealthy — Chrome fallback: CloakBrowser failed to start: exited before DevTools came up');
  });
  it('chosen Chrome and a missing reason read plainly', () => {
    expect(stealthBadge({ engine: 'chrome', stealth: false, stealth_reason: 'the engine is set to chrome' })?.note).toBe('Not stealthy — plain Chrome is set on the PC');
    expect(stealthBadge({ stealth: false })?.note).toBe('Not stealthy — Chrome fallback: CloakBrowser could not start');
  });
  it('an older PC that does not say: no badge', () => {
    expect(stealthBadge({})).toBeUndefined();
    expect(stealthBadge(undefined)).toBeUndefined();
  });
});

describe('profile picker (UNI-296)', () => {
  const data = {
    default: 'default',
    profiles: [
      { name: 'shop', created: 2, running: false, cookie_domains: ['a.com', 'b.com'] },
      { name: 'default', created: 1, running: true, cookie_domains: [] },
      { name: 'bank', created: 3, running: false, cookie_domains: ['bank.example'] },
      { name: 'empty', created: 4, running: false, cookie_domains: [] },
    ],
  };
  it('default first, then by name; current marked; hints', () => {
    const rows = profileChoices(data, 'shop');
    expect(rows.map((r) => r.name)).toEqual(['default', 'bank', 'empty', 'shop']);
    expect(rows.find((r) => r.current)?.name).toBe('shop');
    expect(rows[0]!.hint).toBe('running');
    expect(rows[1]!.hint).toBe('1 site signed in');
    expect(rows[2]!.hint).toBeUndefined();
    expect(rows[3]!.hint).toBe('2 sites signed in');
    expect(canPickProfile(data)).toBe(true);
  });
  it('an unknown or older PC: just the current one, no picking', () => {
    expect(profileChoices(undefined, 'default')).toEqual([{ name: 'default', current: true, running: false }]);
    expect(canPickProfile(undefined)).toBe(false);
    expect(canPickProfile({ default: 'default', profiles: [{ name: 'default', created: 1, running: false, cookie_domains: [] }] })).toBe(false);
  });
});

describe('pane state (UNI-296)', () => {
  afterEach(() => {
    localStorage.clear();
    resetBrowserPane();
  });
  it('clamps the width so the chat keeps room', () => {
    expect(clampPaneWidth(100, 1200)).toBe(PANE_MIN);
    expect(clampPaneWidth(2000, 1200)).toBe(1200 - PANE_CHAT_MIN);
    expect(clampPaneWidth(Number.NaN, 1600)).toBe(PANE_DEFAULT);
    expect(clampPaneWidth(500, 500)).toBe(PANE_MIN);
  });
  it('remembers open + width + per-PC profile; the phone sheet is not remembered', () => {
    toggleBrowser(true);
    browserPane().setWidth(640);
    browserPane().setProfileFor('coffee', 'shop');
    toggleBrowser(false);
    expect(browserPane().sheet()).toBe(true);
    resetBrowserPane();
    expect(browserPane().open()).toBe(true);
    expect(browserPane().width()).toBe(640);
    expect(browserPane().profileFor('coffee')).toBe('shop');
    expect(browserPane().profileFor('laptop')).toBe('default');
    expect(browserPane().sheet()).toBe(false);
    toggleBrowser(true);
    expect(browserPane().open()).toBe(false);
  });
});

describe('folding the chat list (UNI-296)', () => {
  it('folds when list + chat + pane do not fit', async () => {
    const { foldsList } = await import('./browserPane');
    // 1440 wide, list ends at 624: 816 left < 380 + 560.
    expect(foldsList(1440, 624, 560)).toBe(true);
    // 1920 wide: 1296 left ≥ 940.
    expect(foldsList(1920, 624, 560)).toBe(false);
    // A narrow pane fits beside the list on a 1440 screen.
    expect(foldsList(1440, 624, 400)).toBe(false);
    expect(foldsList(1440, 624, 100)).toBe(false);
  });
});

describe('stealthBadge: headless CloakBrowser (UNI-299)', () => {
  it('says less stealthy with the host note', () => {
    const b = stealthBadge({ engine: 'cloak', stealth: true, stealth_note: 'Less stealthy: no hidden display on this PC (install Xvfb)' });
    expect(b?.tone).toBe('warn');
    expect(b?.label).toBe('Less stealthy');
    expect(b?.note).toContain('install Xvfb');
  });
});

describe('WP-B8: following the agent, picker geometry', () => {
  const agent = { connected: true, driving: false, taken_over: false };
  it('shows a tab the agent opened, never on the first list or the user’s own tab', async () => {
    const { followTab, USER_TAB_WINDOW_MS } = await import('./browserPane');
    const known = new Set(['T1']);
    const tabs = [{ id: 'T1' }, { id: 'T2' }];
    expect(followTab(known, tabs, 'T1', agent, 10_000)).toBe('T2');
    expect(followTab(new Set(), tabs, 'T1', agent, 10_000)).toBeUndefined();
    expect(followTab(known, tabs, 'T1', agent, USER_TAB_WINDOW_MS - 1)).toBeUndefined();
    expect(followTab(known, tabs, 'T1', { ...agent, connected: false }, 10_000)).toBeUndefined();
    expect(followTab(known, tabs, 'T1', { ...agent, taken_over: true }, 10_000)).toBeUndefined();
    expect(followTab(known, tabs, 'T2', agent, 10_000)).toBeUndefined();
    expect(followTab(known, [{ id: 'T1' }], 'T1', agent, 10_000)).toBeUndefined();
  });

  it('places the hover box over the letterboxed picture, clipped to it', async () => {
    const { overlayRect } = await import('./browserPane');
    // 800×600 frame in a 400×400 box: scale .5, 50 px bars top and bottom.
    expect(overlayRect({ x: 100, y: 100, width: 200, height: 40 }, { width: 400, height: 400 }, { width: 800, height: 600 })).toEqual({ x: 50, y: 100, width: 100, height: 20 });
    const clipped = overlayRect({ x: -50, y: 590, width: 100, height: 40 }, { width: 400, height: 400 }, { width: 800, height: 600 });
    expect(clipped).toEqual({ x: 0, y: 345, width: 25, height: 5 });
    expect(overlayRect({ x: 0, y: 0, width: 1, height: 1 }, { width: 10, height: 10 }, { width: 0, height: 0 }).width).toBe(0);
  });

  it('crops the element from the frame image (padded, clamped, scaled to ≤ 1600 px)', async () => {
    const { cropRect, SHOT_MAX_SIDE } = await import('./browserPane');
    // 2× DPR frame image.
    expect(cropRect({ x: 10, y: 20, width: 100, height: 50 }, { width: 800, height: 600 }, { width: 1600, height: 1200 })).toEqual({ sx: 12, sy: 32, sw: 216, sh: 116, outWidth: 216, outHeight: 116 });
    expect(cropRect(undefined, { width: 1280, height: 800 }, { width: 2560, height: 1600 })).toMatchObject({ sx: 0, sy: 0, sw: 2560, sh: 1600, outWidth: SHOT_MAX_SIDE, outHeight: 1000 });
    expect(cropRect({ x: 900, y: 0, width: 10, height: 10 }, { width: 800, height: 600 }, { width: 800, height: 600 })).toBeUndefined();
  });
});
