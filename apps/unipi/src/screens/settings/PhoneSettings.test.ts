import { describe, expect, it, vi } from 'vitest';
vi.mock('../../components/phone', () => ({ ListGroup: () => null, ListRow: () => null, RowIcon: () => null }));
import { SETTINGS_SECTIONS } from './catalog';
import { phoneSettingsGroups, phoneSectionTitle, pcScoped } from './PhoneSettings';
import { sectionById } from './catalog';

const all = SETTINGS_SECTIONS;
const ids = (g: ReturnType<typeof phoneSettingsGroups>) => Object.fromEntries(g.map((x) => [x.id, x.entries.map((e) => e.id)]));

describe('phone Settings root (§3.9)', () => {
  it('groups: this phone / PC / PCs & connection / about, in spec order', () => {
    const g = ids(phoneSettingsGroups(all, { desktopApp: false, sharing: false, hosts: true }));
    expect(g.phone).toEqual(['appearance', 'input', 'terminal', 'notifications', 'unicrab']);
    expect(g.pc).toEqual(['providers', 'agent', 'profiles', 'skills', 'subagents', 'extensions', 'mcp', 'memory', 'board', 'ports']);
    expect(g.pcs).toEqual(['hosts', 'connection', 'pcsends']);
    expect(g.about).toEqual(['about']);
  });
  it('hides desktop-only sections on a phone (PH-20)', () => {
    const flat = phoneSettingsGroups(all, { desktopApp: false, sharing: false, hosts: true }).flatMap((g) => g.entries.map((e) => e.id));
    for (const id of ['shortcuts', 'general', 'host', 'editor']) expect(flat).not.toContain(id);
  });
  it('a narrow desktop window keeps them in a This computer group', () => {
    const g = phoneSettingsGroups(all, { desktopApp: true, sharing: true, hosts: true });
    expect(g.find((x) => x.id === 'desktop')?.entries.map((e) => e.id)).toEqual(expect.arrayContaining(['shortcuts', 'general']));
    expect(g.find((x) => x.id === 'pcs')?.entries.map((e) => e.id)).toContain('sharing');
  });
  it('sub-pages are routes (PH-09)', () => {
    const g = phoneSettingsGroups(all, { desktopApp: false, sharing: false, hosts: true });
    const profiles = g.flatMap((x) => x.entries).find((e) => e.id === 'profiles');
    expect(profiles?.href).toBe('/settings/subagents/profiles');
    expect(g.flatMap((x) => x.entries).find((e) => e.id === 'ports')?.href).toBe('/settings/ports');
  });
  it('titles and PC scope', () => {
    expect(phoneSectionTitle(sectionById('input'))).toBe('Input & keyboard');
    expect(phoneSectionTitle(sectionById('subagents'), 'profiles')).toBe('Agent profiles');
    expect(pcScoped(sectionById('providers'))).toBe(true);
    expect(pcScoped(sectionById('appearance'))).toBe(false);
  });
});
