import { describe, expect, it } from 'vitest';
import { dialogShownInApp, entryShownInApp, parseSurfaces, shownInApp } from './surfaces';
import { appHidden, appNamespaces, parseNamespaces } from '../ui/settings/types';

describe('[gui-cleanup] client surfaces', () => {
  it('parses the wire list; absent = every client', () => {
    expect(parseSurfaces(undefined)).toBeUndefined();
    expect(parseSurfaces(['tui', 'nope'])).toEqual(['tui']);
    expect(shownInApp(undefined)).toBe(true);
    expect(shownInApp(['gui'])).toBe(true);
    expect(shownInApp(['tui'])).toBe(false);
    expect(shownInApp([])).toBe(false);
  });

  it('settings: terminal-only fields go, an all-terminal namespace goes whole, page drill-ins are filtered', () => {
    const raw = [
      {
        namespace: 'oino.agent', title: 'Session & queue', extensionId: 'oino',
        sections: [{ id: 'agent', title: 'Agent', fields: [
          { key: 'steeringMode', type: 'enum', label: 'Steering', options: ['all'] },
          { key: 'quietStartup', type: 'boolean', label: 'Quiet startup', surfaces: ['tui'] },
        ] }],
      },
      {
        namespace: 'oino.tui', title: 'Terminal UI', extensionId: 'oino',
        sections: [{ id: 'tui', title: 'TUI', fields: [
          { key: 'tuiMode', type: 'enum', label: 'TUI mode', options: ['fullscreen'], surfaces: ['tui'] },
          { key: 'theme', type: 'enum', label: 'Theme', options: ['dark'], surfaces: ['tui'] },
        ] }],
      },
      {
        namespace: 'ask-user', title: 'Ask User', extensionId: 'unipi.ask-user',
        sections: [
          { id: 'tool', title: 'Tool', fields: [{ key: 'enabled', type: 'boolean', label: 'Enable' }] },
          { id: 'dialog', title: 'Dialog', fields: [{ key: 'escape', type: 'enum', label: 'Esc', options: ['stop'], surfaces: ['tui'] }] },
          { id: 'more', title: 'More', fields: [{ key: 'p', type: 'page', label: 'Page…', sections: [
            { id: 'in', title: 'In', fields: [{ key: 'helpLine', type: 'boolean', label: 'Help', surfaces: ['tui'] }, { key: 'other', type: 'boolean', label: 'Other' }] },
          ] }] },
        ],
      },
    ];
    const list = appNamespaces(parseNamespaces(raw));
    expect(list.map((ns) => ns.namespace)).toEqual(['oino.agent', 'ask-user']);
    expect(list[0]!.sections[0]!.fields.map((f) => f.key)).toEqual(['steeringMode']);
    expect(list[1]!.sections.map((s) => s.id)).toEqual(['tool', 'more']);
    expect(list[1]!.sections[1]!.fields[0]!.sections[0]!.fields.map((f) => f.key)).toEqual(['other']);
    // The form's own guard reads the same flag (and the pre-surfaces quietStartup fallback).
    expect(appHidden('ask-user', 'escape', { surfaces: ['tui'] })).toBe(true);
    expect(appHidden('oino.agent', 'quietStartup')).toBe(true);
    expect(appHidden('ask-user', 'enabled', {})).toBe(false);
  });
});

describe('[ask-user] entries, effects and dialogs per client', () => {
  it('registry entries: surfaces decide; older terminal-only builds are known by id', () => {
    expect(entryShownInApp({ id: 'ext/widget:a' })).toBe(true);
    expect(entryShownInApp({ id: 'ext/widget:a', surfaces: ['tui'] })).toBe(false);
    expect(entryShownInApp({ id: 'ext/widget:a', surfaces: ['gui', 'tui'] })).toBe(true);
    expect(entryShownInApp({ id: 'unipi.utility/widget:unipi-answer-hint' })).toBe(false);
    expect(entryShownInApp({ id: 'unipi.core/tray' })).toBe(false);
    expect(entryShownInApp({ id: 'unipi.core/widget:work-tray-strip' })).toBe(true);
  });

  it('dialogs: /answer\'s editor never opens in the app (surfaces, or its old titles)', () => {
    expect(dialogShownInApp({ method: 'editor', title: 'x', surfaces: ['tui'] })).toBe(false);
    expect(dialogShownInApp({ method: 'editor', extensionId: 'unipi.utility', title: 'Answer 1 question — Tab next answer · external editor available' })).toBe(false);
    expect(dialogShownInApp({ method: 'custom', extensionId: 'unipi.utility', title: '/answer · reply' })).toBe(false);
    expect(dialogShownInApp({ method: 'editor', extensionId: 'other', title: 'Answer 1 question — Tab next answer' })).toBe(true);
    expect(dialogShownInApp({ method: 'form', extensionId: 'unipi.ask-user', title: 'DB' })).toBe(true);
  });
});
