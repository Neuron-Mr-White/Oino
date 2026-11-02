import { describe, expect, it } from 'vitest';
import { resolveSettingsHost } from '../target';

describe('resolveSettingsHost (PH-04)', () => {
  it('keeps this computer when the app has a local host (desktop unchanged)', () => {
    expect(resolveSettingsHost([{ id: 'a', transport: 'iroh' }, { id: 'me', transport: 'local' }], 'a')).toBe('local');
  });
  it('follows the selected PC on a phone', () => {
    expect(resolveSettingsHost([{ id: 'a', transport: 'iroh' }, { id: 'b', transport: 'iroh' }], 'b')).toBe('b');
  });
  it('falls back to the first paired PC when the selection is stale', () => {
    expect(resolveSettingsHost([{ id: 'a', transport: 'iroh' }], 'gone')).toBe('a');
  });
  it('no PCs: local (nothing to manage yet)', () => {
    expect(resolveSettingsHost([], undefined)).toBe('local');
  });
});
