/**
 * Keyboard navigation for the composer's toolbar menus (D-05: permission,
 * thinking, the draft's model list). The menus are `AnchoredMenu`s whose
 * rows are buttons with a `menuitem*` role; this wires the WAI-ARIA menu
 * keys onto them without owning any state:
 *
 * - on open, focus moves to the checked row (else the first);
 * - ↑ / ↓ move (wrapping), Home / End jump, a printable key jumps to the
 *   next row whose label starts with what was typed (typeahead, 600 ms);
 * - Enter / Space activate (native button behaviour);
 * - Esc closes (the menu's own Back handler) and Tab closes too — both hand
 *   focus back to the chip that opened the menu.
 *
 * A search field inside the menu (the draft's model list) keeps its typing:
 * ↓ from it enters the rows, ↑ on the first row goes back to it.
 */

const ROW = '[role^="menuitem"]:not([disabled])';

export type MenuKeysOptions = {
  /** Closes the menu (Tab, or Esc when the menu has no Back handler of its own). */
  close: () => void;
  /** The control that opened the menu: focus goes back to it on close. */
  trigger?: () => HTMLElement | undefined;
  /** Focus the checked row on open (default true). False: the caller focuses (a search field). */
  autoFocus?: boolean;
};

/** The text a row is found by (its `data-label`, else its first line). */
function rowLabel(row: HTMLElement): string {
  return (row.dataset.label ?? row.textContent ?? '').trim().toLowerCase();
}

/**
 * Attach to the element holding the rows (a `ref`). Returns a disposer; also
 * restores focus to the trigger when the element leaves the DOM.
 */
export function menuKeys(el: HTMLElement, opts: MenuKeysOptions): () => void {
  const rows = () => [...el.querySelectorAll<HTMLElement>(ROW)];
  const search = () => el.querySelector<HTMLInputElement>('input[type="search"], input[type="text"]');
  let typed = '';
  let typedAt = 0;

  const focusRow = (row: HTMLElement | undefined) => {
    if (!row) return;
    row.focus({ preventScroll: true });
    row.scrollIntoView?.({ block: 'nearest' });
  };

  const onKey = (e: KeyboardEvent) => {
    if (e.defaultPrevented || e.isComposing || e.altKey || e.ctrlKey || e.metaKey) return;
    const list = rows();
    const active = document.activeElement as HTMLElement | null;
    const at = active ? list.indexOf(active) : -1;
    // Any text field in the menu keeps its own typing and caret keys (search, an id field).
    const inSearch = !!active && (active === search() || active.matches('input, textarea'));
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        focusRow(list[at < 0 || at + 1 >= list.length ? 0 : at + 1]);
        return;
      case 'ArrowUp':
        e.preventDefault();
        if (at === 0 && search()) {
          search()!.focus();
          return;
        }
        focusRow(list[at <= 0 ? list.length - 1 : at - 1]);
        return;
      case 'Home':
        if (inSearch) return;
        e.preventDefault();
        focusRow(list[0]);
        return;
      case 'End':
        if (inSearch) return;
        e.preventDefault();
        focusRow(list[list.length - 1]);
        return;
      case 'Tab':
        e.preventDefault();
        opts.close();
        return;
      default:
        break;
    }
    if (inSearch || e.key.length !== 1 || e.key === ' ') return;
    // A field in the menu takes the typing (the row list follows its filter).
    const field = search();
    if (field) {
      field.focus();
      return;
    }
    const now = Date.now();
    typed = now - typedAt > 600 ? e.key.toLowerCase() : typed + e.key.toLowerCase();
    typedAt = now;
    const start = at < 0 ? 0 : typed.length === 1 ? at + 1 : at;
    const order = [...list.slice(start), ...list.slice(0, start)];
    const hit = order.find((row) => rowLabel(row).startsWith(typed));
    if (hit) {
      e.preventDefault();
      focusRow(hit);
    }
  };

  el.addEventListener('keydown', onKey);
  const trigger = opts.trigger?.();
  if (opts.autoFocus !== false) {
    queueMicrotask(() => {
      const list = rows();
      focusRow(list.find((row) => row.getAttribute('aria-checked') === 'true') ?? list[0]);
    });
  }
  return () => {
    el.removeEventListener('keydown', onKey);
    // Focus was inside the menu (or dropped to <body> as it closed): back to the chip.
    const active = document.activeElement;
    if (trigger?.isConnected && (!active || active === document.body || el.contains(active))) trigger.focus({ preventScroll: true });
  };
}
