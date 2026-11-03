/**
 * The phone composer's field (phone spec §3.5, PH-11): a native `<textarea>`
 * instead of the contenteditable `ChipEditor`. Android keyboards (Gboard's
 * composition, `adb input text`) edit a textarea natively, so the text the user
 * typed is exactly the text sent — no DOM rebuilds under an open composition.
 *
 * Same contract as `ChipEditor` (`ChipEditorApi`, `onChange(text, caret, kind)`),
 * so the composer's menus, history and drafts work unchanged. Quotes / pasted
 * text are the chip row above it. The value is written back into the element
 * only when the draft changed from outside (a picked command, a cleared send, a
 * history walk) and never while a composition is open. Enter inserts a newline
 * (Send sends); autogrow 1 line → 6 lines / 40 % of the visible height, then it
 * scrolls.
 *
 * Inline chips (owner spec 2026-11-03): the textarea can't hold elements, so a
 * draft with chips draws them in a mirror layer under the (then transparent)
 * text — same font, same wrapping, scrolled with it: each chip token keeps its
 * own characters' width, painted as a pill (`/command`, `$skill`, `@path` show
 * their text; an image shows its thumbnail + "image N", a file its name). The
 * textarea stays the only input, so Gboard composes exactly as before. Chips
 * stay atomic: Backspace/Delete next to one (or over the space after it)
 * removes the whole token, a caret landing inside one moves to its end, and an
 * edit that still cut into one (an IME that ignored the cancel) takes the whole
 * token out — all only outside a composition.
 */
import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import type { ChipEditorApi } from './ChipEditor';
import type { EditKind } from '../../lib/composer/undo';
import { chipSpans, removeToken, tokenize, type Chip, type TokenizeOpts } from '../../lib/composer/chips';

/** Where an edit of `prev` into `next` cut into a chip token without removing all of it, the edit
 *  widened to take the whole token (with one neighbouring space). `undefined` when nothing broke. */
export function repairBrokenChips(prev: string, next: string, opts: TokenizeOpts): { text: string; caret: number } | undefined {
  if (prev === next) return undefined;
  let p = 0;
  const max = Math.min(prev.length, next.length);
  while (p < max && prev[p] === next[p]) p++;
  let s = 0;
  while (s < max - p && prev[prev.length - 1 - s] === next[next.length - 1 - s]) s++;
  const delStart = p;
  const delEnd = prev.length - s;
  const inserted = next.slice(p, next.length - s);
  let from = delStart;
  let to = delEnd;
  let broke = false;
  for (const { chip, start } of chipSpans(tokenize(prev, opts))) {
    const end = start + chip.raw.length;
    const overlaps = delEnd > start && delStart < end;
    // An insertion strictly inside a token splits it too.
    const splits = delStart === delEnd && delStart > start && delStart < end;
    // Only the space a finished chip keeps after itself went (Gboard's backspace arrives as a
    // non-cancelable deleteSurroundingText): the chip would un-chip into raw text — take it whole.
    if (!inserted && delStart === end && delEnd === end + 1 && /\s/.test(prev[end]!)) {
      return removeToken(prev, start, chip.raw);
    }
    if (!(overlaps || splits)) continue;
    if (delStart <= start && delEnd >= end) continue; // removed whole: fine
    broke = true;
    from = Math.min(from, start);
    to = Math.max(to, end);
  }
  if (!broke) return undefined;
  if (prev[to] === ' ' && !inserted) to++;
  const text = prev.slice(0, from) + inserted + prev.slice(to);
  return { text, caret: from + inserted.length };
}

