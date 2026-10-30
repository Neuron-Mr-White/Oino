import { describe, expect, it } from 'vitest';
import { bareTarget, cardTarget, chatGroups, effectiveWaiting, liveStatus, needsYouFirst, waitingTargets, withLive, type ChatTarget } from './list';
import type { SessionListData } from './link';

const data: SessionListData = {
  sessions: [
    { id: 'a', title: 'Fix the parser', cwd: '/home/u/proj', state: 'working' },
    { id: 'b', cwd: '/home/u/other', state: 'idle', needs_you: { kind: 'ask_user', title: 'Which branch?', since: 5 } },
    { id: 'c', title: 'Docs', cwd: '/home/u/proj', state: 'idle', pending_work: 'bg task: build', lh_mode: 'graph' },
  ],
};

describe('session list (WP-A1 placeholder over sessions_list)', () => {
  it('groups sessions by project folder, host order kept', () => {
    const groups = chatGroups('h1', data);
    expect(groups.map((g) => [g.label, g.items.map((t) => t.session)])).toEqual([
      ['proj', ['a', 'c']],
      ['other', ['b']],
    ]);
    expect(groups[0].status).toBe('working');
    expect(groups[1].status).toBe('idle');
  });

  it('a card becomes a row: title falls back to the folder, cwd shortened for display', () => {
    const row = cardTarget('h1', data.sessions[1]);
    expect(row).toMatchObject({ hostId: 'h1', session: 'b', title: 'other', cwd: '~/other', cwdFull: '/home/u/other', status: 'idle' });
    expect(row.waiting).toEqual({ kind: 'ask_user', title: 'Which branch?', since: 5 });
    expect(cardTarget('h1', data.sessions[2])).toMatchObject({ pendingWork: 'bg task: build', lhMode: 'graph' });
  });

  it('a finished-run mark is not a finish while wake-capable work is pending', () => {
    expect(effectiveWaiting({ kind: 'agent_end', since: 1 }, 'bg task')).toBeUndefined();
    expect(effectiveWaiting({ kind: 'ask_user', since: 1 }, 'bg task')).toEqual({ kind: 'ask_user', since: 1 });
    expect(effectiveWaiting({ kind: 'agent_end', since: 1 }, undefined)).toEqual({ kind: 'agent_end', since: 1 });
  });

  it('the open session upgrades a stale idle row to working, never downgrades', () => {
    const row: ChatTarget = cardTarget('h1', data.sessions[1]);
    expect(liveStatus(row, { hostId: 'h1', session: 'b', running: true })).toBe('working');
    expect(liveStatus(row, { hostId: 'h1', session: 'x', running: true })).toBe('idle');
    const working = cardTarget('h1', data.sessions[0]);
    expect(liveStatus(working, { hostId: 'h1', session: 'a', running: false })).toBe('working');
    expect(withLive(row, { hostId: 'h1', session: 'b', running: false, lhMode: { mode: 'goal' } }).lhMode).toBe('goal');
  });

  it('needs-you rows float to the top, oldest first; waitingTargets lists them', () => {
    const rows = chatGroups('h1', data).flatMap((g) => g.items);
    expect(needsYouFirst(rows).map((t) => t.session)).toEqual(['b', 'a', 'c']);
    expect(waitingTargets('h1', data).map((t) => t.session)).toEqual(['b']);
  });

  it('a bare target (deep link) carries just the id and a title', () => {
    expect(bareTarget('h1', 's9', 'coffee')).toEqual({ hostId: 'h1', session: 's9', title: 'coffee', status: 'unknown' });
  });

  it('an empty or missing list has no groups', () => {
    expect(chatGroups('h1', undefined)).toEqual([]);
    expect(chatGroups('h1', { sessions: [] })).toEqual([]);
  });
});
