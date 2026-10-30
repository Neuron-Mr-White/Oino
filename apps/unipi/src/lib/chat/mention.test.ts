import { describe, expect, it } from 'vitest';
import { queueMention, consumeMentionFor } from './mention';

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
});
