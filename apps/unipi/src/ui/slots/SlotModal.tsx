/**
 * `overlay` slot host (WP-U10, G-1; ui-protocol §6.2): a stack — the most
 * recently opened or focused entry shows as a centred modal on the desktop
 * (width by `size`: 420/560/800/full, `--radius-float`) and a bottom sheet
 * on the phone (`large`/`full` = full height, §13.6 row 6). Title bar with
 * ✕, `keys` as footer chips, Esc/scrim/✕ send the panel `$close` (§4.6; a
 * deleted entry closes the host, §6.6). Pending dialogs (§3) sit above:
 * the dock's dialog panel outranks this modal (§6.4 "dialogs always above").
 */
import { createEffect, on, onCleanup, For, Show } from 'solid-js';
import { X } from 'lucide-solid';
import { EntryView } from '../NodeView';
import { ICONS } from '../nodes/icons';
import { plainLine } from '../nodes/Text';
import { uiTransport } from '../actions';
import type { UiEntry } from '../types';
import type { UiTarget } from '../actions';

const SIZE_WIDTH: Record<string, string> = { small: '420px', medium: '560px', large: '800px', full: '100%' };

/** Locally dismissed `id:seq` pairs; reopening (new `seq`) shows again. */
const dismissed = new Set<string>();
export const resetOverlayDismissals = () => dismissed.clear();

export function SlotModal(props: { stack: UiEntry[]; target: UiTarget; phone?: boolean }) {
  const top = () => [...props.stack].reverse().find((e) => !dismissed.has(`${e.id}:${e.seq}`));
  createEffect(
    on(
      () => props.stack.map((e) => `${e.id}:${e.seq}`).join('|'),
      () => {
        const e = top();
        if (e) uiTransport().visible(props.target, e.id, true).catch(() => {});
      },
    ),
  );
  onCleanup(() => {
    const e = top();
    if (e) uiTransport().visible(props.target, e.id, false).catch(() => {});
  });
  const close = (entry: UiEntry) => {
    dismissed.add(`${entry.id}:${entry.seq}`);
    uiTransport().action(props.target, { entryId: entry.id, actionId: '$close' }).catch(() => {});
  };
  return (
    <Show when={top()} keyed>
      {(entry) => {
        // `large`/`full` fill the phone sheet (§13.6 row 6); desktop keeps the width.
        const full = () => entry.size === 'full' || (props.phone === true && entry.size === 'large');
        const Icon = entry.icon ? ICONS[entry.icon] : undefined;
        const badge = () => plainLine(entry.badge);
        return (
          <div
            class="ui-modal-scrim"
            data-testid="ui-modal-scrim"
            onKeyDown={(e) => {
              if (e.key === 'Escape' && !e.defaultPrevented && entry.closeOnEscape !== false) {
                e.preventDefault();
                close(entry);
              }
            }}
            tabIndex={-1}
          >
            <div
              class="ui-modal"
              role="dialog"
              aria-modal="true"
              aria-label={entry.title ?? 'Panel'}
              data-size={entry.size ?? 'medium'}
              data-full={full() ? '' : undefined}
              data-testid="ui-modal"
              style={{ '--ui-modal-w': SIZE_WIDTH[entry.size ?? 'medium'] ?? SIZE_WIDTH.medium }}
            >
              <div class="ui-modal-head">
                <Show when={Icon}>
                  {(I) => {
                    const C = I();
                    return <C size={15} class="ui-modal-icon" />;
                  }}
                </Show>
                <span class="ui-modal-title" title={entry.title ?? entry.id}>
                  {entry.title ?? entry.id}
                </span>
                <Show when={badge()}>
                  <span class="ui-modal-badge">{badge()}</span>
                </Show>
                <span class="flex-1" />
                <button type="button" class="ui-modal-x" aria-label={`Close ${entry.title ?? 'panel'}`} onClick={() => close(entry)}>
                  <X size={17} />
                </button>
              </div>
              <div class="ui-modal-body">
                <EntryView entry={entry} target={props.target} mode="auto" />
              </div>
              <Show when={(entry.keys ?? []).length > 0}>
                <div class="ui-modal-keys" aria-label="Panel keys">
                  <For each={entry.keys}>{(k) => <kbd class="kbd ui-modal-key" title={k.key}>{k.label}</kbd>}</For>
                </div>
              </Show>
            </div>
          </div>
        );
      }}
    </Show>
  );
}
