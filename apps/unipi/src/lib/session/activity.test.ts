import { describe, expect, it } from 'vitest';
import type { SessionCard } from '../host/client';
import { activityCounts, activityTitle, cardActivity, liveActivity, type LiveSession } from './activity';

const card = (over: Partial<SessionCard> = {}): SessionCard => ({ session_id: 's', run_state: 'idle', ...over });
const live = (run: Partial<LiveSession['run']> = {}, dialogs: string[] = []): LiveSession => ({
  conn: 'live',
  run: { phase: 'idle', running: false, ...run },
  dialogs,
});

describe('[status] session activity states', () => {
  it('reads the host card: working (run or wake-capable pending work) / background / needs-you / idle', () => {
    expect(cardActivity(card())).toEqual({ activity: 'idle' });
    expect(cardActivity(card({ run_state: 'working' }))).toEqual({ activity: 'working' });
    // A parent waiting on graph nodes: the host keeps it working and says why.
    expect(cardActivity(card({ run_state: 'working', pending_work: 'subagent working' }))).toEqual({ activity: 'working', detail: 'subagent working' });
    // An older host that sends only `pending_work` still reads Working.
    expect(cardActivity(card({ pending_work: 'bg: build' })).activity).toBe('working');
    expect(cardActivity(card({ background_work: 'bg: dev server' }))).toEqual({ activity: 'background', detail: 'bg: dev server' });
    // Needs you wins over everything.
    expect(cardActivity(card({ run_state: 'working', pending_work: 'x', needs_you: true })).activity).toBe('needs-you');
  });

  it('a live stream refines the card: waiting keeps Working, a stale card never flashes Working, dialogs are needs-you', () => {
    // The parent's run ended but it waits on subagents (core `waiting`): Working, not idle.
    expect(liveActivity(card({ run_state: 'working' }), live({ phase: 'waiting', waitingOn: ['2 subagents working'] }))).toEqual({
      activity: 'working',
      detail: '2 subagents working',
    });
    // Opening an idle chat whose card still says working (polled, stale): idle.
    expect(liveActivity(card({ run_state: 'working' }), live()).activity).toBe('idle');
    // …unless the host says wake-capable work is pending.
    expect(liveActivity(card({ run_state: 'working', pending_work: 'bg: tests' }), live()).activity).toBe('working');
    expect(liveActivity(card(), live({ running: true, phase: 'running' })).activity).toBe('working');
    expect(liveActivity(card(), live({ phase: 'compacting' })).activity).toBe('working');
    expect(liveActivity(card(), live({ backgroundOn: ['bg: dev'] }))).toEqual({ activity: 'background', detail: 'bg: dev' });
    expect(liveActivity(card(), live({ running: true }, ['dlg-1'])).activity).toBe('needs-you');
    // Not live: the card is the truth.
    expect(liveActivity(card({ run_state: 'working' }), { ...live(), conn: 'reconnecting' }).activity).toBe('working');
  });

  it('counts for the tray, and the tooltips', () => {
    const facts = [cardActivity(card({ run_state: 'working' })), cardActivity(card({ needs_you: true })), cardActivity(card({ background_work: 'bg: x' })), cardActivity(card())];
    expect(activityCounts(facts)).toEqual({ needs: 1, working: 1, background: 1 });
    expect(activityTitle({ activity: 'working', detail: 'subagent working' })).toBe('Working — waiting on subagent working');
    expect(activityTitle({ activity: 'background', detail: 'bg: dev' })).toBe('Running in the background: bg: dev');
    expect(activityTitle({ activity: 'idle' })).toBeUndefined();
  });
});
