import { describe, it, expect } from 'vitest';
import { ansiToLines } from './ansi';

describe('ansiToLines', () => {
  it('passes plain text through', () => {
    const lines = ansiToLines('hello world');
    expect(lines).toHaveLength(1);
    expect(lines[0]).toEqual([{ text: 'hello world', style: {} }]);
  });

  it('applies the 16-colour foreground', () => {
    const lines = ansiToLines('\x1b[32mgreen');
    expect(lines[0][0].style.color).toBe('#5AD25A');
  });

  it('maps xterm-256 colours', () => {
    const lines = ansiToLines('\x1b[38;5;196mred');
    expect(lines[0][0].style.color).toBe('rgb(255, 0, 0)');
  });

  it('maps truecolor', () => {
    const lines = ansiToLines('\x1b[38;2;12;34;56mtrue\x1b[0m');
    expect(lines[0][0].text).toBe('true');
    expect(lines[0][0].style.color).toBe('rgb(12, 34, 56)');
  });

  it('bold survives until reset', () => {
    const lines = ansiToLines('\x1b[1mbold\x1b[0mplain');
    expect(lines[0][0].style['font-weight']).toBe('600');
    expect(lines[0][1].text).toBe('plain');
    expect(lines[0][1].style['font-weight']).toBeUndefined();
  });

  it('strips OSC titles', () => {
    const lines = ansiToLines('\x1b]0;my title\x07hello');
    expect(lines[0][0].text).toBe('hello');
  });

  it('strips cursor movement and mode switches', () => {
    const lines = ansiToLines('\x1b[2J\x1b[H\x1b[?25hvisible');
    expect(lines[0][0].text).toBe('visible');
  });

  it('keeps CJK characters', () => {
    const lines = ansiToLines('こんにちは 世界');
    expect(lines[0][0].text).toBe('こんにちは 世界');
  });

  it('clips to the requested number of lines', () => {
    const lines = ansiToLines('a\nb\nc\nd', 2);
    expect(lines).toHaveLength(2);
    expect(lines[0][0].text).toBe('c');
  });
});
