import { afterEach, describe, expect, it, vi } from 'vitest';
import { createRoot } from 'solid-js';
import { createDirWatch } from './watch';

function flushFocus() {
  window.dispatchEvent(new Event('focus'));
}

describe('createDirWatch (refresh on focus — the default while H7 has no fs events)', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    document.body.innerHTML = '';
  });

  it('is clean until the window gains focus', () => {
    createRoot((dispose) => {
      const watch = createDirWatch();
      expect(watch.dirty()).toBe(false);
      flushFocus();
      expect(watch.dirty()).toBe(true);
      watch.clear();
      expect(watch.dirty()).toBe(false);
      dispose();
    });
  });

  it('refreshes when the document becomes visible', () => {
    createRoot((dispose) => {
      const watch = createDirWatch();
      Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'visible' });
      document.dispatchEvent(new Event('visibilitychange'));
      expect(watch.dirty()).toBe(true);
      dispose();
    });
  });

  it('refreshes when its pane tab body is un-hidden (tab activation)', async () => {
    await createRoot(async (dispose) => {
      const body = document.createElement('div');
      body.setAttribute('hidden', '');
      document.body.append(body);
      const watch = createDirWatch({ body: () => body });
      expect(watch.dirty()).toBe(false);
      body.removeAttribute('hidden');
      await Promise.resolve(); // MutationObserver delivers on a microtask
      expect(watch.dirty()).toBe(true);
      // Attr flips while already dirty stay dirty; clear resets.
      watch.clear();
      body.setAttribute('hidden', '');
      await Promise.resolve();
      expect(watch.dirty()).toBe(true);
      dispose();
    });
  });

  it('mutated() bumps for the tree own writes (rename/delete/upload)', () => {
    createRoot((dispose) => {
      const watch = createDirWatch();
      watch.mutated();
      expect(watch.dirty()).toBe(true);
      dispose();
    });
  });
});
