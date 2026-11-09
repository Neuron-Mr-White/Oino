/**
 * `overlay` slot host (WP-U10, G-1; ui-protocol §6.2): a stack — the most
 * recently opened or focused entry shows as a centred modal on the desktop
 * (width by `size`: 420/560/800/full, `--radius-float`) and a bottom sheet
 * on the phone (`large`/`full` = full height, §13.6 row 6). Title bar with
 * ✕, `keys` as footer chips, Esc/scrim/✕ send the panel `$close` (§4.6; a
 * deleted entry closes the host, §6.6). Pending dialogs (§3) sit above:
 * the dock's dialog panel outranks this modal (§6.4 "dialogs always above").
 */
import { createEffect, createSignal, on, onCleanup, For, Show } from 'solid-js';
import { X } from 'lucide-solid';
import { EntryView } from '../NodeView';
import { ICONS } from '../nodes/icons';
import { plainLine } from '../nodes/Text';
import { uiTransport } from '../actions';
import { reservedGate, reservedRowFor } from './reserved';
import { desktopUpdateState, isUpdateDismissed, promptOpen, saveDismissal, SNOOZE_MS } from '../../lib/desktopUpdate';
import { closeUpdateForSession, setExtensionUpdateOpen } from '../../lib/updateSurface';
import { wrapTab } from '../../lib/focusTrap';
import type { UiEntry } from '../types';
import type { UiTarget } from '../actions';

const SIZE_WIDTH: Record<string, string> = { small: '420px', medium: '560px', large: '800px', full: '100%' };

/** Locally dismissed `id:seq` pairs; reopening (new `seq`) shows again. */
const dismissed = new Set<string>();
/** Bumped on every local dismissal: the Set is not reactive, `top()` reads this. */
const [dismissRev, setDismissRev] = createSignal(0);
export const resetOverlayDismissals = () => {
  dismissed.clear();
  setDismissRev((n) => n + 1);
};

/**
 * §8 rule 1 (WP-U12): a reserved entry whose native surface *is* the overlay
 * (the Session modal, A17) never draws here — its component owns the modal, and
 * a second, empty one would be a duplicate surface. Every other reserved entry
 * falls through and shows its owner's `fallback` tree (rule 3).
 */
const nativeOverlay = (entry: UiEntry): boolean => {
  const row = reservedRowFor(entry);
  return !!row && row.container === 'overlay' && reservedGate(row);
};

/** Updater payload contains the release version in its tree, not panel metadata. */
function updaterVersion(entry: UiEntry): string | undefined {
  if (entry.id !== 'unipi.updater/update') return undefined;
  const text = JSON.stringify(entry.view);
  return /(?:Oino[^0-9]*|Latest[^0-9]*)([0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?)/.exec(text)?.[1]
    ?? /([0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?)/.exec(text)?.[1];
}

export function SlotModal(props: { stack: UiEntry[]; target: UiTarget; phone?: boolean; /** Drop the terminal key chips (the GUI has buttons). */ hideKeys?: boolean }) {
  const updater = () => [...props.stack].reverse().find(e => e.id === 'unipi.updater/update' && !dismissed.has(`${e.id}:${e.seq}`));
  const desktopOwns = () => {
    const s = desktopUpdateState();
    return 'info' in s && s.info?.kind === 'in_app' && promptOpen();
  };
  const top = () => (void dismissRev(), [...props.stack].reverse().find((e) => !dismissed.has(`${e.id}:${e.seq}`) && !nativeOverlay(e)
    && (e.id !== 'unipi.updater/update' || (!desktopOwns() && !isUpdateDismissed(updaterVersion(e))))));
  createEffect(() => { const e = updater(); setExtensionUpdateOpen(!!e && top()?.id === e.id); });
  onCleanup(() => setExtensionUpdateOpen(false));
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
    // Never read `props.stack` / `top()` here: during Conversation unmount,
    // parent memos (`registry`) have already been cleaned by Solid's `cleanNode`.
    if (shown) uiTransport().visible(shown.target, shown.id, false).catch(() => {});
    shown = undefined;
  });
  const close = (entry: UiEntry, localOnly = false) => {
    if (entry.id === 'unipi.updater/update' && localOnly) closeUpdateForSession(updaterVersion(entry));
    dismissed.add(`${entry.id}:${entry.seq}`);
    setDismissRev((n) => n + 1);
    if (!localOnly) uiTransport().action(props.target, { entryId: entry.id, actionId: '$close' }).catch(() => {});
  };
  return (
    <Show when={top()} keyed>
      {(entry) => {
        // `large`/`full` fill the phone sheet (§13.6 row 6); desktop keeps the width.
        const full = () => entry.size === 'full' || (props.phone === true && entry.size === 'large');
        let dialog: HTMLDivElement | undefined;
        const previous = document.activeElement as HTMLElement | null;
        queueMicrotask(() => dialog?.focus());
        onCleanup(() => previous?.isConnected && previous.focus());
        const Icon = entry.icon ? ICONS[entry.icon] : undefined;
        const badge = () => plainLine(entry.badge);
        return (
          <div
            class="ui-modal-scrim"
            data-testid="ui-modal-scrim"
            onKeyDown={(e) => {
              if (e.key === 'Escape' && !e.defaultPrevented && entry.closeOnEscape !== false) {
                e.preventDefault();
                close(entry, entry.id === 'unipi.updater/update');
              }
            }}
            tabIndex={-1}
          >
            <div
              ref={dialog}
              tabIndex={-1}
              onKeyDown={(e) => wrapTab(e, e.currentTarget)}
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
                <EntryView entry={entry} target={props.target} mode="auto" onAction={(p) => {
                  if (entry.id !== 'unipi.updater/update' || !['skip', 'close'].includes(p.actionId)) return;
                  const version = updaterVersion(entry);
                  if (version) saveDismissal({ version, ...(p.actionId === 'skip' ? {} : { until: Date.now() + SNOOZE_MS }) });
                  close(entry, true);
                }} />
              </div>
              <Show when={!props.hideKeys && entry.id !== 'unipi.updater/update' && (entry.keys ?? []).length > 0}>
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
