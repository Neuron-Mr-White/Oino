import { describe, expect, it } from 'vitest';
import { SCREEN_KEYS } from './defaults';

describe('Ctrl+/ screen keys (N-5)', () => {
  it('no longer mentions the removed Steer/Queue choice', () => {
    const text = SCREEN_KEYS.flatMap((g) => g.items.map((i) => `${i.keys} ${i.what}`)).join('\n');
    expect(text).not.toMatch(/Steer\/Queue|\bSteer\b|S \/ Q/);
    expect(text).toMatch(/After turn/);
    expect(text).toMatch(/Next step/);
  });
});
