/**
 * WP-A25 acceptance: the audit's reachability table, walked as data.
 *
 * Every row of zcode-audit "Unipi feature reachability" must name at least
 * one surface, and every surface must resolve **to code in this tree** — the
 * test imports the module each `app:`/`slot:` surface claims as evidence, so
 * a moved or renamed surface fails the gate instead of silently regressing.
 */
import { describe, expect, it } from 'vitest';
import {
  APP_SURFACES,
  BUILTIN_REACHABILITY,
  REACHABILITY,
  SLOT_SURFACES,
  builtinRouteMismatches,
  reachabilityRows,
  surfaceResolves,
  unresolvedSurfaces,
} from './reachability';
import { MANAGE_ACTIONS } from './settings/manage';
import { BUILTIN_ROUTES } from './composer/builtins';

/** Import `../<module>#export` (or just the module when there is no `#`). */
async function evidenceExists(evidence: string): Promise<boolean> {
  const [path, name] = evidence.split('#');
  try {
    const mod = (await import(/* @vite-ignore */ path!)) as Record<string, unknown>;
    return name ? mod[name] !== undefined : true;
  } catch {
    return false;
  }
}

describe('the audit table has no gap', () => {
  it('every row names at least one surface', () => {
    for (const row of reachabilityRows()) expect(row.surfaces.length, row.id).toBeGreaterThan(0);
  });

  it('every surface resolves (settings page, tab kind, route, ⋯ row, slot, app action)', () => {
    expect(unresolvedSurfaces()).toEqual([]);
  });

  it('covers the audit’s 21 extension rows plus one row per pi built-in', () => {
    expect(REACHABILITY).toHaveLength(21);
    expect(BUILTIN_REACHABILITY).toHaveLength(13);
  });
});

describe('evidence: a surface id points at code that exists', () => {
  it.each(APP_SURFACES.map((s) => [s.id, s.evidence]))('app:%s → %s', async (_id, evidence) => {
    expect(await evidenceExists(evidence)).toBe(true);
  });

  it.each(SLOT_SURFACES.map((s) => [s.id, s.evidence]))('slot:%s → %s', async (_id, evidence) => {
    expect(await evidenceExists(evidence)).toBe(true);
  });
});

describe('G-15: routing and the audit table cannot drift', () => {
  it('each built-in row names the surface its route opens', () => {
    expect(builtinRouteMismatches()).toEqual([]);
  });

  it('every built-in has a route and every route has a row', () => {
    expect(Object.keys(BUILTIN_ROUTES).sort()).toEqual(BUILTIN_REACHABILITY.map((row) => row.id.replace(/^pi:/, '')).sort());
  });
});

describe('G-8..G-11: the capability pages and their Manage actions', () => {
  it('each page has at least one Manage action whose command is real', () => {
    for (const page of ['mcp', 'memory', 'subagents'] as const) { // Skills: the native manager (no command)
      const actions = MANAGE_ACTIONS.filter((action) => action.page === page);
      expect(actions.length, page).toBeGreaterThan(0);
      for (const action of actions) {
        // A command, optionally with its sub-command (`dream run`, P1-07).
        expect(action.command).toMatch(/^[a-z][a-z0-9-]*( [a-z][a-z0-9-]*)?$/);
        expect(action.title.trim()).not.toBe('');
      }
    }
  });

  it('the four capability pages are Settings sections of the Agent group', async () => {
    const { SETTINGS_SECTIONS } = await import('../screens/settings/catalog');
    for (const page of ['mcp', 'memory', 'subagents'] as const) { // Skills: the native manager (no command)
      const section = SETTINGS_SECTIONS.find((s) => s.id === page);
      expect(section, page).toBeTruthy();
      expect(section!.group).toBe('agent');
    }
    expect(SETTINGS_SECTIONS.some((s) => s.id === 'providers')).toBe(true);
    expect(surfaceResolves('settings:mcp')).toBe(true);
  });

  it('every Manage action is also a command-center entry (one action, two entry points)', async () => {
    const mod = await import('../components/command/BuiltinActions');
    expect(mod.BuiltinActions).toBeTypeOf('function');
    // The component registers `manage.<id>` for every row (see its `registerCommands`).
    expect(MANAGE_ACTIONS.every((action) => `manage.${action.id}`.startsWith('manage.'))).toBe(true);
  });
});
