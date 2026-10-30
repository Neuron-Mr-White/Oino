import { describe, expect, it } from 'vitest';
import { plainText } from './plain';

describe('plainText', () => {
  it('strips bold/italic/code markers', () => {
    expect(plainText('Need **a decision**: pin `0.95`?')).toBe('Need a decision: pin 0.95?');
  });
  it('strips table pipes', () => {
    expect(plainText('| a | b |\n|---|---|\n| 1 | 2 |')).not.toContain('|');
  });
  it('strips list markers and joins lines with spaces', () => {
    const text = plainText('- one\n- two\n- three');
    expect(text).not.toMatch(/^[-*]/m);
    expect(text).toContain('one');
    expect(text).toContain('two');
  });
  it('flattens a fenced code block to plain text with no backticks/pipes', () => {
    const text = plainText('before\n\n```\na | b\n```\n\nafter');
    expect(text).not.toContain('`');
    expect(text).not.toContain('|');
  });
  it('empty for blank input', () => {
    expect(plainText(undefined)).toBe('');
    expect(plainText('   ')).toBe('');
  });
});
