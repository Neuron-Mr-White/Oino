/**
 * Board bottom sheet (UNI-130): a Corvu drawer in the app's `.sheet`
 * look that rises with the soft keyboard (the `.sheet` rules cap it to
 * the visible viewport), closes on Android Back (pushBack) and swipe
 * down. `onClose` is a *request*: a guarded sheet (a dirty New task
 * form) can refuse by not flipping `open`.
 */
import { createEffect, onCleanup, Show, type JSX } from 'solid-js';
import Drawer from 'corvu/drawer';
import { X } from 'lucide-solid';
import { pushBack } from '../../lib/back';
import { wrapTab } from '../../lib/focusTrap';

export function BoardSheet(props: {
  open: boolean;
  onClose: () => void;
  /** Accessible name (also the visible title unless `title` is given). */
  label: string;
  title?: JSX.Element;
  /** Right side of the title row (e.g. a Clear button); a close X otherwise. */
  headerAction?: JSX.Element;
  class?: string;
  /** Sticky bottom row (primary action) that stays above the keyboard. */
  footer?: JSX.Element;
  children: JSX.Element;
  testId?: string;
  /** Drag-to-dismiss off (a dirty form): Back / ✕ / outside tap still ask via onClose. */
  guarded?: boolean;
}) {
  createEffect(() => {
    if (!props.open) return;
    onCleanup(pushBack(() => props.onClose()));
  });
  return (
    <Drawer
      open={props.open}
      onOpenChange={(open) => {
        if (!open) props.onClose();
      }}
      side="bottom"
      restoreFocus={false}
      // A field marked `autofocus` gets the first focus (Rename, New folder, Search…),
      // not the header's ✕ — the focus trap would otherwise pick the first button.
      onInitialFocus={(event: Event) => {
        const content = event.currentTarget as HTMLElement | null;
        const field = content?.querySelector<HTMLElement>('[autofocus]');
        event.preventDefault();
        // No field to type in: focus the dialog itself (UNI-218 F8), so the ✕ doesn't
        // open with a focus ring; Tab then walks into the controls as usual.
        if (!field) {
          content?.setAttribute('tabindex', '-1');
          content?.focus({ preventScroll: true });
          return;
        }
        field.focus({ preventScroll: true });
        if (field instanceof HTMLInputElement && field.type === 'text') field.select();
      }}
      // Escape = Android Back, handled once by pushBack (a guarded sheet
      // must not see two close requests for one key press).
      closeOnEscapeKeyDown={false}
      // Focus leaving the WebView (Android's package installer / share sheet / file picker opening
      // over the app) blurs the page; corvu read that as "focus moved outside" and closed the
      // sheet — the push sheet vanished mid-install with its "installer is open" state unseen.
      // Outside *taps* still close it (the overlay).
      closeOnOutsideFocus={false}
    >
      <Drawer.Portal>
        <Drawer.Overlay class="sheet-overlay" />
        <Drawer.Content
          class={`sheet board-sheet ${props.class ?? ''}`}
          data-testid={props.testId}
          data-corvu-no-drag={props.guarded ? '' : undefined}
          // Our own Tab wrap (capture, before corvu's): its trap counts disabled /
          // hidden controls as the ends and let Tab escape into the page.
          onKeyDown={(event: KeyboardEvent) => {
            if (wrapTab(event, event.currentTarget as HTMLElement)) event.stopPropagation();
          }}
        >
          <div class="board-sheet-head">
            <Drawer.Label class="board-sheet-title">{props.title ?? props.label}</Drawer.Label>
            <Show
              when={props.headerAction}
              fallback={
                <button type="button" class="ghost-icon-button board-sheet-close" aria-label="Close" onClick={() => props.onClose()}>
                  <X size={20} />
                </button>
              }
            >
              {props.headerAction}
            </Show>
          </div>
          <div class="board-sheet-body">{props.children}</div>
          <Show when={props.footer}>
            <div class="board-sheet-foot">{props.footer}</div>
          </Show>
        </Drawer.Content>
      </Drawer.Portal>
    </Drawer>
  );
}
