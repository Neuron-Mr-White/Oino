/**
 * Chat list (UNI-161 "needs you"): chatGroups carries `waiting` through from
 * a pane's `chat.waiting`, and needsYouFirst floats waiting items to the top
 * (oldest-waiting first), leaving everything else in its original order.
 */
import { describe, expect, it } from 'vitest';
import { chatGroups, effectiveWaiting, liveStatus, needsYouFirst, paneStatus, waitingTargets, withLive, type ChatTarget } from './list';
import type { ChatListData } from './transport';

function data(panes: Array<{ id: string; waiting?: { kind: string; title?: string; since: number } }>): ChatListData {
  return {
    herdr: {
      available: true,
      sessions: [
        {
          name: 'default',
          running: true,
          default: true,
          workspaces: [
            {
              id: 'w1',
              label: 'main',
              number: 1,
              focused: true,
              agent_status: 'idle',
              tab_count: 1,
              pane_count: panes.length,
              tabs: [
                {
                  id: 'w1:t1',
                  label: 't',
                  number: 1,
                  focused: true,
                  agent_status: 'idle',
                  panes: panes.map((p, i) => ({
                    id: p.id,
                    focused: false,
                    agent: { name: 'pi', status: 'idle' as const },
                    chat: { pid: 100 + i, session_name: p.id, waiting: p.waiting },
                  })),
                },
              ],
            },
          ],
        },
      ],
    },
  };
}

describe('chatGroups', () => {
  it('carries a pane\'s chat.waiting through to the ChatTarget', () => {
    const groups = chatGroups('host1', data([{ id: 'a' }, { id: 'b', waiting: { kind: 'ask_user', title: 'Pick a plan', since: 1000 } }]));
    const items = groups[0]!.items;
    expect(items.find((x) => x.title === 'a')?.waiting).toBeUndefined();
    expect(items.find((x) => x.title === 'b')?.waiting).toEqual({ kind: 'ask_user', title: 'Pick a plan', since: 1000 });
  });
});

describe('needsYouFirst', () => {
  it('floats waiting items to the top, oldest-waiting first, keeping the rest in order', () => {
    const items = [
      { id: 'a' },
      { id: 'b', waiting: { kind: 'ask_user', since: 2000 } },
      { id: 'c' },
      { id: 'd', waiting: { kind: 'agent_end', since: 1000 } },
    ];
    expect(needsYouFirst(items).map((x) => x.id)).toEqual(['d', 'b', 'a', 'c']);
  });

  it('no waiting items: order is unchanged', () => {
    const items: Array<{ id: string; waiting?: { kind: string; since: number } }> = [{ id: 'a' }, { id: 'b' }, { id: 'c' }];
    expect(needsYouFirst(items).map((x) => x.id)).toEqual(['a', 'b', 'c']);
  });
});

describe('UNI-212: busy pis never read "Idle"', () => {
  const pane = (status: 'idle' | 'working' | 'blocked', running?: boolean) => ({ id: 'w1:p1', focused: false, agent: { name: 'pi', status }, chat: { pid: 1, running } });
  it('paneStatus: the bridge record running flag beats a stale herdr idle; blocked still wins', () => {
    expect(paneStatus(pane('idle', true))).toBe('working');
    expect(paneStatus(pane('idle', false))).toBe('idle');
    expect(paneStatus(pane('idle'))).toBe('idle');
    expect(paneStatus(pane('blocked', true))).toBe('blocked');
    expect(paneStatus({ id: 'x', focused: false })).toBe('unknown');
  });
  it('liveStatus: the open live chat upgrades its own row to Working, never downgrades, ignores other rows', () => {
    const t = { hostId: 'h', pid: 1, status: 'idle' as const };
    expect(liveStatus(t, { hostId: 'h', pid: 1, running: true })).toBe('working');
    expect(liveStatus(t, { hostId: 'h', pid: 2, running: true })).toBe('idle');
    expect(liveStatus({ ...t, status: 'working' }, { hostId: 'h', pid: 1, running: false })).toBe('working');
    expect(liveStatus({ ...t, status: 'blocked' }, { hostId: 'h', pid: 1, running: true })).toBe('blocked');
    expect(liveStatus(t, undefined)).toBe('idle');
  });
});

describe('UNI-221: wake-capable pending work reads "Working…", never finished/needs-you', () => {
  const pane = (status: 'idle' | 'working' | 'blocked', chat: Record<string, unknown>) => ({ id: 'w1:p1', focused: false, agent: { name: 'pi', status }, chat: { pid: 1, ...chat } });
  it('paneStatus: pending_work keeps an idle pi "working"; blocked still wins', () => {
    expect(paneStatus(pane('idle', { running: false, pending_work: 'bg: npm test' }))).toBe('working');
    expect(paneStatus(pane('blocked', { pending_work: 'bg: npm test' }))).toBe('blocked');
    expect(paneStatus(pane('idle', { running: false }))).toBe('idle');
  });
  it('effectiveWaiting: an agent_end mark is suppressed while work is pending; dialogs are not', () => {
    const end = { kind: 'agent_end', since: 5 };
    expect(effectiveWaiting(end, 'subagent working')).toBeUndefined();
    expect(effectiveWaiting({ kind: 'agent_settled', since: 5 }, 'x')).toBeUndefined();
    expect(effectiveWaiting(end, undefined)).toEqual(end);
    const ask = { kind: 'ask_user', title: 'Pick', since: 5 };
    expect(effectiveWaiting(ask, 'bg: x')).toEqual(ask);
  });
  it('chatGroups carries pending_work to the row and drops a stale finished mark (so the bell and "Run finished" skip it)', () => {
    const d = data([{ id: 'a', waiting: { kind: 'agent_end', since: 1 } }]);
    const p = d.herdr.sessions![0]!.workspaces[0]!.tabs[0]!.panes[0]!;
    p.chat = { ...p.chat!, pending_work: '2 subagents working' };
    const t = chatGroups('h', d)[0]!.items[0]!;
    expect(t.status).toBe('working');
    expect(t.pendingWork).toBe('2 subagents working');
    expect(t.waiting).toBeUndefined();
    expect(waitingTargets('h', d)).toEqual([]);
  });
  it('withLive: the open chat\'s live run.waiting beats the polled record both ways', () => {
    const base: ChatTarget = { hostId: 'h', session: 's', workspaceId: 'w', workspaceLabel: 'w', tabId: 't', paneId: 'p', pid: 1, title: 'x', status: 'idle', focused: false };
    const on = withLive(base, { hostId: 'h', pid: 1, running: false, waiting: 'bg: sleep 30' });
    expect(on.status).toBe('working');
    expect(on.pendingWork).toBe('bg: sleep 30');
    const off = withLive({ ...base, status: 'working', pendingWork: 'bg: sleep 30' }, { hostId: 'h', pid: 1, running: false });
    expect(off.pendingWork).toBeUndefined();
    const other = withLive(base, { hostId: 'h', pid: 2, running: false, waiting: 'x' });
    expect(other.status).toBe('idle');
    expect(other.pendingWork).toBeUndefined();
  });
  it('liveStatus: an idle open chat with pending work is working', () => {
    expect(liveStatus({ hostId: 'h', pid: 1, status: 'idle' }, { hostId: 'h', pid: 1, running: false, waiting: 'x' })).toBe('working');
  });
});
