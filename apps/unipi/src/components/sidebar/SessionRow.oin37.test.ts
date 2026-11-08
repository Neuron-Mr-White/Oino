import { describe, expect, it } from 'vitest';
import type { SessionCard } from '../../lib/host/client';
import { liveRowState } from './SessionRow';

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
