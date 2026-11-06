/**
 * The one overlay type on the phone (phone spec §2.7, §2.8): a bottom sheet
 * with a drag handle, optional title row, scrolling body and sticky footer,
 * detents (content / half / full), drag / scrim / Back to dismiss, and a
 * "Discard changes?" guard. On the desktop shell the same markup renders as
 * a centred dialog (the existing `.sheet` desktop rules, like BoardSheet).
 *
 * `SheetFrame` is the shared core (BoardSheet re-bases on it with its old
 * "guarded = no drag, onClose is a request" contract); `Sheet` adds the
 * discard confirmation; `ActionSheet` / `confirmSheet` build on `Sheet`.
 */
import { createEffect, createSignal, For, onCleanup, Show, type Component, type JSX } from 'solid-js';
import { Dynamic, render } from 'solid-js/web';
import Drawer from 'corvu/drawer';
import { Check, X } from 'lucide-solid';
import { pushBack } from '../../lib/back';
import { wrapTab } from '../../lib/focusTrap';
import { layout } from '../../lib/layout';
import { haptic, trackSheetOpen } from './services';

export type Detent = 'content' | 'half' | 'full';

/** Pure: the corvu snap points for a detent list (fractions of the drawer's height). */
export function snapPointsFor(detents: readonly Detent[]): number[] {
  if (detents.includes('half') && detents.includes('full')) return [0, 0.5, 1];
  return [0, 1];
}

/** Pure: the drawer's sizing class: the tallest detent decides the height. */
export function sizeFor(detents: readonly Detent[]): Detent {
  if (detents.includes('full')) return 'full';
  if (detents.includes('half')) return 'half';
  return 'content';
}

/**
 * Floating layers portaled out of the drawer (Kobalte Select/Combobox/Menu/Popover,
 * the app's own dialogs and menus). A tap inside one of them belongs to that
 * layer — it must not read as a scrim tap on the sheet below.
 */
const FLOATING_LAYER = '[data-popper-positioner],[data-kb-top-layer],[data-floating-layer],[role="listbox"],[role="menu"],[role="dialog"],[role="alertdialog"],[aria-modal="true"]';

/** Pure: is this pointer/focus event target inside a portaled floating layer (not a real outside tap)? */
export function inFloatingLayer(target: EventTarget | null): boolean {
  return target instanceof Element && !!target.closest(FLOATING_LAYER);
}

export type SheetFrameProps = {
  open: boolean;
  /** A request: the caller flips `open` (or refuses). */
  onClose: () => void;
  label: string;
  title?: JSX.Element | false;
  headerAction?: JSX.Element;
  detents?: Detent[];
  initialDetent?: Detent;
  footer?: JSX.Element;
  /** Drag-to-dismiss off (Back / ✕ / scrim still call onClose). */
  noDrag?: boolean;
  /** Extra classes on the sheet element. */
  class?: string;
  /** BoardSheet keeps its classic desktop-style head (✕ on the right everywhere). */
  classicHead?: boolean;
  /** Stacking: a sheet opened over another sheet. */
  level?: 1 | 2;
  testId?: string;
  children: JSX.Element;
};

