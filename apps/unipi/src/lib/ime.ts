/**
 * Android IME input: a value-diffing model (DESIGN.md §3.3).
 *
 * Android soft keyboards (GBoard, HyperOS, `adb shell input text`)
 * each raise a different, inconsistent mix of `keydown` /
 * `beforeinput` / `compositionupdate` events for the same keystroke —
 * composing-word replacements, autocorrect rewrites and multi-char
 * commits all land differently per IME. Trying to special-case each
 * event type (the previous approach) is exactly what caused the
 * doubling bug: more than one event for the same keystroke tried to
 * deliver bytes.
 *
 * Instead there is exactly one source of truth: the hidden
 * `#term-ime` textarea's **current value**. On every native `input`
 * event we diff that value against the last value we already sent
 * bytes for — common prefix kept, diverging suffix of the old value
 * erased with backspaces (`\x7f`), diverging suffix of the new value
 * typed. This is robust to every IME because it never interprets
 * event semantics, only the resulting text.
 *
 * Non-printable keys (Enter, arrows, Esc, Tab, Ctrl combos) do not
 * reach the textarea's value at all under normal typing — they are
 * read on `keydown` and bypass this diff (see `lib/termToolbar`'s
 * `imeKeyBytes`).
 */

/** Maximum textarea length before it is silently reset (keeps the
 * diff cheap and avoids unbounded growth during a long paste). */
const MAX_TRACKED_LENGTH = 200;

export type ImeState = {
  /** The textarea value bytes were last computed against. */
  lastValue: string;
};

export function imeInitial(): ImeState {
  return { lastValue: '' };
}

export type ImeDiffResult = {
  /** Bytes to send to the PTY for this diff. */
  bytes: string;
  /** Next state. */
  state: ImeState;
  /** Set when the textarea must be reset (value grew too long). */
  resetValue?: string;
};

/** Longest common prefix length of two strings. */
function commonPrefixLength(a: string, b: string): number {
  const max = Math.min(a.length, b.length);
  let i = 0;
  while (i < max && a[i] === b[i]) i += 1;
  return i;
}

/**
 * Diffs the textarea's new value against the last value we sent
 * bytes for: erases the divergent tail of the old value with
 * backspaces, then types the divergent tail of the new value. Pure;
 * never reads the DOM.
 */
export function diffImeValue(nextValue: string, state: ImeState): ImeDiffResult {
  const prefix = commonPrefixLength(state.lastValue, nextValue);
  const deletions = '\x7f'.repeat(state.lastValue.length - prefix);
  const insertions = nextValue.slice(prefix);
  const bytes = deletions + insertions;

  if (nextValue.length > MAX_TRACKED_LENGTH) {
    return { bytes, state: { lastValue: '' }, resetValue: '' };
  }
  return { bytes, state: { lastValue: nextValue } };
}
