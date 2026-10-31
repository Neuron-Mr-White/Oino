import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  bindingForEvent,
  chordMatches,
  chordOf,
  chordTokens,
  displayChord,
  extEntries,
  installShortcutsPersistence,
  normalizeChord,
  rebindShortcut,
  resetShortcuts,
} from './registry';

const key = (init: Partial<KeyboardEvent> & { key: string }): KeyboardEvent => ({ ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...init } as KeyboardEvent);

afterEach(() => resetShortcuts());

describe('chords', () => {
  it('normalizes pi and written forms', () => {
    expect(normalizeChord('ctrl+b')).toBe('Mod+B');
    expect(normalizeChord('Alt+M')).toBe('Alt+M');
    expect(normalizeChord('cmd+shift+p')).toBe('Mod+Shift+P');
    expect(normalizeChord('shift+down')).toBe('Shift+Down');
    expect(normalizeChord('ctrl+alt+c')).toBe('Mod+Alt+C');
    expect(normalizeChord('Mod+/')).toBe('Mod+/');
    expect(normalizeChord('Mod+,')).toBe('Mod+,');
    expect(normalizeChord('Mod+Shift+]')).toBe('Mod+Shift+]');
    expect(normalizeChord('')).toBeUndefined();
    expect(normalizeChord('ctrl+shift')).toBeUndefined();
  });

  it('reads an event (code wins over key for letters)', () => {
    expect(chordOf(key({ key: 'k', code: 'KeyK', ctrlKey: true }))).toBe('Mod+K');
    expect(chordOf(key({ key: 'k', code: 'KeyK', metaKey: true, shiftKey: true }))).toBe('Mod+Shift+K');
    expect(chordOf(key({ key: 'm', code: 'KeyM', altKey: true }))).toBe('Alt+M');
    expect(chordOf(key({ key: 'ArrowDown', code: 'ArrowDown', shiftKey: true }))).toBe('Shift+Down');
    expect(chordOf(key({ key: '/', code: 'Slash', ctrlKey: true }))).toBe('Mod+/');
    expect(chordOf(key({ key: '', code: 'ShiftLeft', shiftKey: true }))).toBeUndefined();
  });

  it('matches chords with exact modifiers', () => {
    const e = key({ key: 'b', code: 'KeyB', ctrlKey: true, shiftKey: true });
    expect(chordMatches('Mod+Shift+B', e)).toBe(true);
    expect(chordMatches('Mod+B', e)).toBe(false);
    expect(chordMatches('Alt+Shift+B', e)).toBe(false);
    // a plain letter chord still matches a plain press
    expect(chordMatches('C', key({ key: 'c', code: 'KeyC' }))).toBe(true);
  });

  it('displays platform tokens', () => {
    expect(chordTokens('Mod+K', false)).toEqual(['Ctrl', 'K']);
    expect(displayChord('Mod+Alt+B', true)).toBe('⌘⌥B');
    expect(displayChord('Alt+M', false)).toBe('Alt+M');
  });
});

describe('registry', () => {
  it('resolves shell chords to their binding ids', () => {
    expect(bindingForEvent(key({ key: 'b', code: 'KeyB', metaKey: true }))).toEqual({ kind: 'app', id: 'shell.sidebar' });
    expect(bindingForEvent(key({ key: 'p', code: 'KeyP', ctrlKey: true }))).toEqual({ kind: 'app', id: 'shell.files' });
    expect(bindingForEvent(key({ key: 'p', code: 'KeyP', ctrlKey: true, shiftKey: true }))).toEqual({ kind: 'app', id: 'shell.palette' });
    expect(bindingForEvent(key({ key: 'f', code: 'KeyF', ctrlKey: true }))).toEqual({ kind: 'app', id: 'find.open' });
    expect(bindingForEvent(key({ key: '1', code: 'Digit1', ctrlKey: true }))).toBeUndefined();
  });

  it('a user chord replaces the default everywhere', () => {
    const write = vi.fn();
    installShortcutsPersistence({ read: () => ({}), write });
    rebindShortcut('shell.sidebar', 'Mod+Shift+S');
    expect(bindingForEvent(key({ key: 's', code: 'KeyS', ctrlKey: true, shiftKey: true }))).toEqual({ kind: 'app', id: 'shell.sidebar' });
    expect(bindingForEvent(key({ key: 'b', code: 'KeyB', ctrlKey: true }))).toBeUndefined();
    expect(write).toHaveBeenCalledWith({ 'shell.sidebar': 'Mod+Shift+S' });
    rebindShortcut('shell.sidebar', null);
    expect(bindingForEvent(key({ key: 'b', code: 'KeyB', ctrlKey: true }))).toEqual({ kind: 'app', id: 'shell.sidebar' });
  });

  it('loads persisted user chords on install', () => {
    installShortcutsPersistence({ read: () => ({ 'find.open': 'Mod+Shift+F2' }), write: () => {} });
    expect(bindingForEvent(key({ key: 'F2', code: 'F2', ctrlKey: true, shiftKey: true }))).toEqual({ kind: 'app', id: 'find.open' });
  });

  it('extension chords are bound but never shadow an app chord', async () => {
    const { setExtensionShortcuts } = await import('./registry');
    setExtensionShortcuts([{ id: 'unipi.permission.cycle', chord: 'alt+m', title: 'Cycle permission mode', extId: 'unipi.workflow' }]);
    expect(bindingForEvent(key({ key: 'm', code: 'KeyM', altKey: true }))).toEqual({ kind: 'ext', id: 'unipi.permission.cycle', extId: 'unipi.workflow', intercepted: false });
    // P0-10: ctrl+b is shared — Mod+B stays the sidebar's binding (which backgrounds a
    // foreground subagent first); no separate Alt+B binding any more
    setExtensionShortcuts([{ id: 'unipi.subagents.foreground', chord: 'ctrl+b', extId: 'unipi.subagents' }]);
    expect(bindingForEvent(key({ key: 'b', code: 'KeyB', ctrlKey: true }))).toEqual({ kind: 'app', id: 'shell.sidebar' });
    expect(bindingForEvent(key({ key: 'b', code: 'KeyB', altKey: true }))).toBeUndefined();
    expect(extEntries().find((e) => e.id === 'unipi.subagents.foreground')).toMatchObject({ chord: 'Mod+B', interceptedBy: 'shell.sidebar' });
  });

  it('the tray shortcut is intercepted natively (never forwarded)', async () => {
    const { setExtensionShortcuts } = await import('./registry');
    setExtensionShortcuts([{ id: 'unipi.tray.open', chord: 'shift+down', extId: 'unipi.core' }]);
    expect(bindingForEvent(key({ key: 'ArrowDown', code: 'ArrowDown', shiftKey: true }))).toEqual({ kind: 'app', id: 'chat.running' });
  });
});
