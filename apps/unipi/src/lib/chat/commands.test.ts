import { describe, expect, it } from 'vitest';
import { applyAt, atQuery, rankCommands, slashQuery } from './commands';

const cmds = [
  { name: 'model', source: 'builtin' as const },
  { name: 'compact', source: 'builtin' as const },
  { name: 'unipi:goal', description: 'Set a goal', source: 'extension' as const },
  { name: 'unipi:kanboard-do', source: 'extension' as const },
  { name: 'skill:find-docs', source: 'skill' as const },
  { name: 'skill:apply-docs', source: 'skill' as const },
  { name: 'release-notes', source: 'prompt' as const },
];

describe('slash commands', () => {
  it('detects /query only as the first word', () => {
    expect(slashQuery('/un', 3)).toEqual({ start: 0, query: 'un' });
    expect(slashQuery('  /', 3)).toEqual({ start: 2, query: '' });
    expect(slashQuery('hi /un', 6)).toBeNull();
    expect(slashQuery('/a b', 4)).toBeNull();
    expect(slashQuery('see https://x/y', 15)).toBeNull();
  });
  it('ranks prefix, word prefix, substring, subsequence', () => {
    expect(rankCommands(cmds, 'go').map((c) => c.name)).toEqual(['unipi:goal']);
    expect(rankCommands(cmds, 'kb').map((c) => c.name)).toEqual(['unipi:kanboard-do']);
    expect(rankCommands(cmds, 'c')[0]!.name).toBe('compact');
    expect(rankCommands(cmds, '').length).toBe(7);
    expect(rankCommands(cmds, 'zzz')).toEqual([]);
  });
  it('ties (same score) sort by source group — builtin, extension, prompt, skill — then alphabetically within a group', () => {
    // An empty query scores every command 0: group order + name alone decide.
    expect(rankCommands(cmds, '').map((c) => `${c.source}:${c.name}`)).toEqual([
      'builtin:compact',
      'builtin:model',
      'extension:unipi:goal',
      'extension:unipi:kanboard-do',
      'prompt:release-notes',
      'skill:skill:apply-docs',
      'skill:skill:find-docs',
    ]);
  });
});

describe('@ files', () => {
  it('finds the @query at the caret only when @ starts a token', () => {
    expect(atQuery('look at @src/ch', 15)).toEqual({ start: 8, query: 'src/ch' });
    expect(atQuery('@', 1)).toEqual({ start: 0, query: '' });
    expect(atQuery('mail me@home', 12)).toBeNull();
    expect(atQuery('see @src now', 12)).toBeNull();
    expect(atQuery('open @"my dir/no', 16)).toEqual({ start: 5, query: '"my dir/no' });
  });
  it('replaces the query: files get a space, directories keep going', () => {
    const t = 'look at @ch please';
    const at = atQuery(t, 11)!;
    expect(applyAt(t, 11, at, '@src/chat/ChatView.tsx', false)).toEqual({ text: 'look at @src/chat/ChatView.tsx please', caret: 31 });
    expect(applyAt('@sr', 3, { start: 0 }, '@src/', true)).toEqual({ text: '@src/', caret: 5 });
    // quoted directory: the caret stays inside the quotes
    expect(applyAt('@my', 3, { start: 0 }, '@"my dir/"', true)).toEqual({ text: '@"my dir/"', caret: 9 });
    // then a file inside it replaces the whole quoted token
    expect(applyAt('@"my dir/"', 9, { start: 0 }, '@"my dir/notes.md"', false)).toEqual({ text: '@"my dir/notes.md" ', caret: 19 });
  });
});

describe('UNI-276: every command is suggested', () => {
  it('adds pi built-ins the session list lacks; a same-named extension command wins', async () => {
    const { allCommands, PI_BUILTINS } = await import('./commands');
    const session = [
      { name: 'model', source: 'builtin' as const },
      { name: 'name', description: 'mine', source: 'extension' as const },
    ];
    const all = allCommands(session);
    expect(all.filter((c) => c.name === 'name')).toEqual([{ name: 'name', description: 'mine', source: 'extension' }]);
    expect(all.length).toBe(session.length + PI_BUILTINS.length - 2);
    expect(all.some((c) => c.name === 'settings')).toBe(true);
  });
});

