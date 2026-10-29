import { describe, expect, it } from 'vitest';
import { setFilesTarget, consumeFilesTarget } from './filesTarget';

describe('filesTarget', () => {
  it('consumeFilesTarget reads and clears the pending target', () => {
    setFilesTarget({ hostId: 'h1', path: '/home/u' });
    expect(consumeFilesTarget()).toEqual({ hostId: 'h1', path: '/home/u' });
    expect(consumeFilesTarget()).toBeNull();
  });
});
