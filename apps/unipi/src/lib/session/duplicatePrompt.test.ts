import { describe, expect, it } from 'vitest';
import { enqueue } from './outbox';
import { applyEvent, applySnapshot, initialState, reconcileEntries } from './reducer';
import { buildTurns } from './turns';
import type { AgentMessage, SessionEntry } from './types';

describe('live prompt echo after history catch-up', () => {
  for (const image of [false, true]) {
    it(`keeps one ${image ? 'text and image' : 'text-only'} user turn live and after reload`, () => {
      const s = initialState('h', 's');
      const text = image ? 'Look at [image 1]' : 'Hello';
      const message: AgentMessage = { role: 'user', timestamp: 1000, content: [{ type: 'text', text }, ...(image ? [{ type: 'image', data: 'abc', mimeType: 'image/png' }] : [])] };
      const reminder: SessionEntry = { type: 'custom_message', id: 'reminder', parentId: null, timestamp: new Date(900).toISOString(), customType: 'oino.system-reminder', content: 'hidden', display: false };
      const user: SessionEntry = { type: 'message', id: 'user', parentId: 'reminder', timestamp: new Date(1000).toISOString(), message };
      const model: SessionEntry = { type: 'custom_message', id: 'model', parentId: 'user', timestamp: new Date(1100).toISOString(), customType: 'oino.system-reminder', content: 'Model changed', display: false };
      enqueue(s, { text, ...(image ? { images: [{ data: 'abc', mimeType: 'image/png' }] } : {}) }, true, 950);
      applyEvent(s, { session: 's', seq: 1, event: { type: 'message_start', message } }, 1000);
      expect(s.outbox).toHaveLength(0);
      // A catch-up request issued before the run can return persisted entries ahead of the queued live events.
      reconcileEntries(s, undefined, [reminder, user, model], 'model');
      applyEvent(s, { session: 's', seq: 2, event: { type: 'agent_start' } }, 1200);
      applyEvent(s, { session: 's', seq: 3, event: { type: 'message_end', message } }, 1200);
      applyEvent(s, { session: 's', seq: 4, event: { type: 'message_start', message: { role: 'assistant', content: [] } } }, 1300);
      applyEvent(s, { session: 's', seq: 5, event: { type: 'message_update', message: { role: 'assistant', content: [{ type: 'thinking', thinking: 'Thinking' }] } } }, 1400);
      const assertOne = () => {
        const turns = buildTurns(s).filter((t) => t.kind === 'turn');
        expect(turns).toHaveLength(1);
        expect(turns[0]!.user?.message).toEqual(message);
        expect(turns.some((t) => !t.live && t.durationMs === 0)).toBe(false);
      };
      assertOne();
      const reloaded = initialState('h', 's');
      applySnapshot(reloaded, { reset: true, seq: 5, entries: [reminder, user, model], leaf_id: 'model', state: { isStreaming: true } } as never, 1400);
      expect(buildTurns(reloaded).filter((t) => t.kind === 'turn')).toHaveLength(1);
    });
  }

  it('does not merge distinct sends with the same text or timestamp', () => {
    const s = initialState('h', 's');
    const first: AgentMessage = { role: 'user', timestamp: 1000, content: [{ type: 'text', text: 'same' }, { type: 'image', data: 'first' }] };
    reconcileEntries(s, undefined, [{ type: 'message', id: 'first', parentId: null, message: first }], 'first');
    const second = { ...first, content: [{ type: 'text', text: 'same' }, { type: 'image', data: 'second' }] };
    applyEvent(s, { session: 's', seq: 1, event: { type: 'message_end', message: second } }, 1000);
    const third = { ...first, timestamp: 1001 };
    applyEvent(s, { session: 's', seq: 2, event: { type: 'message_end', message: third } }, 1001);
    expect(buildTurns(s).filter((t) => t.kind === 'turn')).toHaveLength(3);
  });
});
