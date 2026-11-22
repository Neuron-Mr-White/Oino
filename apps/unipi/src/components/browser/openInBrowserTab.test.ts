import { describe, expect, it, vi } from 'vitest';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri')), transformCallback: () => 0 }));
import { browsableUrl } from './openInBrowserTab';

describe('Z-106: terminal links → the Browser tab', () => {
  it('only http(s) pages go to the in-app browser', () => {
    expect(browsableUrl('https://example.com/a?b=1')).toBe(true);
    expect(browsableUrl('http://localhost:5173')).toBe(true);
    expect(browsableUrl('file:///etc/hosts')).toBe(false);
    expect(browsableUrl('mailto:me@example.com')).toBe(false);
  });
});

describe('Browser pane tabs remember their page across restarts', () => {
  it('a tab reopens on its last page, else on the link it was opened for', async () => {
    const { rememberTabUrl, tabStartUrl } = await import('./BrowserTab');
    localStorage.clear();
    expect(tabStartUrl('t1', 'https://example.com/')).toBe('https://example.com/');
    rememberTabUrl('t1', 'https://example.com/later');
    expect(tabStartUrl('t1', 'https://example.com/')).toBe('https://example.com/later');
    rememberTabUrl('t1', 'about:blank');
    expect(tabStartUrl('t1', undefined)).toBe('https://example.com/later');
    expect(tabStartUrl('t2', undefined)).toBeUndefined();
  });
});
