import { describe, expect, it } from 'vitest';
import { DEFAULT_TOOLBAR_KEYS, LEGACY_DEFAULT_TOOLBAR_KEYS } from '../schema';
import { migrateModifierIcons, migrateToolbarDefault } from '../migrations';
import { loadSettings } from '../load';
import { DEFAULT_SETTINGS } from '../schema';

describe('migrateToolbarDefault', () => {
  it('upgrades the never-customised legacy default to the new list', () => {
    const raw = { version: 1, input: { toolbar: [...LEGACY_DEFAULT_TOOLBAR_KEYS] } };
    const next = migrateToolbarDefault(raw);
    expect((next.input as { toolbar: string[] }).toolbar).toEqual(DEFAULT_TOOLBAR_KEYS);
  });

  it('keeps a customised order', () => {
    const custom = ['ctrl', 'esc', 'arrows'];
    const raw = { version: 1, input: { toolbar: [...custom] } };
    const next = migrateToolbarDefault(raw);
    expect((next.input as { toolbar: string[] }).toolbar).toEqual(custom);
  });

  it('leaves blobs without input untouched', () => {
    const raw = { version: 1 };
    expect(migrateToolbarDefault(raw)).toEqual({ version: 1 });
  });
});

describe('migrateModifierIcons (UNI-127)', () => {
  it('adds the text-label default to an existing input group', () => {
    const raw = { version: 1, input: { haptics: false, toolbar: ['ctrl', 'esc'] } };
    const next = migrateModifierIcons(raw);
    expect(next.input).toEqual({ haptics: false, toolbar: ['ctrl', 'esc'], modifierIcons: false });
  });

  it('keeps an explicit choice', () => {
    const raw = { version: 1, input: { modifierIcons: true } };
    expect((migrateModifierIcons(raw).input as { modifierIcons: boolean }).modifierIcons).toBe(true);
  });

  it('leaves blobs without input untouched and loads with the default', () => {
    expect(migrateModifierIcons({ version: 1 })).toEqual({ version: 1 });
    expect(DEFAULT_SETTINGS.input.modifierIcons).toBe(false);
    const loaded = loadSettings({ version: 1, input: { haptics: true } });
    expect(loaded.settings.input.modifierIcons).toBe(false);
    expect(loadSettings({ version: 1, input: { modifierIcons: true } }).settings.input.modifierIcons).toBe(true);
  });
});
