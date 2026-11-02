/**
 * The phone composer's field (phone spec §3.5, PH-11): a native `<textarea>`
 * instead of the contenteditable `ChipEditor`. Android keyboards (Gboard's
 * composition, `adb input text`) edit a textarea natively, so the text the user
 * typed is exactly the text sent — no DOM rebuilds under an open composition.
 *
 * Same contract as `ChipEditor` (`ChipEditorApi`, `onChange(text, caret, kind)`),
 * so the composer's menus, history and drafts work unchanged. Inline references
 * stay plain text in the field; quotes / pasted text / attachments are the chip
 * row above it. The value is written back into the element only when the draft
 * changed from outside (a picked command, a cleared send, a history walk) and
 * never while a composition is open. Enter inserts a newline (Send sends);
 * autogrow 1 line → 6 lines / 40 % of the visible height, then it scrolls.
 */
import { createEffect, onCleanup, onMount } from 'solid-js';
import type { ChipEditorApi } from './ChipEditor';
import type { EditKind } from '../../lib/composer/undo';

export function PhoneField(props: {
  value: string;
  onChange: (text: string, caret: number, kind: EditKind) => void;
  onCaret?: (caret: number) => void;
  onKeyDown?: (e: KeyboardEvent) => void;
  onHistory?: (op: 'undo' | 'redo') => void;
  /** First look at a pasted text — true when it became a chip (a long paste). */
  onPasteText?: (text: string) => boolean;
  disabled?: boolean;
  placeholder?: string;
  class?: string;
  api?: (api: ChipEditorApi) => void;
  ariaLabel?: string;
}) {
  let el!: HTMLTextAreaElement;
  let composing = false;
  let kind: EditKind = 'other';

  const grow = () => {
    if (!el) return;
    el.style.height = 'auto';
    // The CSS max-height (6 lines / 40 % of the visible height) caps it; past that it scrolls.
    el.style.height = `${el.scrollHeight}px`;
  };
  const caret = () => el?.selectionStart ?? props.value.length;

  props.api?.({
    focus: () => el?.focus({ preventScroll: true }),
    setCaret: (offset) => {
      if (!el) return;
      const at = Math.max(0, Math.min(offset, el.value.length));
      el.setSelectionRange(at, at);
    },
    getCaret: caret,
    syncFromStore: () => {
      if (!el || composing || el.value === props.value) return;
      el.value = props.value;
      grow();
    },
  });

  // The draft changed outside an edit (a picked command, Send cleared it): show it.
  createEffect(() => {
    const value = props.value;
    if (!el || composing || el.value === value) return;
    const at = el === document.activeElement ? Math.min(el.selectionStart ?? value.length, value.length) : value.length;
    el.value = value;
    if (el === document.activeElement) el.setSelectionRange(at, at);
    grow();
  });

  onMount(() => {
    el.value = props.value;
    grow();
    const onSel = () => {
      if (document.activeElement === el) props.onCaret?.(caret());
    };
    document.addEventListener('selectionchange', onSel);
    onCleanup(() => document.removeEventListener('selectionchange', onSel));
  });

  return (
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
      onBeforeInput={(e) => {
        const t = (e as InputEvent).inputType ?? '';
        kind = t === 'insertText' || t === 'insertCompositionText' ? 'type' : t.startsWith('delete') ? 'delete' : 'other';
        if (t === 'historyUndo' || t === 'historyRedo') {
          if (props.onHistory) {
            e.preventDefault();
            props.onHistory(t === 'historyUndo' ? 'undo' : 'redo');
          }
        }
      }}
      onInput={() => {
        props.onChange(el.value, caret(), kind);
        kind = 'other';
        grow();
      }}
      onCompositionStart={() => (composing = true)}
      onCompositionEnd={() => {
        composing = false;
        // The final composed text: make sure the draft has exactly what the field shows.
        if (el.value !== props.value) props.onChange(el.value, caret(), 'type');
      }}
      onKeyDown={(e) => props.onKeyDown?.(e)}
      onKeyUp={() => props.onCaret?.(caret())}
      onClick={() => props.onCaret?.(caret())}
      onPaste={(e) => {
        const text = e.clipboardData?.getData('text/plain') ?? '';
        if (text && props.onPasteText?.(text)) e.preventDefault();
      }}
    />
  );
}
