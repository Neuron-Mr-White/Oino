/**
 * `editorReplace` slot host (WP-U10; ui-protocol §6.2, app spec §3.11): the
 * composer tray — the stack's top entry takes the composer's place in the
 * bottom dock (dialogs still win above it; widgets stay). Esc returns the
 * composer by sending the panel `$close` (§4.6 system actions); a deleted
 * entry restores it too (§6.6). Auto mode is clamped to 50 vh.
 */
import { createEffect, on, onCleanup, Show } from 'solid-js';
import { X } from 'lucide-solid';
import { EntryView } from '../NodeView';
import { uiTransport } from '../actions';
import type { UiEntry } from '../types';
import type { UiTarget } from '../actions';

/** Locally dismissed `id:seq` pairs (an Esc while offline); a reopen changes `seq`. */
const dismissed = new Set<string>();

export function isDismissed(entry: UiEntry): boolean {
  return dismissed.has(`${entry.id}:${entry.seq}`);
}

export function dismissEntry(entry: UiEntry): void {
  dismissed.add(`${entry.id}:${entry.seq}`);
}

export function resetTrayDismissals(): void {
  dismissed.clear();
}

/** The tray's top visible entry (stack order: focused/most recent last). */
export function trayEntry(stack: UiEntry[]): UiEntry | undefined {
  return [...stack].reverse().find((e) => !isDismissed(e));
}

export function ComposerTray(props: { stack: UiEntry[]; target: UiTarget }) {
  const top = () => trayEntry(props.stack);
  let shown: { target: UiTarget; id: string } | undefined;
  createEffect(
    on(
      () => props.stack.map((e) => `${e.id}:${e.seq}`).join('|'),
      () => {
        const e = top();
        shown = e ? { target: props.target, id: e.id } : undefined;
        if (e) uiTransport().visible(props.target, e.id, true).catch(() => {});
      },
    ),
  );
  onCleanup(() => {
    if (shown) uiTransport().visible(shown.target, shown.id, false).catch(() => {});
    shown = undefined;
  });
  const close = (entry: UiEntry) => {
    dismissEntry(entry);
    uiTransport().action(props.target, { entryId: entry.id, actionId: '$close' }).catch(() => {});
  };
  return (
    <Show when={top()} keyed>
      {(entry) => (
        <div
          class="ui-tray"
          data-entry={entry.id}
          data-testid="ui-tray"
          onKeyDown={(e) => {
            if (e.key === 'Escape' && !e.defaultPrevented) {
              e.preventDefault();
              close(entry);
            }
          }}
        >
          <div class="ui-tray-head">
            <span class="ui-tray-title" title={entry.title ?? entry.id}>
              {entry.title ?? entry.extensionId}
            </span>
            <span class="flex-1" />
            <button type="button" class="ui-tray-close" aria-label={`Close ${entry.title ?? 'panel'} (Esc)`} onClick={() => close(entry)}>
              <X size={15} />
            </button>
          </div>
          <div class="ui-tray-body">
            <EntryView entry={entry} target={props.target} mode="auto" />
          </div>
        </div>
      )}
    </Show>
  );
}
