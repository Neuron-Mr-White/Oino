import { describe, expect, it, vi } from 'vitest';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri')), transformCallback: () => 0 }));
import { nativeEnabledOf } from './desktopSwitch';

describe('one desktop switch: the notify extension\u2019s Native desktop', () => {
  it('reads native.enabled nested or dotted; absent → undefined', () => {
    expect(nativeEnabledOf({ native: { enabled: false } })).toBe(false);
    expect(nativeEnabledOf({ 'native.enabled': true })).toBe(true);
    expect(nativeEnabledOf({ recap: {} })).toBeUndefined();
    expect(nativeEnabledOf(null)).toBeUndefined();
  });
});
