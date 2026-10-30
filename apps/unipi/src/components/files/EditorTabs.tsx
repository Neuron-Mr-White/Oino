/**
 * The editor tab strip (WP-A21): one chip per open file — title, dirty
 * dot, diagnostic badge (worst severity from `lib/lsp/diagnostics`),
 * close ×. Clicking activates; the strip scrolls horizontally when it
 * overflows. Rendered above the stacked tab views (`EditorArea`).
 */
import { For, Show } from 'solid-js';
import { X } from 'lucide-solid';
import { diagFor } from '../../lib/lsp/diagnostics';
import type { EditorStore, EditorTab } from '../../lib/editor/tabs';

export function EditorTabs(props: { store: EditorStore; onClose: (tab: EditorTab) => void }) {
  const store = () => props.store;
  return (
    <div class="feditor-tabs" role="tablist" aria-label="Open files" data-testid="files-editor-tabs">
      <For each={store().tabs()}>
        {(tab) => {
          const active = () => store().activeId() === tab.id;
          const diag = () => diagFor(tab.path);
          return (
            <div
              class="feditor-tab"
              classList={{ active: active(), dirty: tab.dirty }}
              role="tab"
              aria-selected={active()}
              title={tab.path}
              data-testid={`editor-tab-${tab.title}`}
              onClick={() => store().activate(tab.id)}
              onAuxClick={(e) => {
                if (e.button === 1) props.onClose(tab);
              }}
            >
              <span class="feditor-tab-name">{tab.title}</span>
              <Show when={diag()?.errors}>
                <span class="feditor-tab-diag err" aria-label={`${diag()!.errors} errors`}>
                  {diag()!.errors}
                </span>
              </Show>
              <Show when={!diag()?.errors && diag()?.warnings}>
                <span class="feditor-tab-diag warn" aria-label={`${diag()!.warnings} warnings`}>
                  {diag()!.warnings}
                </span>
              </Show>
              <Show when={tab.dirty}>
                <span class="editor-dirty-dot" aria-label="Unsaved changes" />
              </Show>
              <button
                type="button"
                class="feditor-tab-close"
                aria-label={`Close ${tab.title}`}
                data-testid={`editor-tab-close-${tab.title}`}
                onClick={(e) => {
                  e.stopPropagation();
                  props.onClose(tab);
                }}
              >
                <X size={12} strokeWidth={2.4} />
              </button>
            </div>
          );
        }}
      </For>
    </div>
  );
}
