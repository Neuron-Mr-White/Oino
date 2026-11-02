import { describe, expect, it } from 'vitest';
import { liveTrayLine } from './trayHint';

describe('E-17 live tray hint', () => {
  it('drops finished work and keeps what runs', () => {
    expect(liveTrayLine('Background tasks 4 · Subagents 2 · ↓ open')).toBeNull();
    expect(liveTrayLine('Background tasks 4 · Subagents 2 running · ↓ open')).toBe('Subagents 2 running · ↓ open');
    expect(liveTrayLine('Subagents 3 (1 running) · ↓ open')).toBe('Subagents 1 running · ↓ open');
    expect(liveTrayLine('Bg 4/2 · ↓ open')).toBe('Bg 2 running · ↓ open');
    expect(liveTrayLine('something else')).toBeUndefined();
  });
});

import { humanStatus } from './statusFeed';
describe('F-13 human status', () => {
  it('humanises and hides', () => {
    expect(humanStatus('🧠 0 mem')).toBeNull();
    expect(humanStatus('🧠 3 mem')).toBe('3 memories');
    expect(humanStatus('skills: 17/17 listed')).toBe('17 skills ready');
    expect(humanStatus('Graph')).toBeNull();
    expect(humanStatus('main ↑1')).toBe('main ↑1');
  });
});
