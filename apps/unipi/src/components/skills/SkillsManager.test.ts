import { describe, expect, it } from 'vitest';
import { effective, parseCatalog, setState } from './SkillsManager';

describe('skills manager rules', () => {
  const row = { name: 'browser', description: '', source: 'user', defaultEnabled: true, defaultMustShow: false };
  it('project over global over defaults; must show forces enabled', () => {
    expect(effective(row, 'global', {}, {})).toEqual({ enabled: true, mustShow: false, from: 'default' });
    expect(effective(row, 'project', { browser: { enabled: false } }, {})).toMatchObject({ enabled: false, from: 'global' });
    expect(effective(row, 'project', { browser: { enabled: false } }, { browser: { must_show: true } })).toEqual({ enabled: true, mustShow: true, from: 'project' });
    expect(effective(row, 'global', { browser: { enabled: false } }, { browser: { enabled: true } }).enabled).toBe(false);
  });
  it('setState drops empty rows (inherit)', () => {
    const a = setState({}, 'x', 'enabled', false);
    expect(a).toEqual({ x: { enabled: false } });
    expect(setState(a, 'x', 'enabled', undefined)).toEqual({});
  });
  it('parses the service reply leniently', () => {
    const c = parseCatalog({ skills: [{ name: 'a', source: 'vault', defaultEnabled: false }, { nope: 1 }], global: { a: { enabled: true, junk: 1 } }, proxy: false });
    expect(c.skills).toHaveLength(1);
    expect(c.skills[0]).toMatchObject({ name: 'a', defaultEnabled: false });
    expect(c.global).toEqual({ a: { enabled: true } });
    expect(c.proxy).toBe(false);
  });
  it('[owner-fix-8] the skill proxy is off unless the reply says it is on', () => {
    // A missing flag is OFF (the proxy is opt-in), not ON.
    expect(parseCatalog({ skills: [] }).proxy).toBe(false);
    expect(parseCatalog({ skills: [], proxy: undefined }).proxy).toBe(false);
    expect(parseCatalog({ skills: [], proxy: null }).proxy).toBe(false);
    expect(parseCatalog({ skills: [], proxy: false }).proxy).toBe(false);
    expect(parseCatalog({ skills: [], proxy: true }).proxy).toBe(true);
  });
});
