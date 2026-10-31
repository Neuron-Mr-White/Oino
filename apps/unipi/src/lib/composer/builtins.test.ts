/**
 * WP-A25 acceptance: pi's built-ins route to app surfaces (G-15).
 *
 * One row per command: the plan names a surface, and `runBuiltin` applies it
 * through the real deps (spied here) — so a rename in the table, a missing
 * surface or a command that silently falls back fails this file.
 */
import { describe, expect, it, vi } from 'vitest';
import { BUILTIN_ROUTES, BUG_URL, builtinPlan, builtinWorksWithoutChat, NO_CHAT_NOTICE, routedBuiltinNames, runBuiltin, runSessionBuiltin, SESSION_BUILTINS, type BuiltinDeps } from './builtins';

function deps(overrides: Partial<BuiltinDeps> = {}): BuiltinDeps {
  return {
    chat: { hostId: 'pc-one', sessionId: 's1' },
    openChatAction: vi.fn(() => true),
    openImport: vi.fn(() => true),
    openPickerCycleSet: vi.fn(() => true),
    openSettings: vi.fn(),
    openHotkeys: vi.fn(),
    reportBug: vi.fn(),
    notice: vi.fn(),
    quit: vi.fn(),
    runInChat: vi.fn(() => true),
    ...overrides,
  };
}

/** The audit's G-15 list, verbatim. */
const G15 = ['export', 'import', 'clone', 'share', 'scoped-models', 'login', 'logout', 'settings', 'changelog', 'hotkeys', 'trust', 'bug', 'quit'];

describe('every pi built-in has a route (G-15)', () => {
  it('covers exactly the audit list', () => {
    expect(routedBuiltinNames().sort()).toEqual([...G15].sort());
  });

  it.each(G15)('/%s plans a surface and runs it', (name) => {
    const plan = builtinPlan(name);
    expect(plan, `no plan for /${name}`).toBeTruthy();
    expect(plan!.surface).toMatch(/^(app|chat|settings|route):/);
    expect(runBuiltin(name, deps())).toBe(true);
  });

  it('an unknown command is not intercepted (it stays an extension/prompt command)', () => {
    expect(builtinPlan('unipi:goal')).toBeUndefined();
    expect(runBuiltin('unipi:goal', deps())).toBe(false);
  });
});

describe('what each built-in opens', () => {
  it('/export runs the ⋯ Export action (Z-20: `_oino/export_jsonl`)', () => {
    const d = deps();
    runBuiltin('export', d);
    expect(d.openChatAction).toHaveBeenCalledWith('export');
  });

  it('/clone runs the ⋯ Duplicate action (session_fork at the leaf)', () => {
    const d = deps();
    runBuiltin('clone', d);
    expect(d.openChatAction).toHaveBeenCalledWith('duplicate');
  });

  it('/import opens the Import session… dialog', () => {
    const d = deps();
    runBuiltin('import', d);
    expect(d.openImport).toHaveBeenCalled();
  });

  it('/scoped-models opens the picker on the Cycle set', () => {
    const d = deps();
    runBuiltin('scoped-models', d);
    expect(d.openPickerCycleSet).toHaveBeenCalled();
  });

  it('/login and /logout land on Settings › Providers', () => {
    for (const name of ['login', 'logout']) {
      const d = deps();
      runBuiltin(name, d);
      expect(d.openSettings).toHaveBeenCalledWith('providers', 'sec-providers');
    }
  });

  it('/settings opens Settings; /changelog opens About › What’s new', () => {
    const a = deps();
    runBuiltin('settings', a);
    expect(a.openSettings).toHaveBeenCalledWith('general', undefined);
    const b = deps();
    runBuiltin('changelog', b);
    expect(b.openSettings).toHaveBeenCalledWith('about', 'about-whats-new');
  });

  it('/hotkeys opens the Ctrl+/ list; /quit quits', () => {
    const d = deps();
    runBuiltin('hotkeys', d);
    expect(d.openHotkeys).toHaveBeenCalled();
    const q = deps();
    runBuiltin('quit', q);
    expect(q.quit).toHaveBeenCalled();
  });

  it('/bug lands on About › Report a bug and opens the issue form', () => {
    const d = deps();
    runBuiltin('bug', d);
    expect(d.reportBug).toHaveBeenCalledWith(BUG_URL);
    expect(BUG_URL).toBe('https://github.com/Neuron-Mr-White/Oino/issues/new');
  });

  it('/share and /trust are dropped **with a notice**, not silently', () => {
    for (const name of ['share', 'trust']) {
      const d = deps();
      runBuiltin(name, d);
      expect(d.notice).toHaveBeenCalledTimes(1);
      expect((d.notice as unknown as { mock: { calls: string[][] } }).mock.calls[0]![0]).toMatch(/Export|trust/i);
    }
    // and neither opens a chat-bound pane
    const d = deps();
    runBuiltin('share', d);
    expect(d.openChatAction).not.toHaveBeenCalled();
    expect(d.openImport).not.toHaveBeenCalled();
  });

  it('the R8 wording is stable (the capability pages reuse it)', () => {
    expect(NO_CHAT_NOTICE).toBe('Open a chat first');
  });

  it('the table has no entry without a plan (no dead command-center rows)', () => {
    for (const plan of Object.values(BUILTIN_ROUTES)) {
      expect(plan.title.trim()).not.toBe('');
      expect(plan.action.do).toBeTruthy();
    }
  });
});

describe('P0-02: the chat built-ins run an action', () => {
  const sdeps = () => ({
    model: vi.fn(),
    thinking: vi.fn(),
    compact: vi.fn(),
    newChat: vi.fn(),
    resume: vi.fn(),
    fork: vi.fn(),
    tree: vi.fn(),
    rename: vi.fn(),
    copyLast: vi.fn(),
    sessionInfo: vi.fn(),
  });
  it.each(Object.keys(SESSION_BUILTINS))('/%s runs', (name) => {
    expect(runSessionBuiltin(name, undefined, sdeps())).toBe(true);
  });
  it('passes the argument', () => {
    const d = sdeps();
    runSessionBuiltin('compact', '  keep the API notes ', d);
    expect(d.compact).toHaveBeenCalledWith('keep the API notes');
    runSessionBuiltin('thinking', 'high', d);
    expect(d.thinking).toHaveBeenCalledWith('high');
    runSessionBuiltin('name', '', d);
    expect(d.rename).toHaveBeenCalledWith(undefined);
  });
  it('knows which work before a chat exists', () => {
    expect(builtinWorksWithoutChat('model')).toBe(true);
    expect(builtinWorksWithoutChat('settings')).toBe(true);
    expect(builtinWorksWithoutChat('compact')).toBe(false);
    expect(builtinWorksWithoutChat('export')).toBe(false);
    expect(runSessionBuiltin('unipi:goal', undefined, sdeps())).toBe(false);
  });
});
