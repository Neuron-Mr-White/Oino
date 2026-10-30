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
