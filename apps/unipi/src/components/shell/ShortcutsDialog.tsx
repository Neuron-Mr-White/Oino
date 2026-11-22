/**
 * The keyboard shortcuts list (Ctrl/Cmd+/ or the palette's "Keyboard
 * shortcuts", WP-A18/Z-94): the shell chords from the shortcut registry
 * (user rebinding applied), the resolved extension shortcuts (G-7: their
 * rebindings noted, `unipi.tray.open` shown as intercepted), and the
 * unipi screen keys the screens own. Rebinding happens in Settings
 * (ShortcutsSettings).
 */
import { createSignal, For, Show } from 'solid-js';
import { BoardSheet } from '../board/BoardSheet';
import { appEntries, chordTokens, extChordTokens, extEntries, isMac, screenKeyGroups } from '../../lib/shortcuts/registry';

const [open, setOpen] = createSignal(false);
export const shortcutsOpen = open;
export const setShortcutsOpen = setOpen;

const GROUP_ORDER = ['Shell', 'Panels', 'Chat', 'Extensions'];

export function ShortcutsDialog() {
  return (
    <BoardSheet open={open()} onClose={() => setOpen(false)} label="Keyboard shortcuts" testId="shortcuts-dialog" class="shortcuts-dialog">
      <div class="shortcuts-groups">
        <For each={shellGroups()}>
          {(group) => (
            <section class="shortcuts-group">
              <h3 class="shortcuts-group-title">{group.title}</h3>
              <For each={group.items}>
                {(item) => (
                  <div class="shortcuts-row" data-testid="shortcut-row">
                    <span class="shortcuts-what">
                      {item.title}
                      {/* The note sits under the title: beside the keys it squeezed the title to one word a line. */}
                      <Show when={item.note}>
                        <span class="shortcuts-note shortcuts-note-under">{item.note}</span>
                      </Show>
                    </span>
                    <span class="shortcuts-keys">
                      <For each={item.tokens}>{(part) => <kbd class="kbd">{part}</kbd>}</For>
                    </span>
                  </div>
                )}
              </For>
            </section>
          )}
        </For>
        <For each={screenKeyGroups()}>
          {(group) => (
            <section class="shortcuts-group">
              <h3 class="shortcuts-group-title">{group.title}</h3>
              <For each={group.items}>
                {(item) => (
                  <div class="shortcuts-row">
                    <span class="shortcuts-what">{item.what}</span>
                    <span class="shortcuts-keys">
                      <For each={item.keys.split(' ')}>{(part) => (part.length <= 12 && !/^[…/]$/.test(part) ? <kbd class="kbd">{part}</kbd> : <span class="shortcuts-sep">{part}</span>)}</For>
                    </span>
                  </div>
                )}
              </For>
            </section>
          )}
        </For>
      </div>
      <p class="shortcuts-rebind">Rebind any of these in Settings · Keyboard shortcuts.</p>
    </BoardSheet>
  );
}

type ShellRow = { title: string; tokens: string[]; note?: string };

function shellGroups(): Array<{ title: string; items: ShellRow[] }> {
  const mac = isMac();
  const entries = appEntries();
  const out: Array<{ title: string; items: ShellRow[] }> = [];
  for (const group of GROUP_ORDER) {
    const items: ShellRow[] = entries
      .filter((e) => e.group === group)
      .map((e) => ({
        title: e.title,
        tokens: chordTokens(e.chord, mac),
        ...(e.rebound ? { note: `was ${chordTokens(e.defaults[0] ?? e.chord, mac).join(mac ? '' : '+')}` } : {}),
      }));
    if (group === 'Chat') {
      for (const ext of extEntries()) {
        items.push({
          title: ext.title,
          tokens: extChordTokens(ext.chord, mac),
          ...(ext.note
            ? { note: ext.note }
            : ext.reboundFrom
              ? { note: `extension wanted ${chordTokens(ext.reboundFrom, mac).join(mac ? '' : '+')}` }
              : {}),
        });
      }
    }
    if (items.length) out.push({ title: group, items });
  }
  return out;
}
