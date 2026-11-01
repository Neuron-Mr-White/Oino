// @vitest-environment jsdom
/**
 * Composer drafts (WP-A12, Z-59): text persisted per session (debounced,
 * removed when emptied, restored after a "reload"), chips/quotes/images in
 * memory only, and the send text assembly (quotes → blockquotes, chip
 * markdown expanded inline — Z-61, then the draft text).
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  addComposerChip,
  addComposerQuote,
  clearComposer,
  composerChips,
  composerDraft,
  composerSendText,
  flushComposerDrafts,
  reloadComposerDrafts,
  removeComposerChip,
  resetComposerDrafts,
  restoreComposer,
  setComposerText,
  setComposerImages,
} from './drafts';

const KEY = 'pc1/s1';

afterEach(() => {
  vi.useRealTimers();
  resetComposerDrafts();
  localStorage.clear();
});

describe('D-11 clearComposer with a persisted draft', () => {
  it('reads back empty at once, even when the text was already stored', () => {
    setComposerText(KEY, '/copy');
    flushComposerDrafts();
    clearComposer(KEY);
    expect(composerDraft(KEY).text).toBe('');
  });
});

describe('text persistence (Z-59)', () => {
  it('persists debounced 500 ms and only while non-empty', () => {
    vi.useFakeTimers();
    setComposerText(KEY, 'half a thought');
    expect(localStorage.getItem('unipi:composer:draft:pc1/s1')).toBeNull();
    vi.advanceTimersByTime(500);
    expect(localStorage.getItem('unipi:composer:draft:pc1/s1')).toBe('half a thought');

    setComposerText(KEY, '');
    // emptied: the record goes at once, not 500 ms later
    expect(localStorage.getItem('unipi:composer:draft:pc1/s1')).toBeNull();
  });

  it('whitespace-only text is not a draft ("没有输入文字就不算草稿")', () => {
    vi.useFakeTimers();
    setComposerText(KEY, '   \n ');
    vi.advanceTimersByTime(600);
    expect(localStorage.getItem('unipi:composer:draft:pc1/s1')).toBeNull();
  });

  it('a reload restores the text (and only the text)', () => {
    vi.useFakeTimers();
    setComposerText(KEY, 'still typing this');
    setComposerImages(KEY, [{ mimeType: 'image/png', data: 'x' }]);
    addComposerQuote(KEY, 'quoted');
    vi.advanceTimersByTime(500);
    // "Reload": the in-memory store dies, storage survives.
    reloadComposerDrafts();
    const draft = composerDraft(KEY);
    expect(draft.text).toBe('still typing this');
    expect(draft.images).toEqual([]);
    expect(draft.quotes).toEqual([]);
  });

  it('flushComposerDrafts writes pending text at once (pagehide)', () => {
    vi.useFakeTimers();
    setComposerText(KEY, 'before the app closes');
    flushComposerDrafts();
    expect(localStorage.getItem('unipi:composer:draft:pc1/s1')).toBe('before the app closes');
  });

  it('clearComposer (a send) removes the stored record', () => {
    vi.useFakeTimers();
    setComposerText(KEY, 'sent!');
    vi.advanceTimersByTime(500);
    clearComposer(KEY);
    expect(localStorage.getItem('unipi:composer:draft:pc1/s1')).toBeNull();
    expect(composerDraft(KEY).text).toBe('');
  });

  it('restoreComposer (Edit) replaces text and images, keeps chips', () => {
    addComposerChip(KEY, { kind: 'paste', label: 'Pasted text · 2 lines', markdown: 'a\nb' });
    restoreComposer(KEY, { text: 'the old message', images: [{ mimeType: 'image/png', data: 'i' }] });
    expect(composerDraft(KEY)).toEqual({ text: 'the old message', quotes: [], images: [{ mimeType: 'image/png', data: 'i' }], chips: [{ id: expect.any(String), kind: 'paste', label: 'Pasted text · 2 lines', markdown: 'a\nb' }] });
  });
});

describe('the public chip API (composerChips, WP-B8)', () => {
  it('add() drops a chip into the session draft and remove() takes it out', () => {
    const id = composerChips.add(KEY, { kind: 'element', label: 'button "Sign in"', markdown: '<picked element>…' });
    expect(composerDraft(KEY).chips).toHaveLength(1);
    expect(composerChips.add(KEY, { kind: 'screenshot', label: 'Screenshot 12:04' })).toBeTruthy();
    expect(composerDraft(KEY).chips).toHaveLength(2);
    composerChips.remove(KEY, id);
    expect(composerDraft(KEY).chips.map((c) => c.kind)).toEqual(['screenshot']);
  });

  it('chips are per session', () => {
    composerChips.add('pc1/s1', { kind: 'element', label: 'a' });
    composerChips.add('pc1/s2', { kind: 'element', label: 'b' });
    expect(composerDraft('pc1/s1').chips[0]!.label).toBe('a');
    expect(composerDraft('pc1/s2').chips[0]!.label).toBe('b');
  });
});

describe('composerSendText (quotes + chips + text)', () => {
  it('quotes become blockquotes, chips expand inline, the text comes last', () => {
    addComposerQuote(KEY, 'first line\nsecond line');
    addComposerChip(KEY, { kind: 'paste', label: 'Pasted text · 2 lines', markdown: 'THE PASTED\nTEXT' });
    setComposerText(KEY, 'summarize both');
    expect(composerSendText(composerDraft(KEY))).toBe('> first line\n> second line\n\nTHE PASTED\nTEXT\n\nsummarize both');
  });

  it('a chip without markdown (a screenshot pointer) adds nothing to the prompt', () => {
    addComposerChip(KEY, { kind: 'screenshot', label: 'Screenshot' });
    setComposerText(KEY, 'look');
    expect(composerSendText(composerDraft(KEY))).toBe('look');
  });

  it('removeComposerChip drops exactly that chip', () => {
    const a = addComposerChip(KEY, { kind: 'paste', label: 'a', markdown: 'A' });
    addComposerChip(KEY, { kind: 'paste', label: 'b', markdown: 'B' });
    removeComposerChip(KEY, a.id);
    expect(composerSendText(composerDraft(KEY))).toBe('B');
  });
});
