import { describe, expect, it } from 'vitest';
import { isTabRoot, isWorkspace, parentFolder, phoneParent, tabOf } from './phoneNav';

describe('phoneParent (phone spec §1.2 / §1.3)', () => {
  it.each([
    ['/chat', '', undefined],
    ['/chat/archived', '', '/chat'],
    ['/s/h1/abc', '', '/chat'],
    ['/new/h1', '?cwd=/x', '/chat'],
    ['/board', '', '/chat'],
    ['/board/task/7', '', '/board'],
    ['/cron', '', '/chat'],
    ['/cron/job/j1', '', '/cron'],
    ['/files', '', '/chat'],
    ['/settings', '', '/chat'],
    ['/settings/providers', '', '/settings'],
    ['/settings/agents/coder', '', '/settings/agents'],
    ['/settings/ports', '', '/settings'],
    ['/x/ext.page', '', '/settings'],
    ['/preview', '', '/settings'],
    ['/files/edit', '', '/files'],
  ])('%s%s → %s', (path, search, parent) => {
    expect(phoneParent(path, search)).toBe(parent);
  });

  it('files folders walk up to the Places root, then Chats', () => {
    expect(phoneParent('/files', '?path=%2Fhome%2Fme%2Fapp')).toBe('/files?path=%2Fhome%2Fme');
    expect(phoneParent('/files', '?path=/')).toBe('/files');
    expect(phoneParent('/files', '?path=~')).toBe('/files');
    expect(phoneParent('/files', '?path=~/a&root=~/a')).toBe('/files');
    expect(phoneParent('/files/edit', '?path=/a/b.txt')).toBe('/files?path=%2Fa');
    expect(phoneParent('/files/edit', '?from=/srv')).toBe('/files?path=%2Fsrv');
  });

  it('parentFolder', () => {
    expect(parentFolder('/a/b/')).toBe('/a');
    expect(parentFolder('/a')).toBe('/');
    expect(parentFolder('/')).toBeUndefined();
    expect(parentFolder('~/x')).toBe('~');
  });

  it('tabs, roots, workspaces', () => {
    expect(tabOf('/s/h/1')).toBe('chat');
    expect(tabOf('/superagent')).toBe('superagent');
    expect(tabOf('/settings/ports')).toBe('settings');
    expect(tabOf('/x/a')).toBe('settings');
    expect(isTabRoot('/cron')).toBe(true);
    expect(isTabRoot('/cron/job/1')).toBe(false);
    expect(isWorkspace('/files/edit')).toBe(true);
    expect(isWorkspace('/files')).toBe(false);
    expect(isWorkspace('/board/task/1')).toBe(true);
  });
});
