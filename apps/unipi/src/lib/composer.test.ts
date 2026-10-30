import { describe, expect, it } from 'vitest';
import {
  SEND_HISTORY_MAX,
  parseStoredSendHistory,
  pushSendHistory,
} from './composer';

describe('send history', () => {
  it('pushes newest first and dedupes', () => {
    let history = pushSendHistory([], 'a');
    history = pushSendHistory(history, 'b');
    history = pushSendHistory(history, 'a');
    expect(history).toEqual(['a', 'b']);
  });

  it('ignores blank sends', () => {
    expect(pushSendHistory([], '  ')).toEqual([]);
  });

  it('caps at 50 entries', () => {
    let history: string[] = [];
    for (let i = 0; i < SEND_HISTORY_MAX + 10; i += 1) {
      history = pushSendHistory(history, `msg-${i}`);
    }
    expect(history).toHaveLength(SEND_HISTORY_MAX);
    expect(history[0]).toBe(`msg-${SEND_HISTORY_MAX + 9}`);
  });

  it('parses stored blobs defensively', () => {
    expect(parseStoredSendHistory(['a', '', 5, 'b'])).toEqual(['a', 'b']);
    expect(parseStoredSendHistory(undefined)).toEqual([]);
    expect(parseStoredSendHistory('nope')).toEqual([]);
  });
});
