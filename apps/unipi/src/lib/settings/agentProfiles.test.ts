import { describe, expect, it } from 'vitest';
import { agentMarkdown, builtinProfiles, defaultProfileId, effectiveDisabled, offeredIds, parseAgent, parseFrontMatter, profileDirs, toggleDisabled, validateName } from './agentProfiles';

describe('agent profiles (owner5 #17, the subagents extension format)', () => {
  it('parses front matter like the extension: scalars, quoted, inline and block lists', () => {
    const raw = '---\nname: reviewer\ndescription: "Reviews: diffs"\nmodel: anthropic/claude-x\nthinking: high\nallowed-tools:\n  - read\n  - grep\nmax-nesting: 1\n---\n\nBe strict.\n';
    const p = parseAgent(raw, 'global', 'file-name', '/x/reviewer.md');
    expect(p).toMatchObject({ id: 'reviewer', description: 'Reviews: diffs', model: 'anthropic/claude-x', thinking: 'high', tools: ['read', 'grep'], maxNesting: 1, systemPrompt: 'Be strict.', source: 'global', path: '/x/reviewer.md' });
    expect(parseFrontMatter('---\ntools: [a, "b"]\n---\nbody').fm.get('tools')).toEqual(['a', 'b']);
    expect(parseAgent('no front matter', 'project', 'plain').id).toBe('plain');
  });

  it('writes the same Markdown the extension writes, and reads it back', () => {
    const md = agentMarkdown({ name: 'reviewer', description: 'Reviews "diffs"', model: 'a/b', thinking: 'low', tools: ['read'], prompt: ' Be strict. ' });
    expect(md).toBe('---\nname: reviewer\ndescription: "Reviews \\"diffs\\""\nmodel: a/b\nthinking: low\nallowed-tools:\n  - read\n---\n\nBe strict.\n');
    expect(parseAgent(md, 'global', 'x')).toMatchObject({ id: 'reviewer', description: 'Reviews "diffs"', tools: ['read'], systemPrompt: 'Be strict.' });
  });

  it('validates names and finds the folders', () => {
    expect(validateName('Bad Name', [])).toMatch(/lowercase/);
    expect(validateName('subagent_explore', [])).toMatch(/built-in/);
    expect(validateName('dup', ['dup'])).toMatch(/already exists/);
    expect(validateName('ok-name_1', [])).toBeUndefined();
    expect(builtinProfiles().map((p) => p.id)).toEqual(['subagent_explore', 'subagent_general']);
    expect(profileDirs('/home/me/.oino/', '/w/app')).toEqual({ global: '/home/me/.oino/state/unipi.subagents/global/profiles', project: '/w/app/.oino/agents' });
  });

  it('agent-disable: offered / default / toggle mirror profiles.rs', () => {
    const ids = ['subagent_explore', 'subagent_general', 'rev'];
    expect(offeredIds(ids, ['subagent_general', 'ghost'])).toEqual(['subagent_explore', 'rev']);
    expect(offeredIds(ids, ids)).toEqual(['subagent_explore']);
    expect(defaultProfileId(ids, [])).toBe('subagent_general');
    expect(defaultProfileId(ids, ['subagent_general'])).toBe('subagent_explore');
    expect(effectiveDisabled({ global: ['a'] })).toEqual(['a']);
    expect(effectiveDisabled({ global: ['a'], project: [] })).toEqual([]);
    expect(toggleDisabled(ids, ['ghost'], 'rev', false)).toEqual({ ok: true, list: ['rev'] });
    expect(toggleDisabled(ids, ['rev'], 'rev', true)).toEqual({ ok: true, list: [] });
    expect(toggleDisabled(ids, ['subagent_explore', 'rev'], 'subagent_general', false)).toEqual({ ok: false, error: 'At least one profile must stay enabled.' });
  });
});
