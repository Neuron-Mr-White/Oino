/**
 * Composer (DESIGN.md §3.5 MVP, Moshi pattern): a compact card docked
 * above the soft keyboard that REPLACES the toolbar while open. Send
 * = bracketed paste + Enter. History (last 50 per host) lives behind
 * the History icon; picking one sends immediately.
 */
import { createEffect, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { ArrowUp, ChevronLeft, CornerDownLeft, History, ListTree, X } from 'lucide-solid';
import { pushBack } from '../lib/back';
import { useRecents } from '../lib/recentsContext';
import { useSettings } from '../settings/context';
import { composeSend } from '../lib/composer';
import { ARROW_BYTES } from '../lib/termToolbar';
import ArrowPad from './ArrowPad';

export type ComposerProps = {
  hostId: string;
  label: string;
  onClose: () => void;
  onSend: (bytes: string) => void;
  /** Opens the shortcuts panel (composer closes first). */
  onShortcuts: () => void;
};

export default function Composer(props: ComposerProps) {
  const recents = useRecents();
  const [settings] = useSettings();
  const [text, setText] = createSignal('');
  const [historyOpen, setHistoryOpen] = createSignal(false);
  let field: HTMLTextAreaElement | undefined;

  const autocorrect = () => settings.input.composerAutocorrect;

  // Android Back closes the composer (the history panel on top of it
  // pushes its own handler and closes first).
  onMount(() => onCleanup(pushBack(() => props.onClose())));

  // Focus on open so the soft keyboard comes up.
  createEffect(() => {
    if (field) field.focus();
  });

  function autoGrow() {
    if (!field) return;
    field.style.height = 'auto';
    field.style.height = `${Math.min(field.scrollHeight, 5 * 22 + 8)}px`;
  }

  function send() {
    const value = text();
    if (value.trim() === '') return;
    props.onSend(composeSend(value));
    recents.pushSend(props.hostId, value);
    setText('');
    if (field) field.style.height = 'auto';
  }

  return (
    <div class="composer-card" role="group" aria-label={`Composer for ${props.label}`}>
      <textarea
        ref={field}
        class="composer-line"
        rows={1}
        placeholder={`Send to ${props.label}…`}
        value={text()}
        onInput={(event) => {
          setText(event.currentTarget.value);
          autoGrow();
        }}
        autocorrect={autocorrect() ? 'on' : 'off'}
        autocapitalize={autocorrect() ? 'sentences' : 'off'}
        spellcheck={autocorrect()}
        data-autocorrect={autocorrect() ? 'on' : 'off'}
      />
      <div class="mt-2 flex items-center justify-between">
        <div class="flex items-center gap-1">
          <button type="button" aria-label="Close composer" class="composer-icon" onClick={props.onClose}>
            <X size={22} />
          </button>
          <button
            type="button"
            aria-label="History"
            class="composer-icon"
            onClick={() => setHistoryOpen(!historyOpen())}
          >
            <History size={22} />
          </button>
          <button
            type="button"
            aria-label="Shortcuts"
            class="composer-icon"
            onClick={() => {
              props.onClose();
              props.onShortcuts();
            }}
          >
            <ListTree size={22} />
          </button>
          <ArrowPad
            class="composer-icon"
            label="Arrows (press and drag)"
            iconSize={22}
            haptics={settings.input.haptics}
            onArrow={(direction) => props.onSend(ARROW_BYTES[direction])}
          />
        </div>
        <button
          type="button"
          aria-label="Send (bracketed paste + Enter)"
          class={`grid h-10 w-10 aspect-square shrink-0 place-items-center rounded-full ${
            text().trim() === '' ? 'bg-surface-3 text-text-muted' : 'bg-shell text-bg'
          }`}
          onClick={send}
        >
          <ArrowUp size={20} />
        </button>
      </div>
      <Show when={historyOpen()}>
        <HistorySheet
          hostId={props.hostId}
          onClose={() => setHistoryOpen(false)}
          onPick={(entry) => {
            setHistoryOpen(false);
            setText(entry);
            props.onSend(composeSend(entry));
            recents.pushSend(props.hostId, entry);
          }}
        />
      </Show>
    </div>
  );
}

/**
 * The per-host composer send history (last 50). Picking one sends it
 * immediately (bracketed paste + Enter), per Moshi. The header's back
 * button, the close button and Android Back all return to where the
 * panel was opened from (UNI-128: the empty state had no way out).
 */
export function HistorySheet(props: {
  hostId: string;
  onClose: () => void;
  onPick: (entry: string) => void;
}) {
  const recents = useRecents();
  onMount(() => onCleanup(pushBack(() => props.onClose())));
  return (
    <div
      class="history-sheet fixed inset-x-3 z-50 flex flex-col rounded-[20px] bg-surface-2 p-2 shadow-[0_-12px_40px_rgb(0_0_0/50%)]"
      role="dialog"
      aria-label="Composer send history"
    >
      <div class="flex items-center gap-1 pb-1">
        <button
          type="button"
          aria-label="Back"
          class="grid h-10 w-10 shrink-0 place-items-center rounded-full text-text-muted active:bg-surface-3"
          onClick={() => props.onClose()}
        >
          <ChevronLeft size={22} />
        </button>
        <p class="min-w-0 flex-1 truncate text-[15px] font-medium text-text">History</p>
        <button
          type="button"
          aria-label="Close history"
          class="grid h-10 w-10 shrink-0 place-items-center rounded-full text-text-muted active:bg-surface-3"
          onClick={() => props.onClose()}
        >
          <X size={20} />
        </button>
      </div>
      <div class="min-h-0 flex-1 overflow-y-auto overscroll-contain" role="listbox" aria-label="Sent messages">
        <For each={recents.sendHistoryFor(props.hostId)}>
          {(entry) => (
            <button
              type="button"
              role="option"
              class="flex w-full items-center gap-2 rounded-input px-3 py-2.5 text-left text-[15px] text-text [&:not(:last-child)]:border-b [&:not(:last-child)]:border-line/40"
              onClick={() => props.onPick(entry)}
            >
              <CornerDownLeft size={16} class="shrink-0 text-text-dim" />
              <span class="min-w-0 flex-1 truncate">{entry}</span>
            </button>
          )}
        </For>
        <Show when={recents.sendHistoryFor(props.hostId).length === 0}>
          <p class="px-3 py-3 text-[14px] text-text-muted">No sends yet.</p>
        </Show>
      </div>
    </div>
  );
}