export function PhoneField(props: {
  value: string;
  onChange: (text: string, caret: number, kind: EditKind) => void;
  onCaret?: (caret: number) => void;
  onKeyDown?: (e: KeyboardEvent) => void;
  onHistory?: (op: 'undo' | 'redo') => void;
  /** First look at a pasted text — true when it became a chip (a long paste). */
  onPasteText?: (text: string) => boolean;
  /** Images / files on the clipboard (a keyboard's image paste): attach them as chips. */
  onPasteFiles?: (files: File[]) => void;
  /** Which tokens are chips (the composer's `tokenize` options). */
  opts?: () => TokenizeOpts;
  /** A chip's label the token can't name ("image N"). */
  chipLabel?: (chip: Chip) => string | undefined;
  chipThumb?: (chip: Chip) => string | undefined;
  /** A chip token removed as a whole. */
  onChipRemoved?: (chip: Chip) => void;
  disabled?: boolean;
  placeholder?: string;
  class?: string;
  api?: (api: ChipEditorApi) => void;
  ariaLabel?: string;
}) {
  let el!: HTMLTextAreaElement;
  let composing = false;
  let kind: EditKind = 'other';
  /** The text the field showed before the current input event (the repair's baseline). */
  let before = props.value;
  /** The text when the current composition started (the repair's baseline at its end). */
  let composeFrom = props.value;

  const opts = () => props.opts?.() ?? {};
  const segs = createMemo(() => tokenize(props.value, opts()));
  const hasChips = createMemo(() => segs().some((s) => 'chip' in s));
  const [scroll, setScroll] = createSignal(0);

  const grow = () => {
    if (!el) return;
    el.style.height = 'auto';
    // The CSS max-height (6 lines / 40 % of the visible height) caps it; past that it scrolls.
    el.style.height = `${el.scrollHeight}px`;
    setScroll(el.scrollTop);
  };
  const caret = () => el?.selectionStart ?? props.value.length;
  const setValue = (text: string, at: number) => {
    el.value = text;
    el.setSelectionRange(at, at);
    before = text;
    grow();
  };

  props.api?.({
    focus: () => el?.focus({ preventScroll: true }),
    setCaret: (offset) => {
      if (!el) return;
      if (el.value !== props.value) {
        // A pick lands before its caret move: show the new text first (the effect may not have run yet).
        if (composing) endComposition();
        el.value = props.value;
        before = props.value;
        grow();
      }
      const at = Math.max(0, Math.min(offset, el.value.length));
      el.setSelectionRange(at, at);
    },
    getCaret: caret,
    syncFromStore: () => {
      if (!el || el.value === props.value) return;
      if (composing) endComposition();
      el.value = props.value;
      before = props.value;
      grow();
    },
  });

  /** A picked suggestion replaced the word Gboard is still composing ("/gra" → "/graph "): the
   *  composition must end, or its compositionend writes the old word back over the pick. Blur +
   *  focus makes the keyboard restart its input on the new text. */
  const endComposition = () => {
    composing = false;
    if (el !== document.activeElement) return;
    el.blur();
    el.focus({ preventScroll: true });
  };

  // The draft changed outside an edit (a picked command, Send cleared it): show it.
  createEffect(() => {
    const value = props.value;
    if (!el || el.value === value) return;
    const wasComposing = composing;
    if (wasComposing) endComposition();
    const at = el === document.activeElement ? Math.min(el.selectionStart ?? value.length, value.length) : value.length;
    el.value = value;
    before = value;
    if (el === document.activeElement) el.setSelectionRange(at, at);
    grow();
  });

  /** The chip whose token ends (`back`) or starts at `at`. */
  const chipAt = (text: string, at: number, back: boolean) => {
    for (const span of chipSpans(tokenize(text, opts()))) {
      if (back ? span.start + span.chip.raw.length === at : span.start === at) return span;
    }
    return undefined;
  };
  /** A collapsed caret inside a chip token moves to its end (never while composing). */
  const snapCaret = () => {
    if (!el || composing || el.selectionStart !== el.selectionEnd) return;
    const at = el.selectionStart;
    for (const { chip, start } of chipSpans(segs())) {
      const end = start + chip.raw.length;
      if (at > start && at < end) {
        el.setSelectionRange(end, end);
        return;
      }
    }
  };

  onMount(() => {
    el.value = props.value;
    before = props.value;
    grow();
    const onSel = () => {
      if (document.activeElement !== el) return;
      if (hasChips()) snapCaret();
      props.onCaret?.(caret());
    };
    document.addEventListener('selectionchange', onSel);
    onCleanup(() => document.removeEventListener('selectionchange', onSel));
  });

  const labelOf = (chip: Chip): string =>
    props.chipLabel?.(chip) ?? (chip.kind === 'file' ? chip.name : chip.kind === 'path' ? chip.path.replace(/\/$/, '').split('/').pop() || chip.path : chip.raw);

  return (
    <div class="ph-field-box" data-chips={hasChips() ? '' : undefined}>
      <Show when={hasChips()}>
        <div class="ph-field-mirror" aria-hidden="true" data-testid="phone-field-mirror">
          <div class="ph-field-mirror-in" style={{ transform: `translateY(${-scroll()}px)` }}>
            <For each={segs()}>
              {(seg) => {
                if (!('chip' in seg)) return <>{seg.text}</>;
                const chip = seg.chip;
                const covered = chip.kind === 'image' || chip.kind === 'file';
                return (
                  <span class="phf-chip" data-kind={chip.kind} data-mode={chip.kind === 'command' && chip.mode ? '' : undefined} data-covered={covered ? '' : undefined}>
                    {chip.raw}
                    <Show when={covered}>
                      <span class="phf-chip-label">
                        <Show when={props.chipThumb?.(chip)}>{(src) => <img class="phf-chip-thumb" src={src()} alt="" />}</Show>
                        <span class="phf-chip-text">{labelOf(chip)}</span>
                      </span>
                    </Show>
                  </span>
                );
              }}
            </For>
            {/* A trailing newline needs a character after it to take its line. */}
            {props.value.endsWith('\n') ? '\u200b' : ''}
          </div>
        </div>
      </Show>
      <textarea
        ref={el}
        class={`ph-field ${props.class ?? ''}`}
        rows={1}
        aria-label={props.ariaLabel}
        placeholder={props.placeholder}
        disabled={props.disabled}
        autocapitalize="sentences"
        autocomplete="off"
        spellcheck
        enterkeyhint="enter"
        data-testid="phone-field"
        onScroll={() => setScroll(el.scrollTop)}
        onBeforeInput={(e) => {
          const t = (e as InputEvent).inputType ?? '';
          kind = t === 'insertText' || t === 'insertCompositionText' ? 'type' : t.startsWith('delete') ? 'delete' : 'other';
          if (t === 'historyUndo' || t === 'historyRedo') {
            if (props.onHistory) {
              e.preventDefault();
              props.onHistory(t === 'historyUndo' ? 'undo' : 'redo');
            }
            return;
          }
          // Atomic chips: Backspace / Delete next to one takes the whole token.
          if ((t === 'deleteContentBackward' || t === 'deleteContentForward') && !composing && !(e as InputEvent).isComposing && el.selectionStart === el.selectionEnd && hasChips()) {
            const text = el.value;
            const at = el.selectionStart;
            const back = t === 'deleteContentBackward';
            // The space a chip keeps after itself goes with it (else Gboard would re-compose the raw token).
            const hit = chipAt(text, at, back) ?? (back && at > 0 && text[at - 1] === ' ' ? chipAt(text, at - 1, true) : undefined);
            if (!hit) return;
            e.preventDefault();
            const next = removeToken(text, hit.start, hit.chip.raw);
            setValue(next.text, next.caret);
            props.onChange(next.text, next.caret, 'delete');
            props.onChipRemoved?.(hit.chip);
          }
        }}
        onInput={() => {
          // An edit that still cut into a chip (the keyboard ignored the cancel) takes the whole token.
          if (!composing && hasChips()) {
            const fixed = repairBrokenChips(before, el.value, opts());
            if (fixed) {
              setValue(fixed.text, fixed.caret);
              props.onChange(fixed.text, fixed.caret, 'delete');
              kind = 'other';
              return;
            }
          }
          before = el.value;
          props.onChange(el.value, caret(), kind);
          kind = 'other';
          grow();
        }}
        onCompositionStart={() => {
          composing = true;
          composeFrom = before;
        }}
        onCompositionEnd={() => {
          if (!composing) return; // ended by a pick (endComposition): the field already shows the draft
          composing = false;
          // Gboard re-composes the word next to the caret, so a Backspace into a chip arrives inside a
          // composition (never rewritten under it): once it ends, take a chip it cut into out whole.
          const fixed = hasChips() || tokenize(composeFrom, opts()).some((x) => 'chip' in x) ? repairBrokenChips(composeFrom, el.value, opts()) : undefined;
          if (fixed) {
            setValue(fixed.text, fixed.caret);
            props.onChange(fixed.text, fixed.caret, 'delete');
            return;
          }
          before = el.value;
          // The final composed text: make sure the draft has exactly what the field shows.
          if (el.value !== props.value) props.onChange(el.value, caret(), 'type');
        }}
        onKeyDown={(e) => props.onKeyDown?.(e)}
        onKeyUp={() => props.onCaret?.(caret())}
        onClick={() => {
          if (hasChips()) snapCaret();
          props.onCaret?.(caret());
        }}
        onPaste={(e) => {
          const files = [...(e.clipboardData?.files ?? [])];
          if (files.length && props.onPasteFiles) {
            e.preventDefault();
            props.onPasteFiles(files);
            return;
          }
          const text = e.clipboardData?.getData('text/plain') ?? '';
          if (text && props.onPasteText?.(text)) e.preventDefault();
        }}
      />
    </div>
  );
}
