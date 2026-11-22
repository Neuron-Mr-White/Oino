import { describe, expect, it } from 'vitest';
import type { SessionCard } from '../../lib/host/client';
import { liveRowFacts, liveRowState } from './SessionRow';

describe('unselected session activity', () => {
  it('keeps the spinner from refreshed host cards without a live feed', () => {
    for (const name of ['Lead · project', 'Regular chat']) {
      const card = { session_id: 's', name, run_state: 'working', pending_work: 'subagent working' } as SessionCard;
      expect(liveRowState(card, undefined)).toBe('working');
      expect(liveRowState({ ...card }, undefined)).toBe('working');
      expect(liveRowState({ ...card, pending_work: 'bg: build' }, undefined)).toBe('working');
      expect(liveRowState({ ...card, run_state: 'idle', pending_work: undefined, background_work: 'bg: server' }, undefined)).toBe('background');
    }
  });
});

describe('D24: a restarted host\'s crashed card while the open chat reconnects', () => {
  const card = { session_id: 's', name: 'Chat', run_state: 'crashed' } as SessionCard;
  const open = (over: Record<string, unknown>) => ({ state: { conn: 'live', run: { running: false, phase: 'idle' }, dialogs: [], ...over } }) as never;
  it('reads Reconnecting while the stream is down or waking, Crashed once it is live and still crashed', () => {
    expect(liveRowState(card, open({ conn: 'reconnecting' }))).toBe('reconnecting');
    expect(liveRowState(card, open({ conn: 'live', waking: true }))).toBe('reconnecting');
    expect(liveRowState(card, open({ conn: 'live' }))).toBe('crashed');
    expect(liveRowState(card, undefined)).toBe('crashed');
    expect(liveRowFacts(card, open({ conn: 'connecting' })).state).toBe('reconnecting');
  });
});

describe('r8 P2-A: a "Working" card the host cannot update while the open chat\'s link is down', () => {
  const card = { session_id: 's', name: 'Chat', run_state: 'working' } as SessionCard;
  const open = (over: Record<string, unknown>) => ({ state: { conn: 'live', everLive: true, run: { running: true, phase: 'run' }, dialogs: [], ...over } }) as never;
  it('reads Reconnecting while the stream is down (also connecting / closed with an error after being live), Working once it is live', () => {
    expect(liveRowState(card, open({ conn: 'live' }))).toBe('working');
    expect(liveRowState(card, open({ conn: 'reconnecting' }))).toBe('reconnecting');
    expect(liveRowState(card, open({ conn: 'connecting' }))).toBe('reconnecting');
    expect(liveRowState(card, open({ conn: 'closed', connError: { code: 'E_UNAUTHORIZED', message: 'x', retryable: false } }))).toBe('reconnecting');
    expect(liveRowState(card, undefined)).toBe('working');
  });
});
