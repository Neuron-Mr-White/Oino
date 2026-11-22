import { describe, expect, it } from 'vitest';
import { breadcrumbs, entrySubtitle, fileFamily, folderView, placesFrom, tildify } from './phone';

const e = (name: string, kind: 'file' | 'dir' = 'file', mtime = 0, size = 0) => ({ name, kind, mtime, size, ignored: false }) as never;

describe('phone Files helpers (§3.11)', () => {
  it('places: Home, deduped projects (not home), other roots', () => {
    const p = placesFrom(['/home/u', '/', '/tmp'], ['/home/u/app', '/home/u/app/', '/home/u', '/srv/x']);
    expect(p.home?.label).toBe('Home');
    expect(p.projects.map((x) => x.path)).toEqual(['/home/u/app', '/srv/x']);
    expect(p.projects[0]!.detail).toBe('~/app');
    expect(p.other.map((x) => x.path)).toEqual(['/', '/tmp']);
  });
  it('folder view: folders first, dotfiles hidden unless asked, sort by name or date', () => {
    const list = [e('b.txt', 'file', 1), e('.git', 'dir'), e('a', 'dir'), e('A10.txt', 'file', 3), e('A9.txt', 'file', 2)];
    expect(folderView(list, { hidden: false, sort: 'name' }).map((x: { name: string }) => x.name)).toEqual(['a', 'A9.txt', 'A10.txt', 'b.txt']);
    expect(folderView(list, { hidden: true, sort: 'modified' }).map((x: { name: string }) => x.name)).toEqual(['.git', 'a', 'A10.txt', 'A9.txt', 'b.txt']);
  });
  it('breadcrumbs start at the place (or ~)', () => {
    expect(breadcrumbs('/home/u/app/src', '/home/u/app', '/home/u').map((c) => c.label)).toEqual(['app', 'src']);
    expect(breadcrumbs('/home/u/app/src', undefined, '/home/u').map((c) => c.label)).toEqual(['~', 'app', 'src']);
    expect(breadcrumbs('/tmp/x', undefined, '/home/u').map((c) => c.path)).toEqual(['/', '/tmp', '/tmp/x']);
  });
  it('subtitles and icons', () => {
    const now = new Date(2026, 0, 10, 12).getTime();
    expect(entrySubtitle(e('a', 'file', new Date(2026, 0, 9, 8).getTime(), 12_000), now)).toMatch(/KB · Yesterday$/);
    expect(entrySubtitle(e('d', 'dir', new Date(2026, 0, 9, 8).getTime()), now)).toBe('Folder · Yesterday');
    expect(fileFamily({ name: 'x.ts', kind: 'file' })).toBe('code');
    expect(fileFamily({ name: 'x', kind: 'dir' })).toBe('folder');
    expect(tildify('/home/u/x', '/home/u')).toBe('~/x');
  });
});
