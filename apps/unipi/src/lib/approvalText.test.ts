import { describe, expect, it } from 'vitest';
import { detailOf, humanizeArgs, humanizeLabel } from './approvalText';

describe('approvalText (E-07)', () => {
  const raw = '{"name":"Write bg2 marker file","command":"echo hi > bg2.txt","isAgent":false}';
  it('turns tool JSON into words', () => {
    expect(humanizeArgs(raw)).toBe('Run: echo hi > bg2.txt');
    expect(humanizeArgs('{"path":"/a/b.ts","content":"x"}')).toBe('Edit /a/b.ts');
    expect(humanizeArgs('Allow bash: ls?')).toBe('Allow bash: ls?');
  });
  it('never shows braces for clipped JSON', () => {
    expect(humanizeArgs('{"name":"n","command":"echo a b')).toBe('Run: echo a b');
  });
  it('humanizes backticked labels and finds the full command', () => {
    expect(humanizeLabel(`Always allow \`${raw}\``)).toBe('Always allow “echo hi > bg2.txt”');
    expect(detailOf(raw)).toEqual({ kind: 'command', text: 'echo hi > bg2.txt' });
  });
});
