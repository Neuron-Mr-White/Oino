import { afterEach, describe, expect, it, vi } from 'vitest';
import { openExternal } from './share';
import { setPlatformOverride } from '../platform';

const invoke = vi.fn(async () => null);
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...(a as [])) }));

afterEach(() => {
  setPlatformOverride(undefined);
  delete (window as { UniPiFiles?: unknown }).UniPiFiles;
  vi.restoreAllMocks();
  invoke.mockClear();
});

/** UNI-224: in the app, window.open loaded the link over the app's own WebView (no way back). */
describe('openExternal', () => {
  it('Android: the bridge opens the system browser, never window.open', () => {
    setPlatformOverride('tauri-mobile');
    const open = vi.spyOn(window, 'open');
    const openUrl = vi.fn(() => '{"ok":true}');
    Object.assign(window, { UniPiFiles: { openUrl } });
    expect(openExternal('https://example.com/x')).toBe(true);
    expect(openUrl).toHaveBeenCalledWith('https://example.com/x');
    expect(open).not.toHaveBeenCalled();
  });

  it('Android without the bridge method (old APK): false (callers copy), never window.open', () => {
    setPlatformOverride('tauri-mobile');
    const open = vi.spyOn(window, 'open');
    Object.assign(window, { UniPiFiles: {} });
    expect(openExternal('https://example.com/x')).toBe(false);
    expect(open).not.toHaveBeenCalled();
  });

  it('desktop: the core opens the default browser', async () => {
    setPlatformOverride('tauri-desktop');
    const open = vi.spyOn(window, 'open');
    expect(openExternal('http://127.0.0.1:5173/')).toBe(true);
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('open_url', { url: 'http://127.0.0.1:5173/' }));
    expect(open).not.toHaveBeenCalled();
  });

  it('web: a new tab', () => {
    setPlatformOverride('web');
    const open = vi.spyOn(window, 'open').mockReturnValue({} as Window);
    expect(openExternal('https://example.com')).toBe(true);
    expect(open).toHaveBeenCalledWith('https://example.com', '_blank', 'noopener,noreferrer');
  });

  it('refuses non-web links', () => {
    setPlatformOverride('web');
    expect(openExternal('javascript:alert(1)')).toBe(false);
    expect(openExternal('file:///etc/passwd')).toBe(false);
  });
});
