import { describe, it, expect } from 'vitest';
import { SettingsV1Schema, DEFAULT_SETTINGS } from '../schema';

describe('SettingsV1Schema', () => {
  it('parses {} into exact spec defaults', () => {
    const parsed = SettingsV1Schema.parse({});
    expect(parsed).toEqual({
      version: 1,
      terminal: {
        fontSize: 14,
        lineHeight: 1.2,
        cursorStyle: 'block',
        cursorBlink: true,
        scrollback: 5000,
        sizePolicy: 'fit_phone',
        keepScreenOn: false,
        reopenLast: true,
      },
      input: {
        haptics: true,
        composerAutocorrect: true,
        modifierIcons: false,
        toolbar: ['ctrl', 'shift', 'esc', 'tab', 'opt', 'arrows', 'shortcuts', 'history'],
      },
      connection: {
        relay: { mode: 'n0' },
        pathPreference: 'auto',
      },
      board: {
        defaultProject: null,
        showBacklog: false,
      },
      notify: {
        enabled: false,
        serverUrl: 'https://ntfy.sh',
        topic: '',
        token: '',
        keepListening: false,
        key: '',
        keySourceHostId: null,
      },
      editor: {
        vim: false,
      },
    });
    expect(parsed).toEqual(DEFAULT_SETTINGS);
  });

  it('allows ctrl anywhere and every supported extra key', () => {
    const r = SettingsV1Schema.safeParse({ input: { toolbar: ['esc', 'ctrl', 'enter', 'backspace', 'home', 'end', 'pgup', 'pgdn'] } });
    expect(r.success).toBe(true);
    expect(SettingsV1Schema.safeParse({ input: { toolbar: ['esc'] } }).success).toBe(false);
  });

  it('rejects duplicate toolbar keys', () => {
    const r = SettingsV1Schema.safeParse({ input: { toolbar: ['ctrl', 'ctrl'] } });
    expect(r.success).toBe(false);
  });

  it('accepts a custom https relay url', () => {
    const r = SettingsV1Schema.safeParse({ connection: { relay: { mode: 'custom', url: 'https://relay.example.com' } } });
    expect(r.success).toBe(true);
  });

  it('rejects a non-https custom relay url', () => {
    const r = SettingsV1Schema.safeParse({ connection: { relay: { mode: 'custom', url: 'http://relay.example.com' } } });
    expect(r.success).toBe(false);
  });

  it('rejects out-of-range fontSize', () => {
    expect(SettingsV1Schema.safeParse({ terminal: { fontSize: 7 } }).success).toBe(false);
    expect(SettingsV1Schema.safeParse({ terminal: { fontSize: 33 } }).success).toBe(false);
  });
});

describe('settings: v1 terminal-multiplexer keys (D-14)', () => {
  it('drops the removed herdr/tmux attach options from saved data', () => {
    const parsed = SettingsV1Schema.parse({ version: 1, terminal: { fontSize: 16, tmuxFriendlyOptions: false, herdrFollowFocus: false } });
    expect(parsed.terminal).not.toHaveProperty('herdrFollowFocus');
    expect(parsed.terminal).not.toHaveProperty('tmuxFriendlyOptions');
    expect(parsed.terminal.fontSize).toBe(16);
  });
});
