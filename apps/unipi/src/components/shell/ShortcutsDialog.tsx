/**
 * The desktop keyboard shortcuts (UNI-117 part 2): Ctrl/⌘+/ or the
 * palette's "Keyboard shortcuts". One list (lib/desktop.ts `shortcuts`),
 * grouped by screen, in the standard centred dialog.
 */
import { createSignal, For } from 'solid-js';
import { BoardSheet } from '../board/BoardSheet';
import { shortcuts } from '../../lib/desktop';
import { modKey } from '../../lib/shell';

const [open, setOpen] = createSignal(false);
export const shortcutsOpen = open;
export const setShortcutsOpen = setOpen;

export function ShortcutsDialog() {
  return (
    <BoardSheet open={open()} onClose={() => setOpen(false)} label="Keyboard shortcuts" testId="shortcuts-dialog" class="shortcuts-dialog">
      <div class="shortcuts-groups">
        <For each={shortcuts(modKey())}>
          {(group) => (
            <section class="shortcuts-group">
              <h3 class="shortcuts-group-title">{group.title}</h3>
              <For each={group.items}>
                {(item) => (
                  <div class="shortcuts-row">
                    <span class="shortcuts-keys">
                      <For each={item.keys.split(' ')}>{(part) => (part.length <= 12 && !/^[…/]$/.test(part) ? <kbd class="kbd">{part}</kbd> : <span class="shortcuts-sep">{part}</span>)}</For>
                    </span>
                    <span class="shortcuts-what">{item.what}</span>
                  </div>
                )}
              </For>
            </section>
          )}
        </For>
      </div>
    </BoardSheet>
  );
}
