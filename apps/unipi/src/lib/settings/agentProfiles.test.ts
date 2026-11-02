import { describe, expect, it } from 'vitest';
import { agentMarkdown, builtinProfiles, parseAgent, parseFrontMatter, profileDirs, validateName } from './agentProfiles';

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
});
