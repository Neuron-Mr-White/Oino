/**
 * "Open ▾" for a file on the user's own PC: Show in file manager, Open with
 * default app, and the editors found on the PC (owner5 #23). `variant`
 * `icon` is the file header's tool button, `button` the empty state's.
 */
import { createResource, createSignal, For, Show } from 'solid-js';
import { ChevronDown, ExternalLink, FolderSearch, AppWindow, Code2 } from 'lucide-solid';
import { AnchoredMenu } from '../board/AnchoredMenu';
import { localEditors, localOpen } from '../../lib/openLocal';

export function OpenMenu(props: { path: string; variant: 'icon' | 'button'; onResult?: (message: string) => void }) {
  const [anchor, setAnchor] = createSignal<DOMRect | undefined>();
  const [editors] = createResource(localEditors);
  const run = async (how: string) => {
    setAnchor(undefined);
    const res = await localOpen(props.path, how);
    if (!res.ok) props.onResult?.(res.message);
  };
  return (
    <>
      <Show
        when={props.variant === 'icon'}
        fallback={
          <button type="button" class="btn btn-secondary" aria-haspopup="menu" aria-expanded={!!anchor()} data-testid="open-menu" onClick={(e) => setAnchor((e.currentTarget as HTMLElement).getBoundingClientRect())}>
            Open <ChevronDown size={14} />
          </button>
        }
      >
        <button type="button" class="ftree-tool open-menu-icon" aria-label="Open" aria-haspopup="menu" aria-expanded={!!anchor()} data-tip="Open" data-testid="open-menu" onClick={(e) => setAnchor((e.currentTarget as HTMLElement).getBoundingClientRect())}>
          <ExternalLink size={15} />
          <ChevronDown size={11} />
        </button>
      </Show>
      <Show when={anchor()}>
        {(rect) => (
          <AnchoredMenu anchor={rect()} width={236} label="Open" alignEnd onDismiss={() => setAnchor(undefined)}>
            <div class="ws-menu" data-testid="open-menu-list">
              <button type="button" role="menuitem" class="ws-row" onClick={() => void run('reveal')}>
                <FolderSearch size={15} class="ws-row-ic" />
                <span class="ws-row-text"><span class="ws-row-label">Show in file manager</span></span>
              </button>
              <button type="button" role="menuitem" class="ws-row" onClick={() => void run('default')}>
                <AppWindow size={15} class="ws-row-ic" />
                <span class="ws-row-text"><span class="ws-row-label">Open with default app</span></span>
              </button>
              <Show when={(editors() ?? []).length > 0}>
                <div class="ws-sep" role="separator" />
                <For each={editors()}>
                  {(editor) => (
                    <button type="button" role="menuitem" class="ws-row" onClick={() => void run(editor.id)}>
                      <Code2 size={15} class="ws-row-ic" />
                      <span class="ws-row-text"><span class="ws-row-label">{editor.name}</span></span>
                    </button>
                  )}
                </For>
              </Show>
            </div>
          </AnchoredMenu>
        )}
      </Show>
    </>
  );
}
