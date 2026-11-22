import { describe, expect, it } from 'vitest';
import { installedVersionLabel } from './ComponentsCard';

describe('running product versions', () => {
  it('labels app and live host independently and never substitutes a bundled version', () => {
    expect(installedVersionLabel('1.0.2', '1.0.1')).toBe('App 1.0.2 · Host 1.0.1 (restart host to update)');
    expect(installedVersionLabel('1.0.2', '1.0.2')).toBe('App 1.0.2 · Host 1.0.2');
    expect(installedVersionLabel('1.0.2')).toBe('App 1.0.2 · Host unavailable');
    expect(installedVersionLabel(undefined, '1.0.1')).toBe('App unavailable');
  });
});
