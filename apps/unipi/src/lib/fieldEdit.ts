/**
 * Programmatic edits to a textarea/input that keep the browser's own undo
 * stack (UNI-270). Assigning `el.value` wipes it — after inserting an
 * `@task` mention or a **bold** from the toolbar, Ctrl/⌘+Z did nothing.
 * `execCommand('insertText')` edits like typing does: one undo step, an
 * `input` event, the caret after the insert. Where it's missing (jsdom, old
 * engines), refused, or the field isn't focused, the value is set directly
 * as before.
 */

/** The smallest [start, end) of `from` to replace with `insert` to get `to`. */
export function diffRange(from: string, to: string): { start: number; end: number; insert: string } {
  let p = 0;
  const max = Math.min(from.length, to.length);
  while (p < max && from[p] === to[p]) p++;
  let s = 0;
  while (s < max - p && from[from.length - 1 - s] === to[to.length - 1 - s]) s++;
  return { start: p, end: from.length - s, insert: to.slice(p, to.length - s) };
}

/**
 * Makes the field's value `next` as one undoable edit, then selects
 * [selStart, selEnd] (default: right after the change). Returns true when
 * the browser applied it natively (an `input` event fired).
 */
export function setFieldValue(el: HTMLTextAreaElement | HTMLInputElement, next: string, selStart?: number, selEnd = selStart): boolean {
  const cur = el.value;
  if (cur === next) {
    if (selStart !== undefined) el.setSelectionRange(selStart, selEnd ?? selStart);
    return false;
  }
  const { start, end, insert } = diffRange(cur, next);
  let native = false;
  // Only in the focused field: execCommand edits the focused one, and focusing it here would pop a
  // phone's keyboard (an upload finishing after the user moved on).
  if (typeof document.execCommand === 'function' && document.activeElement === el) {
    el.setSelectionRange(start, end);
    try {
      native = insert ? document.execCommand('insertText', false, insert) : document.execCommand('delete', false);
    } catch {
      native = false;
    }
    native = native && el.value === next;
  }
  if (!native) el.value = next;
  const at = selStart ?? start + insert.length;
  el.setSelectionRange(at, selEnd ?? at);
  return native;
}
