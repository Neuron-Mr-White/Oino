import { describe, expect, it, vi } from 'vitest';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri')), transformCallback: () => 0 }));
import { desktopNotes } from './AboutSection';

describe('P1-08: About › What’s new reads the desktop feed', () => {
  it('notes of the newer build; none while idle or without notes', () => {
    const info = { current: '1.0.0', kind: 'in_app' as const, version: '1.0.1', notes: '- faster opens\n', downloadPage: 'x' };
    expect(desktopNotes({ kind: 'available', info })).toEqual({ version: '1.0.1', notes: '- faster opens' });
    expect(desktopNotes({ kind: 'idle' })).toBeUndefined();
    expect(desktopNotes({ kind: 'current', info: { ...info, version: undefined, notes: undefined } })).toBeUndefined();
  });
});