export function SheetFrame(props: SheetFrameProps) {
  const detents = () => (props.detents?.length ? props.detents : (['content'] as Detent[]));
  const size = () => sizeFor(detents());
  const snaps = () => snapPointsFor(detents());
  const initial = () => {
    const want = props.initialDetent ?? detents()[0]!;
    return snaps().length === 3 ? (want === 'half' ? 0.5 : 1) : 1;
  };
  const [snap, setSnap] = createSignal<number>(initial());
  createEffect(() => {
    if (!props.open) return;
    setSnap(initial());
    onCleanup(pushBack(() => props.onClose()));
    onCleanup(trackSheetOpen());
  });
  const phone = () => !layout().desktop();
  // Full-height sheets show a ✕ left of the title on the phone (§2.7); others none.
  const closeLeft = () => !props.classicHead && phone() && size() === 'full' && !props.headerAction;
  const closeRight = () => !props.headerAction && (props.classicHead || !phone());
  const hasHead = () => props.title !== false || closeRight() || closeLeft() || !!props.headerAction;
  return (
    <Drawer
      open={props.open}
      onOpenChange={(open) => {
        if (!open) props.onClose();
      }}
      side="bottom"
      snapPoints={snaps()}
      activeSnapPoint={snap()}
      onActiveSnapPointChange={(p) => {
        const next = typeof p === 'number' ? p : 1;
        if (next !== snap() && next !== 0) haptic('selection');
        setSnap(next);
      }}
      restoreFocus={false}
      onInitialFocus={(event: Event) => {
        const content = event.currentTarget as HTMLElement | null;
        const field = content?.querySelector<HTMLElement>('[autofocus]');
        event.preventDefault();
        if (!field) {
          content?.setAttribute('tabindex', '-1');
          content?.focus({ preventScroll: true });
          return;
        }
        field.focus({ preventScroll: true });
        if (field instanceof HTMLInputElement && field.type === 'text') field.select();
      }}
      closeOnEscapeKeyDown={false}
      closeOnOutsideFocus={false}
      onOutsidePointer={(event: PointerEvent) => {
        // Picking an option in a portaled popover is not a scrim tap.
        if (inFloatingLayer(event.target)) event.preventDefault();
      }}
    >
      <Drawer.Portal>
        <Drawer.Overlay class="sheet-overlay ph-scrim" data-level={props.level === 2 ? '2' : undefined} />
        <Drawer.Content
          class={`sheet board-sheet ph-sheet ${props.class ?? ''}`}
          data-size={size()}
          data-level={props.level === 2 ? '2' : undefined}
          data-testid={props.testId}
          data-corvu-no-drag={props.noDrag ? '' : undefined}
          onKeyDown={(event: KeyboardEvent) => {
            if (wrapTab(event, event.currentTarget as HTMLElement)) event.stopPropagation();
          }}
        >
          <Show when={hasHead()} fallback={<Drawer.Label class="sr-only">{props.label}</Drawer.Label>}>
            <div class="board-sheet-head ph-sheet-head" data-close-left={closeLeft() ? '' : undefined}>
              <Show when={closeLeft()}>
                <button type="button" class="ph-icon-btn ph-sheet-close-left" aria-label="Close" onClick={() => props.onClose()}>
                  <X size={24} />
                </button>
              </Show>
              <Drawer.Label class="board-sheet-title ph-sheet-title" classList={{ 'sr-only': props.title === false }}>
                {props.title || props.label}
              </Drawer.Label>
              <Show
                when={props.headerAction}
                fallback={
                  <Show when={closeRight()}>
                    <button type="button" class="ghost-icon-button board-sheet-close" aria-label="Close" onClick={() => props.onClose()}>
                      <X size={20} />
                    </button>
                  </Show>
                }
              >
                {props.headerAction}
              </Show>
            </div>
          </Show>
          <div class="board-sheet-body ph-sheet-body">{props.children}</div>
          <Show when={props.footer}>
            <div class="board-sheet-foot ph-sheet-foot">{props.footer}</div>
          </Show>
        </Drawer.Content>
      </Drawer.Portal>
    </Drawer>
  );
}

export function Sheet(p: {
  open: boolean;
  onClose: () => void;
  label: string;
  title?: JSX.Element | false;
  headerAction?: JSX.Element;
  detents?: Detent[];
  initialDetent?: Detent;
  footer?: JSX.Element;
  guarded?: boolean | (() => boolean);
  class?: string;
  testId?: string;
  children: JSX.Element;
}) {
  const dirty = () => (typeof p.guarded === 'function' ? p.guarded() : !!p.guarded);
  let asking = false;
  const request = async () => {
    if (!dirty()) return p.onClose();
    if (asking) return;
    asking = true;
    const ok = await confirmSheet({ title: 'Discard changes?', body: 'What you typed here will be lost.', confirm: 'Discard', tone: 'danger' });
    asking = false;
    if (ok) p.onClose();
  };
  return (
    <SheetFrame
      open={p.open}
      onClose={() => void request()}
      label={p.label}
      title={p.title}
      headerAction={p.headerAction}
      detents={p.detents}
      initialDetent={p.initialDetent}
      footer={p.footer}
      noDrag={dirty()}
      class={p.class}
      testId={p.testId}
    >
      {p.children}
    </SheetFrame>
  );
}

// ── Action sheet ─────────────────────────────────────────────────────────

export type ActionItem = {
  id: string;
  label: string;
  icon?: Component<{ size?: number }>;
  description?: string;
  value?: string;
  checked?: boolean;
  tone?: 'danger';
  disabled?: boolean;
  /** Ask first: a second sheet with this text as its title. */
  confirm?: string;
  onSelect: () => void;
};

