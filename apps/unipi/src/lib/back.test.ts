// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { backDepth, pushBack } from './back';
import { setPlatformOverride } from './platform';

/** jsdom fires popstate asynchronously after history.back(). */
const settle = () => new Promise((resolve) => setTimeout(resolve, 30));

describe('pushBack in the web build: the browser Back button closes the top overlay', () => {
  beforeEach(() => setPlatformOverride('web'));
  afterEach(async () => {
    setPlatformOverride(undefined);
    await settle();
  });

  it('adds a history entry; browser Back runs the top handler and stays in the app', async () => {
    const start = history.length;
    const close = vi.fn();
    const pop = pushBack(() => {
      close();
      pop();
    });
    expect(history.length).toBe(start + 1);
    history.back();
    await settle();
    expect(close).toHaveBeenCalledTimes(1);
    expect(backDepth()).toBe(0);
  });

  it('closing in the app (✕ / Esc) takes its entry back off, without re-running a handler', async () => {
    const outer = vi.fn();
    const popOuter = pushBack(outer);
    const inner = vi.fn();
    const popInner = pushBack(inner);
    const marked = history.state;
    popInner();
    await settle();
    expect(history.state).not.toEqual(marked);
    expect(inner).not.toHaveBeenCalled();
    expect(outer).not.toHaveBeenCalled();
    // The outer overlay still owns the current entry: Back closes it.
    history.back();
    await settle();
    expect(outer).toHaveBeenCalledTimes(1);
    popOuter();
  });

  it('a handler that keeps its overlay (discard guard) gets a fresh entry for the next Back', async () => {
    let asks = 0;
    const pop = pushBack(() => {
      asks += 1; // shows "Discard?" and stays open
    });
    history.back();
    await settle();
    expect(asks).toBe(1);
    history.back();
    await settle();
    expect(asks).toBe(2);
    pop();
    await settle();
  });

  it('a menu closing straight into a sheet hands its entry over (no back/push race)', async () => {
    const popMenu = pushBack(() => {});
    const withMenu = history.length;
    popMenu();
    const sheet = vi.fn();
    const popSheet = pushBack(sheet);
    await settle();
    expect(history.length).toBe(withMenu);
    expect(sheet).not.toHaveBeenCalled();
    expect(backDepth()).toBe(1);
    history.back();
    await settle();
    expect(sheet).toHaveBeenCalledTimes(1);
    popSheet();
  });

  it('the app (tauri) does not touch browser history', () => {
    setPlatformOverride('tauri-mobile');
    const start = history.length;
    const pop = pushBack(() => {});
    expect(history.length).toBe(start);
    pop();
  });
});
