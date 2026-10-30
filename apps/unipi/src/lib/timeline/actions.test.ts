/**
 * Turn-action helpers (WP-A7): whole-turn copy text, quote source, the
 * composer handoff (quote chips + restored edit drafts), the phone swipe
 * threshold and the prose-key → entry-id mapping.
 */
import { afterEach, describe, expect, it } from 'vitest';
import { buildTurns, type Turn } from '../session/turns';
import type { SessionEntry } from '../session/types';
import {
  addComposerQuote,
  clearComposer,
  composerDraft,
  composerSendText,
  entryIdOfKey,
  entryImages,
  excerpt,
  removeComposerQuote,
  restoreComposer,
  setComposerText,
  swipeRight,
  turnQuoteText,
  turnText,
  turnTime,
} from './actions';

const entry = (id: string, parentId: string | null, message: Record<string, unknown>, timestamp: string): SessionEntry =>
  ({ type: 'message', id, parentId, timestamp, message: { timestamp: Date.parse(timestamp), ...message } }) as unknown as SessionEntry;

function twoTurnSession(): Turn[] {
  const entries: SessionEntry[] = [
    entry('u1', null, { role: 'user', content: [{ type: 'text', text: 'first question' }] }, '2026-10-30T09:00:00Z'),
    entry('a1', 'u1', { role: 'assistant', content: [{ type: 'text', text: 'first answer' }], stopReason: 'toolUse' }, '2026-10-30T09:00:10Z'),
    entry('u1s', 'a1', { role: 'user', content: [{ type: 'text', text: 'a steer' }] }, '2026-10-30T09:00:20Z'),
    entry('a1b', 'u1s', { role: 'assistant', content: [{ type: 'text', text: 'answer after steer' }], stopReason: 'stop' }, '2026-10-30T09:01:00Z'),
    entry('u2', 'a1b', { role: 'user', content: [{ type: 'text', text: 'second question' }, { type: 'image', mimeType: 'image/png', data: 'AAAA' }] }, '2026-10-30T09:05:00Z'),
    entry('a2', 'u2', { role: 'assistant', content: [{ type: 'text', text: 'second answer' }], stopReason: 'stop' }, '2026-10-30T09:05:30Z'),
  ];
  return buildTurns({ entries, leafId: 'a2', entriesRev: 1, liveTools: {}, run: { running: false } });
}

afterEach(() => {
  for (const key of ['h1/s1', 'h1/s2']) clearComposer(key);
});

describe('turn text', () => {
  it('copies the query, steers and answers of the turn', () => {
    const [first] = twoTurnSession();
    expect(turnText(first!)).toBe('first question\n\nfirst answer\n\na steer\n\nanswer after steer');
  });

  it('quotes the last answer, falling back to the query', () => {
    const [first, second] = twoTurnSession();
    expect(turnQuoteText(first!)).toBe('answer after steer');
    expect(turnQuoteText(second!)).toBe('second answer');
  });

  it('takes the settle time from the end of the turn', () => {
    const [first, second] = twoTurnSession();
    expect(turnTime(first!)).toBe(Date.parse('2026-10-30T09:01:00Z'));
    expect(turnTime(second!)).toBe(Date.parse('2026-10-30T09:05:30Z'));
  });

  it('maps prose keys back to settled entries only', () => {
    const state = { byId: { a1: 0, a2: 1 } as Record<string, number> };
    expect(entryIdOfKey(state, 'a1-0')).toBe('a1');
    expect(entryIdOfKey(state, 'stream-0')).toBeUndefined();
    expect(entryIdOfKey(state, undefined)).toBeUndefined();
  });

  it('reads a message image in the outbox shape', () => {
    const turns = twoTurnSession();
    expect(entryImages(turns[1]!.user)).toEqual([{ mimeType: 'image/png', data: 'AAAA' }]);
  });

  it('flattens and clips excerpts', () => {
    expect(excerpt('one\ntwo   three', 8)).toBe('one two…');
    expect(excerpt('short')).toBe('short');
  });
});

describe('composer handoff', () => {
  it('keeps text and quote chips, and expands quotes into blockquotes on send', () => {
    const key = 'h1/s1';
    setComposerText(key, 'my follow-up');
    const quote = addComposerQuote(key, 'line one\nline two');
    expect(composerDraft(key).quotes.map((q) => q.text)).toEqual(['line one\nline two']);
    expect(composerSendText(composerDraft(key))).toBe('> line one\n> line two\n\nmy follow-up');
    removeComposerQuote(key, quote.id);
    expect(composerSendText(composerDraft(key))).toBe('my follow-up');
  });

  it('a quote-only draft still sends', () => {
    addComposerQuote('h1/s1', '> nested');
    expect(composerSendText(composerDraft('h1/s1'))).toBe('> > nested');
  });

  it('edit restores text + images and replaces quotes', () => {
    addComposerQuote('h1/s2', 'old quote');
    restoreComposer('h1/s2', { text: 'the original message', images: [{ mimeType: 'image/png', data: 'BBBB' }] });
    expect(composerDraft('h1/s2')).toEqual({ text: 'the original message', quotes: [], images: [{ mimeType: 'image/png', data: 'BBBB' }], chips: [] });
  });

  it('clears back to the shared empty draft', () => {
    setComposerText('h1/s1', 'typed');
    clearComposer('h1/s1');
    expect(composerDraft('h1/s1')).toEqual({ text: '', quotes: [], images: [], chips: [] });
  });
});

describe('phone swipe-to-quote', () => {
  it('needs a mostly horizontal, far-enough swipe', () => {
    expect(swipeRight({ x: 10, y: 10 }, { x: 90, y: 20 })).toBe(true);
    expect(swipeRight({ x: 10, y: 10 }, { x: 60, y: 10 })).toBe(false);
    expect(swipeRight({ x: 10, y: 10 }, { x: 90, y: 70 })).toBe(false);
    expect(swipeRight({ x: 90, y: 10 }, { x: 10, y: 10 })).toBe(false);
  });
});