export function ActionRow(p: { item: ActionItem; onPick: (item: ActionItem) => void }) {
  return (
    <button
      type="button"
      role="menuitem"
      class="ph-action-row"
      data-tone={p.item.tone}
      data-two={p.item.description ? '' : undefined}
      disabled={p.item.disabled}
      data-testid={`action-${p.item.id}`}
      onClick={() => p.onPick(p.item)}
    >
      <Show when={p.item.icon}>
        <span class="ph-action-icon">
          <Dynamic component={p.item.icon} size={22} />
        </span>
      </Show>
      <span class="ph-action-text">
        <span class="ph-action-label">{p.item.label}</span>
        <Show when={p.item.description}>
          <span class="ph-action-desc">{p.item.description}</span>
        </Show>
      </span>
      <Show when={p.item.value}>
        <span class="ph-action-value">{p.item.value}</span>
      </Show>
      <Show when={p.item.checked}>
        <Check size={20} class="ph-action-check" aria-label="Selected" />
      </Show>
    </button>
  );
}

export function ActionSheet(p: {
  open: boolean;
  onClose: () => void;
  title?: string;
  subtitle?: string;
  sections: ActionItem[][];
  testId?: string;
}) {
  const pick = async (item: ActionItem) => {
    if (item.disabled) return;
    if (item.confirm) {
      p.onClose();
      haptic('warning');
      const ok = await confirmSheet({ title: item.confirm, confirm: item.label, tone: item.tone });
      if (ok) item.onSelect();
      return;
    }
    p.onClose();
    item.onSelect();
  };
  const title = () =>
    p.title ? (
      <span class="ph-action-head">
        <span class="ph-action-title">{p.title}</span>
        <Show when={p.subtitle}>
          <span class="ph-action-subtitle">{p.subtitle}</span>
        </Show>
      </span>
    ) : (
      false
    );
  return (
    <SheetFrame open={p.open} onClose={p.onClose} label={p.title ?? 'Actions'} title={title()} class="ph-action-sheet" testId={p.testId ?? 'action-sheet'}>
      <div role="menu" aria-label={p.title ?? 'Actions'} class="ph-action-sections">
        <For each={p.sections.filter((s) => s.length > 0)}>
          {(section) => (
            <div class="ph-action-section" role="group">
              <For each={section}>{(item) => <ActionRow item={item} onPick={(i) => void pick(i)} />}</For>
            </div>
          )}
        </For>
      </div>
    </SheetFrame>
  );
}

// ── confirmSheet ─────────────────────────────────────────────────────────

/**
 * Imperative confirmation (§2.8 destructive rows, guarded sheets): a small
 * sheet over whatever is open (`--z-sheet-2`). Resolves true on confirm.
 */
export function confirmSheet(o: { title: string; body?: string; confirm: string; tone?: 'danger' }): Promise<boolean> {
  if (typeof document === 'undefined') return Promise.resolve(false);
  return new Promise((resolve) => {
    const host = document.createElement('div');
    host.className = 'ph-confirm-host';
    document.body.appendChild(host);
    let settled = false;
    let dispose: (() => void) | undefined;
    dispose = render(() => {
      const [open, setOpen] = createSignal(true);
      const done = (ok: boolean) => {
        if (settled) return;
        settled = true;
        setOpen(false);
        resolve(ok);
        // Let the close transition run, then unmount.
        setTimeout(() => {
          dispose?.();
          host.remove();
        }, 260);
      };
      return (
        <SheetFrame
          open={open()}
          onClose={() => done(false)}
          label={o.title}
          title={o.title}
          level={2}
          class="ph-confirm"
          testId="confirm-sheet"
          footer={
            <div class="ph-confirm-actions">
              <button type="button" class="ph-btn" data-kind="secondary" data-block="" data-testid="confirm-cancel" onClick={() => done(false)}>
                Cancel
              </button>
              <button
                type="button"
                class="ph-btn"
                data-kind={o.tone === 'danger' ? 'danger' : 'primary'}
                data-block=""
                data-testid="confirm-ok"
                onClick={() => done(true)}
              >
                {o.confirm}
              </button>
            </div>
          }
        >
          <Show when={o.body}>
            <p class="ph-confirm-body">{o.body}</p>
          </Show>
        </SheetFrame>
      );
    }, host);
  });
}