describe('UNI-308 / WP-A25: every PI_BUILTINS entry has an app surface', () => {
  it('APP_HANDLED_BUILTINS \u222a BUILTIN_ROUTES covers exactly PI_BUILTINS \u2014 none falls through to a prompt', async () => {
    const { APP_HANDLED_BUILTINS, PI_BUILTINS } = await import('./commands');
    const { routedBuiltinNames } = await import('../composer/builtins');
    const union = new Set([...APP_HANDLED_BUILTINS, ...routedBuiltinNames()]);
    for (const c of PI_BUILTINS) expect(union.has(c.name)).toBe(true);
    expect(union.size).toBe(PI_BUILTINS.length);
  });
});

describe('UNI-308: isKnownCommand', () => {
  it('a session command (extension/prompt/skill) is known even if not in PI_BUILTINS', async () => {
    const { isKnownCommand } = await import('./commands');
    expect(isKnownCommand('unipi:goal', cmds)).toBe(true);
    expect(isKnownCommand('skill:find-docs', cmds)).toBe(true);
    expect(isKnownCommand('release-notes', cmds)).toBe(true);
  });
  it('a pi built-in is known (routed or app-handled)', async () => {
    const { isKnownCommand } = await import('./commands');
    expect(isKnownCommand('compact', cmds)).toBe(true);
    // No reload RPC: /reload is not a command the app offers.
    expect(isKnownCommand('reload', cmds)).toBe(false);
    expect(isKnownCommand('settings', cmds)).toBe(true);
  });
  it('anything else is unknown', async () => {
    const { isKnownCommand } = await import('./commands');
    expect(isKnownCommand('made-up-command', cmds)).toBe(false);
  });
});

describe('UNI-276: argument position', () => {
  it('argQuery finds `/cmd <prefix>` on one line only', async () => {
    const { argQuery } = await import('./commands');
    expect(argQuery('/unipi:dream st', 15)).toEqual({ command: 'unipi:dream', start: 13, prefix: 'st' });
    expect(argQuery('/unipi:dream ', 13)).toEqual({ command: 'unipi:dream', start: 13, prefix: '' });
    expect(argQuery('/unipi:dream', 12)).toBeNull();
    expect(argQuery('hi /x y', 7)).toBeNull();
    expect(argQuery('/x a\nb', 6)).toBeNull();
  });

  it("wantsArgCompletions (UNI-304): extension commands and skills (/skill:name) are worth asking; prompts and builtins aren't (/model etc. have their own UI)", async () => {
    const { wantsArgCompletions } = await import('./commands');
    expect(wantsArgCompletions('extension')).toBe(true);
    expect(wantsArgCompletions('skill')).toBe(true);
    expect(wantsArgCompletions('prompt')).toBe(false);
    expect(wantsArgCompletions('builtin')).toBe(false);
  });
});

describe('skillQuery / applySkill (owner spec 2026-11-03)', () => {
  it('opens on $ at the start, after whitespace or a bracket; never on prices or inside words', async () => {
    const { skillQuery, applySkill } = await import('./commands');
    expect(skillQuery('$', 1)).toEqual({ start: 0, query: '' });
    expect(skillQuery('fix with $rev', 13)).toEqual({ start: 9, query: 'rev' });
    expect(skillQuery('(see $pdf', 9)).toEqual({ start: 5, query: 'pdf' });
    expect(skillQuery('costs $5', 8)).toBeNull();
    expect(skillQuery('a$b', 3)).toBeNull();
    expect(skillQuery('$review done', 12)).toBeNull();
    expect(applySkill('fix with $rev now', 13, { start: 9 }, 'review')).toEqual({ text: 'fix with $review now', caret: 17 });
  });
});
