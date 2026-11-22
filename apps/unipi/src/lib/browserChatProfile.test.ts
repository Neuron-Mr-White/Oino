import { describe, expect, it } from 'vitest';
import { latestBrowserResult } from './browserChatProfile';
import { effectiveProfile, profileChoices } from './browserPane';
import type { SessionEntry } from './session/types';

const entry = (id: string, toolName: string, profile?: string): SessionEntry => ({
  type: 'message',
  id,
  message: { role: 'toolResult', toolCallId: id, toolName, details: profile ? { browser: { profile } } : {} },
});

describe('latestBrowserResult', () => {
  it('takes the newest browser_* tool result that names a profile', () => {
    const entries = [entry('a', 'browser_open', 'default'), entry('b', 'bash'), entry('c', 'browser_click', 'ace-jobs'), entry('d', 'browser_click')];
    expect(latestBrowserResult(entries, {})).toEqual({ id: 'c', profile: 'ace-jobs' });
  });
  it('a finished live tool is newer than the timeline', () => {
    const live = { l: { id: 'l', name: 'browser_open', args: {}, startedAt: 1, done: true, result: { details: { browser: { profile: 'shop' } } } } };
    expect(latestBrowserResult([entry('a', 'browser_open', 'default')], live)).toEqual({ id: 'l', profile: 'shop' });
    expect(latestBrowserResult([], { l: { ...live.l, done: false } })).toBeUndefined();
  });
  it('none when the chat never used the browser', () => {
    expect(latestBrowserResult([entry('a', 'bash')], {})).toBeUndefined();
  });
});

describe('effectiveProfile', () => {
  it('chat pick while the agent is unchanged, else the agent, else the PC choice', () => {
    expect(effectiveProfile({ profile: 'shop', seen: 'ace-jobs' }, 'ace-jobs', 'default')).toBe('shop');
    expect(effectiveProfile({ profile: 'shop', seen: 'ace-jobs' }, 'default', 'x')).toBe('default');
    expect(effectiveProfile(undefined, 'ace-jobs', 'default')).toBe('ace-jobs');
    expect(effectiveProfile(undefined, undefined, 'shop')).toBe('shop');
  });
});

describe('profileChoices', () => {
  it('tags the agent’s profile, adding it when the PC list lacks it', () => {
    const rows = profileChoices({ default: 'default', profiles: [{ name: 'default', created: 1, running: false, cookie_domains: [] }] }, 'default', 'ace-jobs');
    expect(rows.map((r) => [r.name, !!r.agent, r.current])).toEqual([['default', false, true], ['ace-jobs', true, false]]);
  });
});
