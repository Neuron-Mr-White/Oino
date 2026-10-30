/**
 * The tree's editor area (WP-A21): a tab strip over stacked per-tab
 * CodeMirror views (`components/files/Editor.tsx`) — each tab keeps its
 * own view alive so switching preserves undo history, selection and LSP
 * plugin state; closing a tab only closes its view (the LSP document
 * stays warm on the server). The strip shows dirty + diagnostic badges;
 * the bar carries the pooled `LspStatus` chip and the "Open in external
 * editor" split button (local host only — the path must exist on this
 * machine).
 *
 * The shared editor-target slot (`lib/editorTarget.ts`) still feeds it:
 * anything that sets a target (tree click, search result, preview
 * "open") becomes a tab; the phone `/files/edit` screen keeps its own
 * single-file editor.
 */
import { createEffect, createSignal, For, Show } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { ChevronDown, ExternalLink, FileText } from 'lucide-solid';
import { clearEditorTarget, getEditorTarget } from '../../lib/editorTarget';
import { createEditorStore, type EditorStore, type EditorTab } from '../../lib/editor/tabs';
import type { PooledLsp } from '../../lib/lsp/pool';
import { LOCAL_HOST } from '../../lib/host/client';
import { BoardSheet } from '../board/BoardSheet';
import { EmptyState } from '../EmptyState';
import { Editor } from './Editor';
import { EditorTabs } from './EditorTabs';
import { LspStatus } from './LspStatus';
import './files.css';

/** `file://` editor URL schemes the split button offers (Z-102). */
const EXTERNAL_EDITORS: { id: string; name: string; url: (path: string) => string }[] = [
  { id: 'default', name: 'System default', url: (p) => `file://${p}` },
  { id: 'vscode', name: 'VS Code', url: (p) => `vscode://file${p}` },
  { id: 'zed', name: 'Zed', url: (p) => `zed://file${p}` },
  { id: 'sublime', name: 'Sublime Text', url: (p) => `subl://open?url=file://${p}` },
];

export function EditorArea(props: { root?: () => string | undefined; onBinary: (path: string) => void }) {
  const store = createEditorStore();
  const [lspMap, setLspMap] = createSignal<Record<string, PooledLsp | null>>({});
  const [closeAsk, setCloseAsk] = createSignal<EditorTab | undefined>();
  const [editorMenu, setEditorMenu] = createSignal(false);

  // Anything that sets the shared target becomes a tab (tree, search,
  // preview, jump-to-definition through the workspace hook).
  createEffect(() => {
    const target = getEditorTarget();
    if (!target) return;
    clearEditorTarget();
    store.open(target);
  });

  const active = () => {
    const id = store.activeId();
    return id ? store.tab(id) : undefined;
  };
  const activeLsp = () => {
    const id = store.activeId();
    return id ? lspMap()[id] : undefined;
  };

  function onLsp(tabId: string, pooled: PooledLsp | null) {
    setLspMap((map) => ({ ...map, [tabId]: pooled }));
  }

  function requestClose(tab: EditorTab) {
    if (tab.dirty) {
      setCloseAsk(tab);
      return;
    }
    store.remove(tab.id);
  }

  async function closeSaving(tab: EditorTab) {
    const save = store.opsFor(tab.id)?.save;
    const ok = save ? await save() : true;
    if (ok) {
      setCloseAsk(undefined);
      store.remove(tab.id);
    }
  }

  function openExternal(url: (path: string) => string) {
    const tab = active();
    if (!tab || tab.hostId !== LOCAL_HOST) return;
    void invoke('open_url', { url: url(tab.path) }).catch(() => {});
    setEditorMenu(false);
  }

  return (
    <div class="feditor-area" data-testid="files-editor-area">
      <Show when={store.tabs().length > 0} fallback={<EmptyState compact icon={<FileText size={22} />} title="No file open" body="Pick a file in the tree." testId="files-editor-empty" />}>
        <div class="feditor-topbar">
          <EditorTabs store={store as EditorStore} onClose={requestClose} />
          <div class="feditor-tools">
            <LspStatus
              pooled={activeLsp}
              onTakeBack={() => {
                const tab = active();
                if (!tab) return;
                // Re-acquire: the host's single-attach makes this stream
                // the owner again (the other device gets E_LSP_TAKEN).
                store.opsFor(tab.id)?.reconnectLsp?.();
              }}
            />
            <Show when={active() && active()!.hostId === LOCAL_HOST}>
              <div class="feditor-external">
                <button
                  type="button"
                  class="ftree-tool"
                  aria-label="Open in external editor"
                  data-tip="Open in external editor"
                  data-testid="files-editor-external"
                  onClick={() => openExternal(EXTERNAL_EDITORS[0]!.url)}
                >
                  <ExternalLink size={14} />
                </button>
                <button
                  type="button"
                  class="ftree-tool"
                  aria-label="Choose external editor"
                  aria-expanded={editorMenu()}
                  onClick={(e) => {
                    e.stopPropagation();
                    setEditorMenu((v) => !v);
                  }}
                >
                  <ChevronDown size={12} />
                </button>
                <Show when={editorMenu()}>
                  <div class="feditor-external-menu" role="menu">
                    <For each={EXTERNAL_EDITORS}>
                      {(editor) => (
                        <button type="button" role="menuitem" class="feditor-external-item" onClick={() => openExternal(editor.url)}>
                          {editor.name}
                        </button>
                      )}
                    </For>
                  </div>
                </Show>
              </div>
            </Show>
          </div>
        </div>
        <div class="feditor-stack">
          <For each={store.tabs()}>
            {(tab) => (
              <Show when={store.seen().has(tab.id)}>
                <div class="feditor-pane" classList={{ hidden: store.activeId() !== tab.id }}>
                  <Editor
                    tab={tab}
                    store={store}
                    root={props.root}
                    visible={() => store.activeId() === tab.id}
                    onBinary={props.onBinary}
                    onLsp={onLsp}
                    onRequestClose={requestClose}
                  />
                </div>
              </Show>
            )}
          </For>
        </div>
      </Show>

      <BoardSheet open={closeAsk() !== undefined} onClose={() => setCloseAsk(undefined)} label="Unsaved changes" testId="files-editor-leave">
        <p class="p-2 text-text-muted">Save your changes to {closeAsk()?.title} before closing?</p>
        <div class="files-sheet-actions">
          <button type="button" class="files-sheet-btn" onClick={() => setCloseAsk(undefined)}>Keep editing</button>
          <button
            type="button"
            class="files-sheet-btn danger"
            onClick={() => {
              const tab = closeAsk();
              setCloseAsk(undefined);
              if (tab) store.remove(tab.id);
            }}
          >
            Discard
          </button>
          <button type="button" class="files-sheet-btn primary" onClick={() => void closeSaving(closeAsk()!)}>Save</button>
        </div>
      </BoardSheet>
    </div>
  );
}
