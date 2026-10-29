import { describe, expect, it } from 'vitest';
import { diffImeValue, imeInitial, type ImeState } from './ime';

/** Replays a sequence of textarea values (as observed on `input`
 * events) through the diff model and returns the concatenated bytes
 * sent to the PTY plus the final state. */
function replay(values: string[]): { bytes: string[]; total: string; state: ImeState } {
  let state = imeInitial();
  const bytes: string[] = [];
  for (const value of values) {
    const result = diffImeValue(value, state);
    state = result.state;
    bytes.push(result.bytes);
  }
  return { bytes, total: bytes.join(''), state };
}

describe('diffImeValue — value-diffing IME model', () => {
  it('sends each new character once as the textarea grows letter by letter', () => {
    // Captured via CDP: `adb shell input text hello` on a focused
    // #term-ime with a real inputmode fires one `input` event per
    // character, each with the textarea's cumulative value.
    const { bytes, total } = replay(['h', 'he', 'hel', 'hell', 'hello']);
    expect(bytes).toEqual(['h', 'e', 'l', 'l', 'o']);
    expect(total).toBe('hello');
  });

  it('a GBoard word-level replacement (compositionupdate collapsed into input) diffs correctly', () => {
    // GBoard can replace the whole composing word in one `input`
    // event instead of appending letter by letter.
    const { total } = replay(['t', 'teh', 'the']);
    // "t" -> "teh": common prefix "t", insert "eh".
    // "teh" -> "the": common prefix "t", erase "eh" (2 backspaces), insert "he".
    expect(total).toBe('t' + 'eh' + '\x7f\x7f' + 'he');
  });

  it('backspace shrinks the value and sends exactly one \\x7f per removed char', () => {
    const { bytes, total } = replay(['abc', 'ab']);
    expect(bytes).toEqual(['abc', '\x7f']);
    expect(total).toBe('abc\x7f');
  });

  it('abc, one backspace via keyevent 67, then d -> sends abd with one backspace', () => {
    const { total } = replay(['abc', 'ab', 'abd']);
    expect(total).toBe('abc' + '\x7f' + 'd');
  });

  it('never double-sends when the same value fires input twice (duplicate event)', () => {
    const { bytes, total } = replay(['hello', 'hello']);
    expect(bytes).toEqual(['hello', '']);
    expect(total).toBe('hello');
  });

  it('clearing the textarea sends one backspace per remaining character', () => {
    const { bytes } = replay(['abc', '']);
    expect(bytes[1]).toBe('\x7f\x7f\x7f');
  });

  it('UNI-313 regression: compositionend followed by a duplicate `input` with the same value sends the composed text once, not twice', () => {
    // Captured shape of a composition + input sequence: compositionupdate
    // fires `input` with the in-progress text, compositionend commits it
    // (another `input`, same final value — some engines fire the commit
    // `input` before `compositionend`, some after, but either way the
    // textarea's value does not change a second time) and the diff model
    // is keyed on that value, not on how many events carried it.
    const { bytes, total } = replay(['t', 'te', 'tes', 'test', 'test']);
    expect(bytes).toEqual(['t', 'e', 's', 't', '']);
    expect(total).toBe('test');
  });

  it('UNI-313: a stray trailing space from an autocomplete/inline-completion commit after composition is sent once, as a single appended character', () => {
    // WebKitGTK/IME inline-completion root cause: the composition commits
    // "test", then the webview appends a completion space to the same
    // textarea in a *separate* `input` event. The diff model still sees
    // one new character at the end and sends exactly one space — the bug
    // this guards against was a second, independent send path (e.g. a
    // `compositionend` listener) re-sending the whole word on top of this.
    const { bytes, total } = replay(['t', 'te', 'tes', 'test', 'test ']);
    expect(bytes).toEqual(['t', 'e', 's', 't', ' ']);
    expect(total).toBe('test ');
  });

  it('resets the tracked value once the textarea grows too long (e.g. a long paste)', () => {
    const long = 'x'.repeat(250);
    const result = diffImeValue(long, imeInitial());
    expect(result.resetValue).toBe('');
    expect(result.state.lastValue).toBe('');
    expect(result.bytes).toBe(long);
  });
});
