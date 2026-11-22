import { afterEach, describe, expect, it, vi } from 'vitest';
import { composerDraft, resetComposerDrafts } from '../composer/drafts';
import { consumeMentionFor, pendingMention, queueMention, resetMentions, subscribeMention } from './mention';

afterEach(() => {
  resetMentions();
  resetComposerDrafts();
});

describe('mention mailbox', () => {
  it('consumeMentionFor returns the text for a matching key and clears it', () => {
    queueMention('h1:123', '@/home/u/a.txt');
    expect(consumeMentionFor('h1:123')).toBe('@/home/u/a.txt');
    expect(consumeMentionFor('h1:123')).toBeNull();
  });

  it('consumeMentionFor returns null and clears for a non-matching key', () => {
    queueMention('h1:123', '@/home/u/a.txt');
    expect(consumeMentionFor('other:1')).toBeNull();
    // Cleared even though the key didn't match (one-shot mailbox).
    expect(consumeMentionFor('h1:123')).toBeNull();
  });

  it('matches v1 host:session and v2 host/session keys and delivers to subscribeMention without duplication', () => {
    queueMention('local:s42', '@src/app.ts ');
    expect(pendingMention()).toEqual({ key: 'local:s42', text: '@src/app.ts ' });
    expect(composerDraft('local/s42').text).toBe('@src/app.ts ');

    let draftBeforeCallback = '';
    const seen = vi.fn((_text: string) => {
      draftBeforeCallback = composerDraft('local/s42').text;
    });
    const off = subscribeMention('local/s42', seen);
    expect(seen).toHaveBeenCalledTimes(1);
    expect(seen).toHaveBeenCalledWith('@src/app.ts ');
    expect(draftBeforeCallback).toBe('');
    expect(pendingMention()).toBeNull();

    queueMention('local:s42', '@README.md ');
    expect(seen).toHaveBeenCalledTimes(2);
    expect(seen).toHaveBeenLastCalledWith('@README.md ');
    off();
  });
});
