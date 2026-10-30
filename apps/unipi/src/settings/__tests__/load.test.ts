import { describe, it, expect } from 'vitest';
import { loadSettings, loadSettingsFromAdapter } from '../load';
import { createMemoryAdapter } from '../adapter';
import { DEFAULT_SETTINGS, SETTINGS_GROUP_SCHEMAS, SettingsV1Schema } from '../schema';

describe('loadSettings', () => {
  it('returns exact defaults when the adapter has nothing stored', async () => {
    const adapter = createMemoryAdapter(undefined);
    const result = await loadSettingsFromAdapter(adapter);
    expect(result.settings).toEqual(DEFAULT_SETTINGS);
    expect(result.resetKeys).toEqual([]);
    expect(result.migrated).toBe(false);
    expect(result.newerVersion).toBe(false);
  });

  it('falls back per-key on 2 bad keys while preserving the rest', async () => {
    const adapter = createMemoryAdapter({
      version: 1,
      terminal: {
        fontSize: 999, // invalid: out of range
        lineHeight: 1.5, // valid, should be preserved
        cursorStyle: 'nonsense', // invalid enum
        scrollback: 1234, // valid, should be preserved
      },
      input: { haptics: false },
      connection: { pathPreference: 'prefer_ssh' },
      board: { showBacklog: true },
    });
    const result = await loadSettingsFromAdapter(adapter);

    expect(result.settings.terminal.fontSize).toBe(DEFAULT_SETTINGS.terminal.fontSize);
    expect(result.settings.terminal.cursorStyle).toBe(DEFAULT_SETTINGS.terminal.cursorStyle);
    // Good sibling keys in the same group must be preserved, not reset.
    expect(result.settings.terminal.lineHeight).toBe(1.5);
    expect(result.settings.terminal.scrollback).toBe(1234);
    // Other groups untouched.
    expect(result.settings.input.haptics).toBe(false);
    expect(result.settings.connection.pathPreference).toBe('prefer_ssh');
    expect(result.settings.board.showBacklog).toBe(true);

    expect(result.resetKeys).toContain('terminal.fontSize');
    expect(result.resetKeys).toContain('terminal.cursorStyle');
    expect(result.resetKeys).toHaveLength(2);
  });

  it('migrates v0 terminal.font to terminal.fontSize', async () => {
    const adapter = createMemoryAdapter({
      version: 0,
      terminal: { font: 18 },
    });
    const result = await loadSettingsFromAdapter(adapter);
    expect(result.migrated).toBe(true);
    expect(result.settings.terminal.fontSize).toBe(18);
    expect(result.newerVersion).toBe(false);
  });

  it('is lenient with a newer version: parses what it can and flags it', async () => {
    const adapter = createMemoryAdapter({
      version: 999,
      terminal: { fontSize: 20 },
      someFutureKey: { whatever: true },
    });
    const result = await loadSettingsFromAdapter(adapter);
    expect(result.newerVersion).toBe(true);
    // Still produces a usable settings object (per-key fallback over the raw blob).
    expect(result.settings.terminal.fontSize).toBe(20);
    expect(result.settings.board).toEqual(DEFAULT_SETTINGS.board);
  });
});

it('loads raw data synchronously', () => { expect(loadSettings({}).settings).toEqual(DEFAULT_SETTINGS); expect(loadSettings({version: 2}).notice).toBe('newer_version'); });

describe('every settings group round-trips (UNI-286)', () => {
  /** Non-default but still valid values for the keys whose schema constrains
   *  them (enums, ranges); everything else is twisted below. */
  const OVERRIDES: Record<string, unknown> = {
    'terminal.cursorStyle': 'bar',
    'terminal.sizePolicy': 'keep_pc',
    'terminal.lineHeight': 1.5,
    'input.toolbar': ['history', 'shortcuts', 'arrows', 'opt', 'tab', 'esc', 'shift', 'ctrl'],
    'connection.relay': { mode: 'n0' },
    'connection.pathPreference': 'prefer_ssh',
  };

  /** A valid value that is certainly not the default, for any JSON value. */
  function twist(path: string, value: unknown): unknown {
    if (path in OVERRIDES) return OVERRIDES[path];
    if (typeof value === 'boolean') return !value;
    if (typeof value === 'number') return value + 1;
    if (typeof value === 'string') return `${value}-x`;
    if (value === null) return 'set';
    if (Array.isArray(value)) return [...value].reverse();
    if (value && typeof value === 'object') {
      return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, twist(`${path}.${k}`, v)]));
    }
    return value;
  }

  /** Every key of every group (notify included) set to a non-default value. */
  function twistedBlob(): Record<string, unknown> {
    const { version: _version, ...groups } = DEFAULT_SETTINGS;
    return {
      version: 1,
      ...Object.fromEntries(Object.entries(groups).map(([g, v]) => [g, twist(g, v)])),
    };
  }

  it('SETTINGS_GROUP_SCHEMAS lists every SettingsV1 group', () => {
    const groups = Object.keys(DEFAULT_SETTINGS).filter((g) => g !== 'version');
    expect(groups).toContain('notify');
    for (const group of groups) expect(SETTINGS_GROUP_SCHEMAS).toHaveProperty(group);
    // ...and nothing that isn't a group (e.g. `version`).
    expect(Object.keys(SETTINGS_GROUP_SCHEMAS).sort()).toEqual([...groups].sort());
  });

  it('persists and reloads a non-default value for every key of every group', async () => {
    const blob = twistedBlob();
    // Sanity: the blob really is different from the defaults everywhere.
    expect(blob).not.toEqual({ ...DEFAULT_SETTINGS });
    expect(blob).toMatchObject({ notify: { enabled: true, key: '-x' } });

    const adapter = createMemoryAdapter(blob);
    const first = await loadSettingsFromAdapter(adapter);
    expect(first.resetKeys).toEqual([]);
    expect(first.migrated).toBe(false);

    // Persist what we read and read it back through the same adapter.
    await adapter.write(first.settings);
    const second = await loadSettingsFromAdapter(adapter);
    expect(second.resetKeys).toEqual([]);
    expect(second.settings).toEqual(first.settings);
    expect(second.settings).toEqual(SettingsV1Schema.parse(blob));
    // The notification settings in particular survive.
    expect(second.settings.notify).toEqual({
      enabled: true,
      serverUrl: 'https://ntfy.sh-x',
      topic: '-x',
      token: '-x',
      keepListening: true,
      key: '-x',
      keySourceHostId: 'set',
    });
  });
});
