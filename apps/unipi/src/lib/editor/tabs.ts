/**
 * Editor tabs model (WP-A21): the files pane's editor opens several
 * files; each tab owns its `EditorView` (created by its host component),
 * so switching tabs preserves everything — document, undo history,
 * selection, LSP plugin state. The store is per `files` pane, plain
 * signals so it's testable without DOM.
 *
 * Tabs live in a `createStore` array: `dirty` updates mutate the node in
 * place, so `For` keeps each row (and its whole editor) mounted.
 */
import { createSignal } from 'solid-js';
import { createStore, produce } from 'solid-js/store';
import type { EditorView } from '@codemirror/view';
import type { EditorTarget } from '../editorTarget';

export type EditorTab = {
  /** Stable id (`hostId:path` — a file is one tab). */
  id: string;
  hostId: string;
  path: string;
  line?: number;
  asText?: boolean;
  title: string;
  dirty: boolean;
};

export function tabId(hostId: string, path: string): string {
  return `${hostId}:${path}`;
}

export type EditorStore = ReturnType<typeof createEditorStore>;

export function createEditorStore() {
  const [tabs, setTabs] = createStore<EditorTab[]>([]);
  const [activeId, setActiveId] = createSignal<string | undefined>(undefined);
  /** Tabs ever activated — their view mounts lazily on first show and
   *  stays alive after (undo history, LSP plugin state). */
  const [seen, setSeen] = createSignal<Set<string>>(new Set());
  const views = new Map<string, EditorView>();
  const viewWaiters = new Map<string, Array<(view: EditorView | null) => void>>();
  const ops = new Map<string, { save: () => Promise<boolean>; reconnectLsp?: () => void }>();

  const tab = (id: string) => tabs.find((t) => t.id === id);

  function open(target: EditorTarget): EditorTab {
    const id = tabId(target.hostId, target.path);
    const existing = tab(id);
    if (existing) {
      setActiveId(id);
      return existing;
    }
    const next: EditorTab = {
      id,
      hostId: target.hostId,
      path: target.path,
      line: target.line,
      asText: target.asText,
      title: target.path.split('/').pop() ?? target.path,
      dirty: false,
    };
    setTabs(tabs.length, next);
    setActiveId(id);
    setSeen((set) => new Set(set).add(id));
    return next;
  }

  function activate(id: string): void {
    if (tab(id)) {
      setActiveId(id);
      setSeen((set) => new Set(set).add(id));
    }
  }

  /** The tab to switch to once `id` closes (its left neighbour, then right). */
  function neighbour(id: string): string | undefined {
    const at = tabs.findIndex((t) => t.id === id);
    if (at < 0) return undefined;
    return tabs[at - 1]?.id ?? tabs[at + 1]?.id;
  }

  function remove(id: string): void {
    const wasActive = activeId() === id;
    const next = neighbour(id);
    ops.delete(id);
    viewWaiters.delete(id);
    setTabs((list) => list.filter((t) => t.id !== id));
    if (wasActive) setActiveId(next);
  }

  function setDirty(id: string, dirty: boolean): void {
    setTabs(
      (t) => t.id === id && t.dirty !== dirty,
      produce((t) => {
        t.dirty = dirty;
      }),
    );
  }

  /** The live `EditorView` a tab host registered (`null` once it closed). */
  function registerView(id: string, view: EditorView | null): void {
    if (view) {
      views.set(id, view);
      for (const waiter of viewWaiters.get(id) ?? []) waiter(view);
      viewWaiters.delete(id);
    } else {
      views.delete(id);
    }
  }

  function viewFor(id: string): EditorView | undefined {
    return views.get(id);
  }

  /** Per-tab actions the host component exposes (the close sheet's Save,
   *  the status chip's take-back). */
  function registerOps(id: string, next: { save: () => Promise<boolean>; reconnectLsp?: () => void }): void {
    ops.set(id, next);
  }

  function opsFor(id: string): { save: () => Promise<boolean>; reconnectLsp?: () => void } | undefined {
    return ops.get(id);
  }

  /** Resolves with the view once the tab's host mounts it (`displayFile`). */
  function viewReady(id: string, timeoutMs = 5000): Promise<EditorView | null> {
    const existing = views.get(id);
    if (existing) return Promise.resolve(existing);
    return new Promise((resolve) => {
      const waiters = viewWaiters.get(id) ?? [];
      const timer = setTimeout(() => {
        const current = viewWaiters.get(id) ?? [];
        viewWaiters.set(id, current.filter((w) => w !== done));
        resolve(null);
      }, timeoutMs);
      const done = (view: EditorView | null) => {
        clearTimeout(timer);
        resolve(view);
      };
      waiters.push(done);
      viewWaiters.set(id, waiters);
    });
  }

  return {
    tabs: () => tabs,
    activeId,
    seen,
    tab,
    open,
    activate,
    remove,
    setDirty,
    neighbour,
    registerView,
    registerOps,
    opsFor,
    viewFor,
    viewReady,
    /** Any dirty tab (the app-close guard). */
    hasDirty: () => tabs.some((t) => t.dirty),
  };
}
