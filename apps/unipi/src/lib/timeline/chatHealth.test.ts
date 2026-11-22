import { describe, expect, it } from 'vitest';
import { blankReason, healBudget, type ChatHealthInput } from './chatHealth';

const base: ChatHealthInput = { hostOnline: true, handleClosed: false, conn: 'live', takenOver: false, archived: false, settled: 3, rows: 2 };

describe('chat-vanish: blankReason', () => {
  it('a healthy live chat, an empty new chat and an open in progress need nothing', () => {
    expect(blankReason(base)).toBeUndefined();
    expect(blankReason({ ...base, settled: 0, rows: 0 })).toBeUndefined();
    expect(blankReason({ ...base, conn: 'connecting' })).toBeUndefined();
    expect(blankReason({ ...base, conn: 'reconnecting' })).toBeUndefined();
  });

  it('names every state that used to stay blank', () => {
    expect(blankReason({ ...base, handleClosed: true, conn: 'closed' })).toBe('handle_closed');
    expect(blankReason({ ...base, conn: 'closed' })).toBe('stream_closed');
    expect(blankReason({ ...base, conn: 'lost' })).toBe('stream_lost');
    expect(blankReason({ ...base, conn: 'closed', connErrorCode: 'E_INTERNAL' })).toBe('open_failed');
    // live, turns loaded, the virtual window rendered no row (0-height scroller after a hide/resize)
    expect(blankReason({ ...base, rows: 0 })).toBe('no_rows');
  });

  it('never fights an offline host, a takeover, an archived chat or a deleted session', () => {
    expect(blankReason({ ...base, conn: 'lost', hostOnline: false })).toBeUndefined();
    expect(blankReason({ ...base, conn: 'closed', takenOver: true })).toBeUndefined();
    expect(blankReason({ ...base, rows: 0, archived: true })).toBeUndefined();
    expect(blankReason({ ...base, conn: 'closed', connErrorCode: 'E_NOT_FOUND' })).toBeUndefined();
  });
});

describe('chat-vanish: healBudget', () => {
  it('allows max heals per window, then refuses until the window slides', () => {
    let t = 0;
    const b = healBudget(2, 60_000, () => t);
    expect(b.allow()).toBe(true);
    expect(b.allow()).toBe(true);
    expect(b.allow()).toBe(false);
    t = 59_999;
    expect(b.allow()).toBe(false);
    t = 60_000;
    expect(b.allow()).toBe(true);
  });
});
