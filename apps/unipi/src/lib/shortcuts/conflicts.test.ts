import { describe, expect, it } from 'vitest';
import { EXT_ALTERNATES, resolveExtShortcuts } from './conflicts';

const APP = ['Mod+K', 'Mod+B', 'Mod+J', 'Mod+Alt+B', 'Mod+F', 'Mod+Shift+F', 'Mod+/'];

describe('resolveExtShortcuts', () => {
  it('keeps chords that collide with nothing', () => {
    const out = resolveExtShortcuts(APP, [{ id: 'unipi.permission.cycle', chord: 'alt+m', title: 'Cycle permission mode' }]);
    expect(out).toEqual([{ id: 'unipi.permission.cycle', chord: 'Alt+M', title: 'Cycle permission mode', extId: 'extension' }]);
  });

  it('shares Ctrl+B with the sidebar: the subagents shortcut is intercepted by shell.sidebar (P0-10)', () => {
    const out = resolveExtShortcuts(APP, [{ id: 'unipi.subagents.foreground', chord: 'ctrl+b', extId: 'unipi.subagents' }], {}, (id) => (id === 'shell.sidebar' ? 'Mod+B' : undefined));
    expect(out[0]!.interceptedBy).toBe('shell.sidebar');
    expect(out[0]!.chord).toBe('Mod+B');
    expect(out[0]!.note).toContain('foreground');
    expect(EXT_ALTERNATES['unipi.subagents.foreground']).toBeNull();
    // a rebound sidebar takes the shared chord with it
    const moved = resolveExtShortcuts(['Mod+Shift+B'], [{ id: 'unipi.subagents.foreground', chord: 'ctrl+b' }], {}, () => 'Mod+Shift+B');
    expect(moved[0]!.chord).toBe('Mod+Shift+B');
  });

  it('marks the tray shortcut as intercepted (Shift+Down is the app\'s)', () => {
    const out = resolveExtShortcuts(APP, [{ id: 'unipi.tray.open', chord: 'shift+down', extId: 'unipi.core' }]);
    expect(out[0]!.interceptedBy).toBe('chat.running');
    expect(out[0]!.chord).toBe('Shift+Down');
  });

  it('an unknown collision gets a free app-chosen alternate', () => {
    const out = resolveExtShortcuts([...APP, 'Alt+J'], [{ id: 'ext.one', chord: 'alt+j' }, { id: 'ext.two', chord: 'mod+f' }]);
    expect(out[0]!.chord).toBe('Alt+Shift+J'); // Alt+J taken → next alternate
    expect(out[1]!.chord).toBe('Alt+F'); // collides with Mod+F
    expect(out[1]!.reboundFrom).toBe('Mod+F');
  });

  it('no extension chord shadows an app chord (property, many specs)', () => {
    const specs = Array.from({ length: 40 }, (_, i) => ({ id: `x.${i}`, chord: ['mod+k', 'mod+b', 'mod+j', 'alt+m', 'shift+down', 'mod+,'][i % 6]! }));
    const out = resolveExtShortcuts(APP, specs);
    const bound = out.filter((e) => !e.interceptedBy && e.chord);
    expect(bound.length).toBeGreaterThan(0);
    for (const entry of bound) {
      for (const app of APP) expect(entry.chord).not.toBe(app);
    }
    const chords = bound.map((e) => e.chord);
    expect(new Set(chords).size).toBe(chords.length); // unique, too
  });

  it('a user-rebound extension chord wins over the automatic alternate', () => {
    const out = resolveExtShortcuts(APP, [{ id: 'unipi.subagents.foreground', chord: 'ctrl+b' }], { 'ext:unipi.subagents.foreground': 'Alt+Shift+B' });
    expect(out[0]!.chord).toBe('Alt+Shift+B');
    expect(out[0]!.reboundFrom).toBeUndefined();
  });

  it('a user chord that lands on an app chord is ignored', () => {
    const out = resolveExtShortcuts(APP, [{ id: 'unipi.permission.cycle', chord: 'alt+m' }], { 'ext:unipi.permission.cycle': 'mod+b' });
    expect(out[0]!.chord).toBe('Alt+M');
  });
});
