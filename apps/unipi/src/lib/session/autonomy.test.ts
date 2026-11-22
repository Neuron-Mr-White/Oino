import { describe, expect, it } from 'vitest';
import { autonomyKnown, autonomyLevel } from './autonomy';
import type { SessionHandle } from './store';

const handle = (state: Record<string, unknown>) => ({ state: { entries: [], info: {}, ...state } }) as unknown as SessionHandle;

describe('autonomyKnown (r5 D21, r7 P2-1)', () => {
  it('is pending before the first state: nothing arrived yet, not "medium"', () => {
    expect(autonomyKnown(handle({}))).toBe(false);
  });
  it('is pending while a waking chat has no level of its own', () => {
    expect(autonomyKnown(handle({ waking: true, info: { cwd: '/x' } }))).toBe(false);
  });
  it('is known once the state arrived, with or without a level (older host: medium)', () => {
    const g = handle({ info: { cwd: '/x' } });
    expect(autonomyKnown(g)).toBe(true);
    expect(autonomyLevel(g)).toBe('medium');
  });
  it('a level from the host is known at once, even while waking', () => {
    expect(autonomyKnown(handle({ waking: true, info: { autonomy: 'high' } }))).toBe(true);
  });
});
