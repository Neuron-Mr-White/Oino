/**
 * Shortcut rebinding (WP-A18, Z-94): Settings › Keyboard shortcuts. One row
 * per app binding — "Change" records the next chord press (Esc cancels,
 * Backspace resets to the default), the user chord persists in app settings
 * (`shortcuts.chords`) and applies everywhere at once. Extension shortcuts
 * list read-only with their resolved chords (they are the extension's).
 */
import { createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { Keyboard, RotateCcw } from 'lucide-solid';
import { appEntries, chordOf, chordTokens, extEntries, isMac, normalizeChord, rebindShortcut } from '../../lib/shortcuts/registry';

export function ShortcutsSettings() {
  const [recording, setRecording] = createSignal<string | undefined>();
  const rows = () => appEntries().filter((e) => !e.displayOnly);

  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      const id = recording();
      if (!id) return;
      e.preventDefault();
      e.stopPropagation();
      if (e.key === 'Escape') return setRecording(undefined);
      if (e.key === 'Backspace' || e.key === 'Delete') {
        rebindShortcut(id, null);
        return setRecording(undefined);
      }
      const chord = chordOf(e);
      if (chord && normalizeChord(chord)) {
        rebindShortcut(id, chord);
        setRecording(undefined);
      }
    };
    window.addEventListener('keydown', onKey, true);
    onCleanup(() => window.removeEventListener('keydown', onKey, true));
  });

  return (
    <div class="sc-set" data-testid="shortcuts-settings">
      <For each={rows()}>
        {(entry) => (
          <div class="sc-set-row" data-testid={`sc-row-${entry.id}`}>
            <span class="sc-set-title">{entry.title}</span>
            <span class="sc-set-keys">
              <Show when={recording() === entry.id} fallback={<kbd class="kbd">{chordTokens(entry.chord, isMac()).join(isMac() ? '' : '+')}</kbd>}>
                <span class="sc-set-record">Press keys…</span>
              </Show>
              <Show when={entry.rebound}>
                <button type="button" class="sc-set-reset" aria-label={`Reset ${entry.title} to ${chordTokens(entry.defaults[0] ?? '', isMac()).join(isMac() ? '' : '+')}`} title="Reset to default" onClick={() => rebindShortcut(entry.id, null)}>
                  <RotateCcw size={13} />
                </button>
              </Show>
              <button type="button" class="sc-set-change" onClick={() => setRecording(recording() === entry.id ? undefined : entry.id)}>
                {recording() === entry.id ? 'Cancel' : 'Change'}
              </button>
            </span>
          </div>
        )}
      </For>
      <Show when={extEntries().length > 0}>
        <p class="sc-set-ext-title">
          <Keyboard size={13} /> Extension shortcuts (as bound; conflicts with app keys are rebound automatically)
        </p>
        <For each={extEntries()}>
          {(ext) => (
            <div class="sc-set-row sc-set-row-ext">
              <span class="sc-set-title">{ext.title}</span>
              <span class="sc-set-keys">
                <kbd class="kbd">{ext.interceptedBy ? 'Shift ↓' : ext.chord ? chordTokens(ext.chord, isMac()).join(isMac() ? '' : '+') : 'unbound'}</kbd>
                <Show when={ext.reboundFrom}>
                  <span class="sc-set-note">extension wanted {chordTokens(ext.reboundFrom!, isMac()).join(isMac() ? '' : '+')}</span>
                </Show>
              </span>
            </div>
          )}
        </For>
      </Show>
    </div>
  );
}
