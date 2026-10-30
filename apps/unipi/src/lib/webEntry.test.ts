import { afterEach, describe, expect, it } from 'vitest';
import { applyWebEntry, parseWebEntry } from './webEntry';
import { readLast } from './board/ui';

const loc = (pathname: string, search = '', hash = '') => ({ pathname, search, hash });

describe('web entry (daemon links → #/board)', () => {
  afterEach(() => localStorage.clear());
  it('leaves plain app URLs alone', () => {
    expect(parseWebEntry(loc('/', '', '#/board'))).toBeNull();
    expect(parseWebEntry(loc('/index.html'))).toBeNull();
  });
  it('reads /p/<slug> (what `/unipi:kanboard open` prints)', () => {
    expect(parseWebEntry(loc('/p/my-proj-1a2b3c'))).toEqual({ slug: 'my-proj-1a2b3c', view: undefined, clean: '/#/board' });
    expect(parseWebEntry(loc('/p/my%20proj/', '', '#/board'))).toEqual({ slug: 'my proj', view: undefined, clean: '/#/board' });
  });
  it('reads the old UI query (?project=&view=list), keeps unrelated params, drops t', () => {
    expect(parseWebEntry(loc('/', '?project=x&view=list&t=secret&debug=1'))).toEqual({ slug: 'x', view: 'list', clean: '/?debug=1#/board' });
  });
  it('sends unknown paths to the board', () => {
    expect(parseWebEntry(loc('/whatever'))).toEqual({ slug: undefined, view: undefined, clean: '/#/board' });
  });
  it('applies: remembers the project for the web PC, the view, and rewrites the address bar', () => {
    window.history.replaceState(null, '', '/p/slug-1?view=list');
    const entry = applyWebEntry(window);
    expect(entry?.slug).toBe('slug-1');
    expect(readLast()).toEqual({ hostId: 'dev', projects: { dev: 'slug-1' } });
    expect(localStorage.getItem('unipi.board.view')).toBe('list');
    expect(window.location.pathname).toBe('/');
    expect(window.location.hash).toBe('#/board');
  });
});
