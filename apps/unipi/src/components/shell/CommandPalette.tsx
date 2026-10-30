/**
 * Ctrl/⌘+K command palette (UNI-117 desktop shell): a centred dialog with a
 * search field and the registered commands (lib/shell.ts), grouped. Arrow
 * keys move, Enter runs, Esc closes. Mouse hover selects.
 */
import { createEffect, createMemo, createSignal, For, onCleanup, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import { Search } from 'lucide-solid';
import { allCommands, paletteOpen, rankCommands, setPaletteOpen, type Command } from '../../lib/shell';
import { pushBack } from '../../lib/back';

export function CommandPalette() {
  return (
    <Show when={paletteOpen()}>
      <PaletteDialog />
    </Show>
  );
}

function PaletteDialog() {
  const [query, setQuery] = createSignal('');
  const [index, setIndex] = createSignal(0);
  let input: HTMLInputElement | undefined;
  let list: HTMLDivElement | undefined;
  const results = createMemo(() => rankCommands(allCommands(), query()));
  const close = () => setPaletteOpen(false);
  createEffect(() => {
    query();
    setIndex(0);
  });
  createEffect(() => {
    const row = list?.querySelector<HTMLElement>(`[data-index="${index()}"]`);
    row?.scrollIntoView?.({ block: 'nearest' });
  });
  onCleanup(pushBack(close));
  queueMicrotask(() => input?.focus());

  const run = (command: Command | undefined) => {
    if (!command) return;
    close();
    // After the dialog unmounts, so focus/scroll land on the new screen.
    queueMicrotask(() => command.run());
  };
  const onKey = (event: KeyboardEvent) => {
    const count = results().length;
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      setIndex((i) => (count ? (i + 1) % count : 0));
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      setIndex((i) => (count ? (i - 1 + count) % count : 0));
    } else if (event.key === 'Enter') {
      event.preventDefault();
      run(results()[index()]);
    }
  };
  // Group headers only when the query is empty (ranked results read as one list).
  const groupStart = (i: number) => !query().trim() && (i === 0 || results()[i - 1]!.group !== results()[i]!.group);

  return (
    <Portal>
      <div class="palette-backdrop" onClick={close} />
      <div class="palette" role="dialog" aria-modal="true" aria-label="Command palette" data-testid="command-palette">
        <div class="palette-search">
          <Search size={18} />
          <input
            ref={input}
            class="palette-input"
            placeholder="Type a command or screen…"
            aria-label="Command"
            aria-controls="palette-list"
            aria-activedescendant={results().length ? `palette-row-${index()}` : undefined}
            value={query()}
            onInput={(event) => setQuery(event.currentTarget.value)}
            onKeyDown={onKey}
            autocomplete="off"
            spellcheck={false}
          />
          <kbd class="kbd">Esc</kbd>
        </div>
        <div class="palette-list" id="palette-list" role="listbox" ref={list}>
          <For each={results()} fallback={<p class="palette-empty">No matching commands</p>}>
            {(command, i) => (
              <>
                <Show when={groupStart(i())}>
                  <p class="palette-group">{command.group}</p>
                </Show>
                <button
                  type="button"
                  role="option"
                  id={`palette-row-${i()}`}
                  data-index={i()}
                  class="palette-row"
                  aria-selected={i() === index()}
                  onMouseMove={() => setIndex(i())}
                  onClick={() => run(command)}
                >
                  <span class="palette-row-icon">{command.icon?.()}</span>
                  <span class="palette-row-title">{command.title}</span>
                  <Show when={query().trim()}>
                    <span class="palette-row-group">{command.group}</span>
                  </Show>
                  <Show when={command.keys}>
                    <span class="palette-row-keys">
                      <For each={command.keys!.split(' ')}>{(key) => <kbd class="kbd">{key}</kbd>}</For>
                    </span>
                  </Show>
                </button>
              </>
            )}
          </For>
        </div>
        <div class="palette-foot">
          <span><kbd class="kbd">↑</kbd><kbd class="kbd">↓</kbd> move</span>
          <span><kbd class="kbd">↵</kbd> run</span>
          <span><kbd class="kbd">Esc</kbd> close</span>
        </div>
      </div>
    </Portal>
  );
}
