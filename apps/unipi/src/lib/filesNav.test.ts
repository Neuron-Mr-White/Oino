import { afterEach, describe, expect, it } from 'vitest';
import { activeRoot, crumbsFor, lastFolder, nextFilesBack, placesLayout, rememberFolder, rootLabel, startFolder } from './filesNav';

describe('filesNav last folder', () => {
  afterEach(() => {
    localStorage.clear();
  });

  it('remembers and restores the folder per host', () => {
    rememberFolder('h1', '/home/u/proj');
    rememberFolder('h2', '/tmp');
    expect(lastFolder('h1')).toBe('/home/u/proj');
    expect(lastFolder('h2')).toBe('/tmp');
    rememberFolder('h1', '/home/u/proj/src');
    expect(lastFolder('h1')).toBe('/home/u/proj/src');
    expect(lastFolder('h2')).toBe('/tmp');
  });

  it('returns undefined for an unknown host', () => {
    expect(lastFolder('nobody')).toBeUndefined();
  });

  it('tolerates corrupt JSON', () => {
    localStorage.setItem('unipi.files.lastFolder', '{not json');
    expect(lastFolder('h1')).toBeUndefined();
    rememberFolder('h1', '/ok');
    expect(lastFolder('h1')).toBe('/ok');
  });
});

describe('nextFilesBack', () => {
  const roots = ['/home/u', '/tmp'];

  it('goes up one folder inside a host tree', () => {
    expect(nextFilesBack({ path: '/home/u/proj/src', roots, hasReturnChat: false })).toEqual({
      action: 'parent',
      to: '/home/u/proj',
    });
  });

  it('returns to the chat from a root when one is armed', () => {
    expect(nextFilesBack({ path: '/home/u', roots, hasReturnChat: true })).toEqual({ action: 'chat' });
    expect(nextFilesBack({ path: '/', roots, hasReturnChat: true })).toEqual({ action: 'chat' });
  });

  it('goes home from a root with no chat', () => {
    expect(nextFilesBack({ path: '/home/u', roots, hasReturnChat: false })).toEqual({ action: 'home' });
    expect(nextFilesBack({ path: '/', roots, hasReturnChat: false })).toEqual({ action: 'home' });
  });

  it('goes home from the roots list', () => {
    expect(nextFilesBack({ path: undefined, roots, hasReturnChat: false })).toEqual({ action: 'home' });
    expect(nextFilesBack({ path: undefined, roots, hasReturnChat: true })).toEqual({ action: 'chat' });
  });
});

describe('root picker (UNI-220)', () => {
  const roots = ['/home/u', '/tmp', '/var/tmp', '/mnt', '/home/u/proj', '/srv/site'];

  it('labels home ~, folders under home by name, extra roots by full path', () => {
    expect(roots.map((r) => rootLabel(r, '/home/u'))).toEqual(['~', '/tmp', '/var/tmp', '/mnt', 'proj', '/srv/site']);
    expect(rootLabel('/home/user2', '/home/u')).toBe('/home/user2');
    expect(rootLabel('/tmp', undefined)).toBe('/tmp');
  });

  it('activeRoot is the deepest root containing the path', () => {
    expect(activeRoot('/home/u/proj/src', roots)).toBe('/home/u/proj');
    expect(activeRoot('/home/u/docs', roots)).toBe('/home/u');
    expect(activeRoot('/tmp', roots)).toBe('/tmp');
    expect(activeRoot('/tmp/x/y', roots)).toBe('/tmp');
    expect(activeRoot('/tmpfoo', roots)).toBeUndefined();
    expect(activeRoot('/var/tmp/a', roots)).toBe('/var/tmp');
    expect(activeRoot('/etc', roots)).toBeUndefined();
    expect(activeRoot(undefined, roots)).toBeUndefined();
  });

  it('Back stops at an extra root instead of walking to /', () => {
    expect(nextFilesBack({ path: '/tmp', roots, hasReturnChat: false })).toEqual({ action: 'home' });
    expect(nextFilesBack({ path: '/tmp/work', roots, hasReturnChat: false })).toEqual({ action: 'parent', to: '/tmp' });
  });
});

describe('breadcrumb + Places layout (UNI-218 F38)', () => {
  const roots = ['/home/u', '/tmp', '/home/u/proj'];
  it('starts the breadcrumb at the root being browsed', () => {
    expect(crumbsFor('/home/u/docs/a', roots)).toEqual([
      { label: '~', path: '/home/u', root: true },
      { label: 'docs', path: '/home/u/docs' },
      { label: 'a', path: '/home/u/docs/a' },
    ]);
    expect(crumbsFor('/home/u/proj/src', roots).map((c) => c.label)).toEqual(['proj', 'src']);
    expect(crumbsFor('/tmp', roots)).toEqual([{ label: '/tmp', path: '/tmp', root: true }]);
    expect(crumbsFor('/tmp/qa2-files/empty-dir', roots).map((c) => c.path)).toEqual(['/tmp', '/tmp/qa2-files', '/tmp/qa2-files/empty-dir']);
  });
  it('falls back to / for a path outside every root, and nothing without a path', () => {
    expect(crumbsFor('/etc/x', roots).map((c) => c.label)).toEqual(['/', 'etc', 'x']);
    expect(crumbsFor('/x', ['/'])).toEqual([{ label: '/', path: '/', root: true }, { label: 'x', path: '/x' }]);
    expect(crumbsFor(undefined, roots)).toEqual([]);
  });
  it('a few short roots inline, many or long ones in a menu', () => {
    expect(placesLayout(['/home/u'])).toBe('none');
    expect(placesLayout(['/home/u', '/tmp'])).toBe('inline');
    expect(placesLayout(['/home/u', '/tmp', '/mnt'])).toBe('inline');
    expect(placesLayout(['/home/u', '/tmp', '/mnt', '/srv'])).toBe('menu');
    expect(placesLayout(['/home/u', '/opt/a/very/deep/path/to/a/project'])).toBe('menu');
    expect(placesLayout(['/home/u', '/tmp', '/mnt'], 2)).toBe('menu');
  });
});

describe('filesNav start folder (F63)', () => {
  it('reopens the remembered folder while it is under a root', () => {
    localStorage.clear();
    rememberFolder('h1', '/home/u/proj/src');
    expect(startFolder('h1', ['/home/u', '/tmp'])).toBe('/home/u/proj/src');
  });
  it('falls back to the first root when the remembered folder is out of scope or missing', () => {
    localStorage.clear();
    rememberFolder('h1', '/home/old');
    expect(startFolder('h1', ['/home/u', '/tmp'])).toBe('/home/u');
    expect(startFolder('h2', ['/tmp'])).toBe('/tmp');
    expect(startFolder('h3', [])).toBeUndefined();
  });
});
