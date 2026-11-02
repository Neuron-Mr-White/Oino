import { afterEach, describe, expect, it } from 'vitest';
import { CAPABILITIES, can, capabilities, detectPlatform, isTauri, isWeb, platform, setPlatformOverride, unavailable } from './platform';

const ANDROID_UA = 'Mozilla/5.0 (Linux; Android 14; Pixel 7) AppleWebKit/537.36 Chrome/126 Mobile Safari/537.36';
const LINUX_UA = 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko)';

afterEach(() => {
  setPlatformOverride(undefined);
  delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  delete (window as { __TAURI_OS_PLUGIN_INTERNALS__?: unknown }).__TAURI_OS_PLUGIN_INTERNALS__;
});

describe('detectPlatform', () => {
  it('plain browser → web (any OS)', () => {
    expect(detectPlatform({}, LINUX_UA)).toEqual({ mode: 'web', os: 'linux', tauri: false });
    expect(detectPlatform({}, ANDROID_UA)).toEqual({ mode: 'web', os: 'android', tauri: false });
  });
  it('Tauri + os plugin android → tauri-mobile', () => {
    expect(detectPlatform({ __TAURI_INTERNALS__: {}, __TAURI_OS_PLUGIN_INTERNALS__: { platform: 'android' } }, LINUX_UA)).toEqual({ mode: 'tauri-mobile', os: 'android', tauri: true });
  });
  it('Tauri + ios → tauri-mobile', () => {
    expect(detectPlatform({ __TAURI_INTERNALS__: {}, __TAURI_OS_PLUGIN_INTERNALS__: { platform: 'ios' } }, '').mode).toBe('tauri-mobile');
  });
  it.each(['linux', 'windows', 'macos'])('Tauri + %s → tauri-desktop', (os) => {
    expect(detectPlatform({ __TAURI_INTERNALS__: {}, __TAURI_OS_PLUGIN_INTERNALS__: { platform: os } }, '')).toEqual({ mode: 'tauri-desktop', os, tauri: true });
  });
  it('Tauri without the os plugin falls back to the user agent', () => {
    expect(detectPlatform({ __TAURI_INTERNALS__: {} }, ANDROID_UA).mode).toBe('tauri-mobile');
    expect(detectPlatform({ __TAURI_INTERNALS__: {} }, LINUX_UA).mode).toBe('tauri-desktop');
    expect(detectPlatform({ __TAURI_INTERNALS__: {} }, '').mode).toBe('tauri-desktop');
  });
  it('reads the real window (not cached)', () => {
    expect(isWeb()).toBe(true);
    Object.assign(window, { __TAURI_INTERNALS__: {}, __TAURI_OS_PLUGIN_INTERNALS__: { platform: 'android' } });
    expect(platform().mode).toBe('tauri-mobile');
    expect(isTauri()).toBe(true);
  });
});

describe('capabilities', () => {
  it('Android keeps every capability it had before (no behaviour change)', () => {
    setPlatformOverride('tauri-mobile');
    for (const cap of ['hosts', 'terminal', 'chat', 'files', 'blob', 'board', 'portForward', 'notifications', 'haptics', 'deepLinks', 'updater', 'camera', 'clipboard', 'backButton'] as const) {
      expect(can(cap), cap).toBe(true);
    }
  });
  it('desktop: native core yes, phone-only plugins no', () => {
    setPlatformOverride('tauri-desktop');
    expect(can('terminal')).toBe(true);
    expect(can('chat')).toBe(true);
    expect(can('fileDragDrop')).toBe(true);
    expect(can('camera')).toBe(false);
    expect(can('haptics')).toBe(false);
    expect(can('updater')).toBe(false);
    expect(can('backButton')).toBe(false);
  });
  it('desktop-native caps (UNI-117 part 2): desktop app only', () => {
    for (const cap of ['nativeNotifications', 'tray', 'windowState'] as const) {
      expect(can(cap, { mode: 'tauri-desktop', os: 'linux', tauri: true }), cap).toBe(true);
      expect(can(cap, { mode: 'tauri-mobile', os: 'android', tauri: true }), cap).toBe(false);
      expect(can(cap, { mode: 'web', os: 'linux', tauri: false }), cap).toBe(false);
    }
    setPlatformOverride('tauri-mobile');
    expect(unavailable('tray')).toMatchObject({ kind: 'desktop-only' });
  });
  it('web: only the board (plus browser APIs)', () => {
    setPlatformOverride('web');
    const on = Object.entries(capabilities()).filter(([, v]) => v).map(([k]) => k).sort();
    expect(on).toEqual(['board', 'clipboard', 'fileDragDrop', 'windowTitle']);
  });
  it('every capability has a matrix row', () => {
    for (const cap of CAPABILITIES) {
      for (const mode of ['web', 'tauri-mobile', 'tauri-desktop'] as const) {
        expect(typeof can(cap, { mode, os: 'unknown', tauri: mode !== 'web' })).toBe('boolean');
      }
    }
  });
});

describe('unavailable', () => {
  it('available → undefined', () => {
    setPlatformOverride('web');
    expect(unavailable('board')).toBeUndefined();
  });
  it('web, works in the app → needs-app wording', () => {
    setPlatformOverride('web');
    const info = unavailable('chat')!;
    expect(info.kind).toBe('needs-app');
    expect(info.title).toBe('Not available in the web version');
    expect(info.body).toMatch(/open the Unipi app/);
  });
  it('desktop, phone-only feature → Phone only', () => {
    setPlatformOverride('tauri-desktop');
    expect(unavailable('camera')).toMatchObject({ kind: 'phone-only', title: 'Phone only' });
  });
  it('phone, desktop-only feature → Desktop only', () => {
    setPlatformOverride('tauri-mobile');
    expect(unavailable('fileDragDrop')).toMatchObject({ kind: 'desktop-only', title: 'Desktop only' });
  });
  it('nowhere yet → Not available yet', () => {
    setPlatformOverride('tauri-desktop');
    expect(unavailable('multiWindow')).toMatchObject({ kind: 'not-yet' });
  });
});
