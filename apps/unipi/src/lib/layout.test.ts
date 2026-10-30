import { afterEach, describe, expect, it } from 'vitest';
import {
  autoCollapse,
  chatWidth,
  clampBottom,
  clampSide,
  clampSidebar,
  DEFAULT_PREFS,
  layout,
  readPrefs,
  resetLayout,
  setDesktopOverride,
  sideMode,
  wantsDesktopShell,
  type ShellPrefs,
} from './layout';

const prefs = (p: Partial<ShellPrefs> = {}): ShellPrefs => ({ ...DEFAULT_PREFS, sidebarOpen: true, sideOpen: true, ...p });
const none = { sidebar: false, side: false };

function setWidth(w: number) {
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: w });
}

afterEach(() => {
  setDesktopOverride(undefined);
  localStorage.clear();
  setWidth(1024);
});

describe('desktop switch', () => {
  it('desktop shell only at ≥ 900px and never in the phone app', () => {
    expect(wantsDesktopShell(1440, 'web', true)).toBe(true);
    expect(wantsDesktopShell(1440, 'tauri-desktop', true)).toBe(true);
    expect(wantsDesktopShell(899, 'tauri-desktop', true)).toBe(false);
    expect(wantsDesktopShell(915, 'tauri-mobile', true)).toBe(false);
    expect(wantsDesktopShell(1440, 'web', false)).toBe(false);
  });
});

describe('push vs overlay (Z-95)', () => {
  it('the side pane pushes only at ≥ 1280px', () => {
    expect(sideMode(1440)).toBe('push');
    expect(sideMode(1280)).toBe('push');
    expect(sideMode(1279)).toBe('overlay');
    expect(sideMode(1000)).toBe('overlay');
  });
  it('an overlay does not take chat width', () => {
    expect(chatWidth(1440, 264, 480)).toBe(696);
    expect(chatWidth(1000, 264, 480)).toBe(736);
  });
});

describe('auto-collapse on resize (Z-3)', () => {
  it('collapses the side pane when the chat would be < 480px', () => {
    // 1300 - 264 - 600 = 436 < 480
    expect(autoCollapse(prefs({ sideW: 600 }), none, 1300)).toEqual({ sidebar: false, side: true });
    // 1440 - 264 - 480 = 696: fine
    expect(autoCollapse(prefs(), none, 1440)).toEqual(none);
  });
  it('collapses the side pane when the window drops into overlay mode, restores it when push returns', () => {
    const narrow = autoCollapse(prefs(), none, 1100);
    expect(narrow).toEqual({ sidebar: false, side: true });
    expect(autoCollapse(prefs(), narrow, 1440)).toEqual(none);
  });
  it('collapses the sidebar when the chat would be < 360px, restores when space returns', () => {
    const p = prefs({ sideOpen: false, sidebarW: 400 });
    // 700 - 400 = 300 < 360
    const tiny = autoCollapse(p, none, 700);
    expect(tiny).toEqual({ sidebar: true, side: false });
    // 759 - 400 = 359: still collapsed
    expect(autoCollapse(p, tiny, 759)).toEqual(tiny);
    expect(autoCollapse(p, tiny, 760)).toEqual(none);
  });
  it('side pane goes first; the sidebar only if that was not enough', () => {
    // 1290: push; 1290 - 420 - 700 < 480 → side collapses; 1290 - 420 = 870 ≥ 360 → sidebar stays
    expect(autoCollapse(prefs({ sidebarW: 420, sideW: 700 }), none, 1290)).toEqual({ sidebar: false, side: true });
  });
  it('never restores what the user closed', () => {
    const closedByUser = prefs({ sideOpen: false, sidebarOpen: false });
    expect(autoCollapse(closedByUser, { sidebar: true, side: true }, 1600)).toEqual(none);
  });
  it('store: a user toggle clears the auto flag; resize re-evaluates', () => {
    setDesktopOverride(true);
    setWidth(1440);
    localStorage.setItem('unipi.shell.layout', JSON.stringify(prefs()));
    resetLayout();
    const L = layout();
    expect(L.sideOpen()).toBe(true);
    expect(L.sideMode()).toBe('push');
    setWidth(1000);
    window.dispatchEvent(new Event('resize'));
    expect(L.sideOpen()).toBe(false); // auto-collapsed
    expect(L.prefs().sideOpen).toBe(true); // the user's choice is kept
    L.toggleSide(true); // the user opens it as an overlay
    expect(L.sideOpen()).toBe(true);
    expect(L.sideMode()).toBe('overlay');
    setWidth(1440);
    window.dispatchEvent(new Event('resize'));
    expect(L.sideOpen()).toBe(true);
    L.toggleSidebar();
    expect(L.sidebarOpen()).toBe(false);
    expect(JSON.parse(localStorage.getItem('unipi.shell.layout')!).sidebarOpen).toBe(false);
  });
});

describe('sizes', () => {
  it('sidebar 220–420 and leaves 360px', () => {
    expect(clampSidebar(100, 1440)).toBe(220);
    expect(clampSidebar(900, 1440)).toBe(420);
    expect(clampSidebar(300, 1440)).toBe(300);
    expect(clampSidebar(400, 700)).toBe(340);
  });
  it('side pane ≥ 320 and, in push mode, leaves the chat 480px', () => {
    expect(clampSide(100, 1440, 264)).toBe(320);
    expect(clampSide(1000, 1440, 264)).toBe(1440 - 264 - 480);
    expect(clampSide(700, 1000, 264)).toBe(688); // overlay: up to the frame minus a sliver
  });
  it('bottom panel ≥ 120 and leaves the conversation 200px', () => {
    expect(clampBottom(50, 800)).toBe(120);
    expect(clampBottom(900, 800)).toBe(600);
  });
  it('prefs parse tolerates junk', () => {
    expect(readPrefs('nope')).toEqual(DEFAULT_PREFS);
    expect(readPrefs('{"sideW":"wide","sidebarOpen":false}')).toEqual({ ...DEFAULT_PREFS, sidebarOpen: false });
  });
});

describe('full view (Z-6)', () => {
  it('opens the side pane; hiding the pane leaves full view; exiting keeps the rest', () => {
    setDesktopOverride(true);
    setWidth(1440);
    const L = layout();
    L.toggleBottom(true);
    L.toggleFullView();
    expect(L.fullView()).toBe(true);
    expect(L.sideOpen()).toBe(true);
    L.toggleFullView();
    expect(L.fullView()).toBe(false);
    expect(L.sideOpen()).toBe(true);
    expect(L.bottomOpen()).toBe(true);
    L.toggleFullView(true);
    L.toggleSide(false);
    expect(L.fullView()).toBe(false);
    L.toggleSide(true);
    expect(L.fullView()).toBe(false); // reopening the pane does not return to full view
  });
});
