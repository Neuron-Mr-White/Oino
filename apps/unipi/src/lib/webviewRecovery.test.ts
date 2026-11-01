import { describe, expect, it, vi } from 'vitest';
import { toasts } from './toasts';
import { announceWebviewRecovery } from './webviewRecovery';

describe('announceWebviewRecovery', () => {
  it('says nothing on a normal start', async () => {
    const push = vi.spyOn(toasts(), 'push');
    expect(await announceWebviewRecovery(async () => null)).toBe(false);
    expect(push).not.toHaveBeenCalled();
  });
  it('toasts once after a recovered web process', async () => {
    const push = vi.spyOn(toasts(), 'push');
    expect(await announceWebviewRecovery(async () => 'it ran out of memory')).toBe(true);
    expect(push).toHaveBeenCalledWith(expect.stringContaining('ran out of memory'), 'info');
  });
  it('ignores a core without the command', async () => {
    expect(await announceWebviewRecovery(async () => { throw new Error('unknown command'); })).toBe(false);
  });
});
