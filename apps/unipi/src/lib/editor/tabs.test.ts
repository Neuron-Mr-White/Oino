import { describe, expect, it } from 'vitest';
import { createEditorStore, tabId } from './tabs';

const t = (hostId: string, path: string, line?: number) => ({ hostId, path, line });

describe('editor tabs store', () => {
  it('opens a file once and activates it', () => {
    const store = createEditorStore();
    store.open(t('h1', '/p/a.ts'));
    store.open(t('h1', '/p/b.ts'));
    expect(store.tabs().map((x) => x.title)).toEqual(['a.ts', 'b.ts']);
    expect(store.activeId()).toBe(tabId('h1', '/p/b.ts'));
    // Re-open dedupes + activates.
    store.open(t('h1', '/p/a.ts'));
    expect(store.tabs()).toHaveLength(2);
    expect(store.activeId()).toBe(tabId('h1', '/p/a.ts'));
  });

  it('tracks dirty per tab independently', () => {
    const store = createEditorStore();
    const a = store.open(t('h1', '/p/a.ts'));
    const b = store.open(t('h1', '/p/b.ts'));
    store.setDirty(a.id, true);
    expect(store.tab(a.id)!.dirty).toBe(true);
    expect(store.tab(b.id)!.dirty).toBe(false);
    expect(store.hasDirty()).toBe(true);
    store.setDirty(a.id, false);
    expect(store.hasDirty()).toBe(false);
  });

  it('closing the active tab activates its left neighbour', () => {
    const store = createEditorStore();
    const a = store.open(t('h1', '/p/a.ts'));
    const b = store.open(t('h1', '/p/b.ts'));
    const c = store.open(t('h1', '/p/c.ts'));
    store.activate(b.id);
    store.remove(b.id);
    expect(store.activeId()).toBe(a.id);
    // Closing a non-active tab keeps the active one.
    store.activate(c.id);
    store.remove(a.id);
    expect(store.activeId()).toBe(c.id);
    store.remove(c.id);
    expect(store.activeId()).toBeUndefined();
  });

  it('the same file on different hosts is two tabs', () => {
    const store = createEditorStore();
    store.open(t('h1', '/p/a.ts'));
    store.open(t('h2', '/p/a.ts'));
    expect(store.tabs()).toHaveLength(2);
  });

  it('viewReady resolves when the tab host registers its view', async () => {
    const store = createEditorStore();
    const tab = store.open(t('h1', '/p/a.ts'));
    const pending = store.viewReady(tab.id);
    store.registerView(tab.id, {} as never);
    expect(await pending).not.toBeNull();
    store.registerView(tab.id, null);
    expect(store.viewFor(tab.id)).toBeUndefined();
  });
});
