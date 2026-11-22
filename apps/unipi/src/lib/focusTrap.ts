/**
 * Tab / Shift+Tab wrap inside a dialog (UNI-117 QA). corvu's trap wraps from
 * its *first/last focusable in DOM order* — but that list includes disabled
 * buttons and hidden inputs (a disabled "Create task", a hidden file input),
 * which can't take focus, so Tab walked straight out of the dialog into the
 * page behind. This one only counts elements that can really be focused.
 */
const CANDIDATES =
  'a[href], button, input, textarea, select, details > summary, [tabindex], [contenteditable=""], [contenteditable="true"]';

export function tabbables(root: ParentNode): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(CANDIDATES)].filter((el) => {
    if (el.tabIndex < 0) return false;
    if ((el as HTMLButtonElement).disabled) return false;
    if (el.hidden || el.closest('[hidden], [inert]')) return false;
    if (el instanceof HTMLInputElement && el.type === 'hidden') return false;
    const style = typeof getComputedStyle === 'function' ? getComputedStyle(el) : undefined;
    if (style && (style.display === 'none' || style.visibility === 'hidden')) return false;
    return true;
  });
}

/** Keydown handler for a dialog element: wraps Tab at its ends. Returns true when it moved focus. */
export function wrapTab(event: KeyboardEvent, root: HTMLElement): boolean {
  if (event.key !== 'Tab' || event.altKey || event.ctrlKey || event.metaKey) return false;
  const list = tabbables(root);
  if (list.length === 0) {
    event.preventDefault();
    return true;
  }
  const first = list[0]!;
  const last = list[list.length - 1]!;
  const active = document.activeElement as HTMLElement | null;
  const inside = !!active && root.contains(active);
  if (event.shiftKey && (!inside || active === first)) {
    event.preventDefault();
    last.focus();
    return true;
  }
  if (!event.shiftKey && (!inside || active === last)) {
    event.preventDefault();
    first.focus();
    return true;
  }
  return false;
}
