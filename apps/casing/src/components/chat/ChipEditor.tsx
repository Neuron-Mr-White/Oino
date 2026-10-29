/**
 * The composer's chip-aware field (UNI-212): a contenteditable whose model
 * is still the plain draft string (exactly what pi gets, modulo the
 * attachment-path swap at send). `tokenize()` decides which tokens show as
 * atomic chips; everything else is plain text.
 *
 * DOM rules (canonical form): one text node per plain run, one
 * `contenteditable=false` span per chip (`data-raw` = its token), and a
 * trailing `<br data-tail>` when the text ends in a newline (so the empty
 * last line is visible). Typing edits text nodes in place — the DOM is only
 * rebuilt when its structure no longer matches the tokenization (a chip
 * formed or broke, the browser inserted a `<div>`/`<br>`, a paste), and
 * never while an IME (GBoard) composition is open: rebuilding under a live
 * composition is what makes Android keyboards duplicate or eat words.
 *
 * Chips are atomic: Backspace/Delete next to one removes the whole token
 * (handled in `beforeinput`, which GBoard also fires), the caret can't land
 * inside one, paste/drop insert plain text only.
 */
import { createEffect, onCleanup, onMount } from 'solid-js';
import { File as FileIcon, Folder, Image as ImageIcon, Sparkles, SquareSlash } from 'lucide-solid';
import { removeToken, tokenize, type Chip, type Seg, type TokenizeOpts } from '../../lib/chat/chips';
import { classifyPaste } from '../../lib/chat/paste';
import { undoKey, type EditKind } from '../../lib/chat/undo';

export type ChipEditorApi = {
  focus(): void;
  /** Moves the caret to a character offset in the draft string. */
  setCaret(offset: number): void;
  getCaret(): number;
};

export type ChipStatus = { status?: 'uploading' | 'done' | 'error'; error?: string };

export function ChipEditor(props: {
  value: string;
  /** `kind` (UNI-270): how the text changed — typing and deleting coalesce in the undo history. */
  onChange: (text: string, caret: number, kind: EditKind) => void;
  /** UNI-270: Ctrl/⌘+Z, Ctrl/⌘+Shift+Z, Ctrl/⌘+Y and the Edit menu's / context menu's Undo/Redo
   * (`historyUndo`/`historyRedo`). The browser's own stack can't follow a DOM this field rebuilds,
   * so the caller keeps the history; without this prop the browser's is left alone. */
  onHistory?: (op: 'undo' | 'redo') => void;
  onCaret?: (caret: number) => void;
  opts: () => TokenizeOpts;
  /** Live state for an attachment chip (upload status). */
  chipState?: (chip: Chip) => ChipStatus | undefined;
  /** UNI-285: the label for a chip the token itself can't name — an image
   *  chip's "image N" (its position among the pending images). */
  chipLabel?: (chip: Chip) => string | undefined;
  /** A file chip's ✕ (the chip's token is already gone from `value` then). */
  onChipRemoved?: (chip: Chip) => void;
  onKeyDown?: (e: KeyboardEvent) => void;
  /** Files pasted from the clipboard (UNI-117, desktop): attach them instead of pasting text. */
  onPasteFiles?: (files: File[]) => void;
  /** UNI-256 (desktop app): the paste event showed no files and maybe no text (WebKitGTK hides
   * clipboard images and copied files) — the caller reads the clipboard natively. Resolves true
   * when it attached something; otherwise the event's text (if any) is pasted. */
  onPasteNative?: () => Promise<boolean>;
  placeholder?: string;
  class?: string;
  api?: (api: ChipEditorApi) => void;
  ariaLabel?: string;
}) {
  let root!: HTMLDivElement;
  let composing = false;
  /** The text the DOM currently shows (what we last rendered or read). */
  let shown = '';
  let caret = 0;
  /** The kind of the edit the browser is applying (from `beforeinput`), read back in `sync`. */
  let pendingKind: EditKind = 'other';
  const kindOf = (inputType: string): EditKind =>
    inputType === 'insertText' || inputType === 'insertCompositionText' ? 'type' : /^delete(Content|Word|SoftLine|HardLine)/.test(inputType) ? 'delete' : 'other';

  const segsFor = (text: string) => tokenize(text, props.opts());
  /** A chip's visible label: the caller's (an image chip's "image N"), else
   *  what the token itself names. */
  const labelOf = (chip: Chip): string =>
    props.chipLabel?.(chip) ??
    (chip.kind === 'file' || chip.kind === 'skill'
      ? chip.name
      : chip.kind === 'path'
        ? chip.path.replace(/\/$/, '').split('/').pop() || chip.path
        : chip.kind === 'image'
          ? chip.raw
          : `/${chip.name}`);
  const stateKey = (chip: Chip) => {
    const st = props.chipState?.(chip);
    return st ? `${st.status ?? ''}:${st.error ?? ''}` : '';
  };

  // ── DOM ⇄ text ─────────────────────────────────────────────────────
  const serialize = (node: Node): string => {
    let out = '';
    node.childNodes.forEach((child, i) => {
      if (child.nodeType === Node.TEXT_NODE) out += (child as Text).data.replace(/\u200b/g, '');
      else if (child instanceof HTMLElement) {
        if (child.dataset.raw !== undefined) out += child.dataset.raw;
        else if (child.tagName === 'BR') {
          // A <br> closing a block (Chrome's `<div><br></div>`) or our tail is not a newline of its own,
          // nor is the placeholder <br> Chrome drops at the end of the field when its last text node
          // empties (backspacing the last char, or the space after a trailing chip) — counting that
          // one made the draft "\n", which no Backspace could clear (Chrome re-adds it every time).
          const last = i === node.childNodes.length - 1;
          if (child.dataset.tail === undefined && !last) out += '\n';
        } else if (child.tagName === 'DIV' || child.tagName === 'P') {
          if (out && !out.endsWith('\n')) out += '\n';
          else if (!out && i > 0) out += '\n';
          out += serialize(child);
        } else out += serialize(child);
      }
    });
    return out;
  };
  const chipOf = (node: Node | null): HTMLElement | null => {
    for (let n: Node | null = node; n && n !== root; n = n.parentNode) if (n instanceof HTMLElement && n.dataset.raw !== undefined) return n;
    return null;
  };
  /** Draft offset of a DOM point (a point inside a chip snaps to its end). */
  const offsetOf = (node: Node, offset: number): number => {
    const chip = chipOf(node);
    const range = document.createRange();
    range.setStart(root, 0);
    if (chip) range.setEndAfter(chip);
    else range.setEnd(node, offset);
    const holder = document.createElement('div');
    holder.append(range.cloneContents());
    // cloneContents drops our tail marker's meaning only for the tail itself.
    return serialize(holder).length;
  };
  const readCaret = (): number | undefined => {
    const sel = document.getSelection();
    if (!sel || !sel.rangeCount || !sel.focusNode || !root.contains(sel.focusNode)) return undefined;
    return offsetOf(sel.focusNode, sel.focusOffset);
  };
  const placeCaret = (offset: number) => {
    const sel = document.getSelection();
    if (!sel) return;
    let at = 0;
    const range = document.createRange();
    for (const child of [...root.childNodes]) {
      const len = child.nodeType === Node.TEXT_NODE ? (child as Text).data.length : child instanceof HTMLElement && child.dataset.raw !== undefined ? child.dataset.raw.length : 0;
      if (child.nodeType === Node.TEXT_NODE && offset <= at + len) {
        range.setStart(child, Math.max(0, offset - at));
        range.collapse(true);
        sel.removeAllRanges();
        sel.addRange(range);
        return;
      }
      if (len && offset < at + len) {
        // Inside a chip: snap after it.
        range.setStartAfter(child);
        range.collapse(true);
        sel.removeAllRanges();
        sel.addRange(range);
        return;
      }
      at += len;
      if (len && offset === at) {
        range.setStartAfter(child);
        range.collapse(true);
        sel.removeAllRanges();
        sel.addRange(range);
        return;
      }
    }
    range.selectNodeContents(root);
    const tail = root.querySelector('br[data-tail]');
    if (tail) range.setStartBefore(tail);
    range.collapse(!!tail);
    sel.removeAllRanges();
    sel.addRange(range);
  };

  // ── rendering ──────────────────────────────────────────────────────
  const chipEl = (chip: Chip): HTMLElement => {
    const el = document.createElement('span');
    el.className = 'ichip';
    el.contentEditable = 'false';
    el.dataset.raw = chip.raw;
    el.dataset.kind = chip.kind;
    const st = props.chipState?.(chip);
    if (st?.status) el.dataset.status = st.status;
    const label = labelOf(chip);
    const icon =
      chip.kind === 'image' ? <ImageIcon size={13} /> : chip.kind === 'file' ? <FileIcon size={13} /> : chip.kind === 'path' ? (chip.path.endsWith('/') ? <Folder size={13} /> : <FileIcon size={13} />) : chip.kind === 'skill' ? <Sparkles size={13} /> : <SquareSlash size={13} />;
    el.append(icon as Node);
    const text = document.createElement('span');
    text.className = 'ichip-label';
    text.textContent = label;
    el.title = chip.kind === 'file' ? (st?.status === 'error' ? `${chip.name}: ${st.error ?? 'upload failed'}` : chip.name) : chip.kind === 'path' ? chip.path : chip.raw;
    el.setAttribute('aria-label', `${chip.kind === 'command' ? 'command' : chip.kind === 'skill' ? 'skill' : chip.kind === 'image' ? 'image' : 'file'} ${text.textContent}${st?.status === 'uploading' ? ', uploading' : st?.status === 'error' ? ', failed' : ''}`);
    el.append(text);
    if (st?.status === 'uploading') {
      const spin = document.createElement('span');
      spin.className = 'ichip-spin';
      spin.setAttribute('aria-hidden', 'true');
      el.append(spin);
    }
    if (chip.kind === 'file' || chip.kind === 'image') {
      const x = document.createElement('button');
      x.type = 'button';
      x.className = 'ichip-x';
      x.tabIndex = -1;
      x.setAttribute('aria-label', `Remove ${text.textContent}`);
      x.textContent = '×';
      x.addEventListener('mousedown', (e) => e.preventDefault());
      x.addEventListener('click', (e) => {
        e.preventDefault();
        e.stopPropagation();
        removeChipEl(el, chip);
      });
      el.append(x);
    }
    return el;
  };
  let renderedKey = '';
  const markEmpty = (text: string) => {
    if (text) delete root.dataset.empty;
    else root.dataset.empty = '';
  };
  // Structure only (chips + their live state, and the label a caller derives
  // from live state — UNI-285's "image N"); plain text is compared node by
  // node in `canonical`.
  const keyOf = (segs: Seg[]) => segs.map((s) => ('chip' in s ? `c${s.chip.raw}|${stateKey(s.chip)}|${labelOf(s.chip)}` : 't')).join('\u0000');
  const render = (text: string, caretAt?: number) => {
    const segs = segsFor(text);
    root.textContent = '';
    for (const s of segs) root.append('chip' in s ? chipEl(s.chip) : document.createTextNode(s.text));
    if (text.endsWith('\n') || !text) {
      const br = document.createElement('br');
      br.dataset.tail = '';
      root.append(br);
    }
    shown = text;
    renderedKey = keyOf(segs);
    markEmpty(text);
    if (caretAt !== undefined && document.activeElement === root) {
      placeCaret(caretAt);
      revealCaret(caretAt, text.length);
    }
  };
  /** UNI-256: a rebuilt DOM + programmatic caret never scrolls the field — after a long paste the
   * field sat at its max height showing the top, the pasted end hidden below. Scroll the caret in. */
  const revealCaret = (at: number, length: number) => {
    if (root.scrollHeight <= root.clientHeight) return;
    if (at >= length) {
      root.scrollTop = root.scrollHeight;
      return;
    }
    const sel = document.getSelection();
    const range = sel && sel.rangeCount ? sel.getRangeAt(0) : undefined;
    const rect = range?.getClientRects()[0] ?? range?.getBoundingClientRect();
    if (!rect || (!rect.top && !rect.bottom)) return;
    const box = root.getBoundingClientRect();
    const pad = 4;
    if (rect.bottom > box.bottom - pad) root.scrollTop += rect.bottom - box.bottom + pad;
    else if (rect.top < box.top + pad) root.scrollTop -= box.top + pad - rect.top;
  };
  /** Whether the live DOM is still the canonical rendering of `text`. */
  const canonical = (text: string): boolean => {
    const segs = segsFor(text);
    if (keyOf(segs) !== renderedKey) return false;
    const nodes = [...root.childNodes];
    const wantTail = text.endsWith('\n') || !text;
    const tail = nodes[nodes.length - 1];
    const hasTail = tail instanceof HTMLElement && tail.tagName === 'BR' && tail.dataset.tail !== undefined;
    if (wantTail !== hasTail) return false;
    const body = hasTail ? nodes.slice(0, -1) : nodes;
    if (body.length !== segs.length) return false;
    return segs.every((s, i) => {
      const n = body[i]!;
      if ('chip' in s) return n instanceof HTMLElement && n.dataset.raw === s.chip.raw;
      return n.nodeType === Node.TEXT_NODE && (n as Text).data === s.text;
    });
  };

  const emit = (text: string, at: number, kind: EditKind = 'other') => {
    shown = text;
    caret = at;
    props.onChange(text, at, kind);
  };
  /** After the browser changed the DOM: read it back, emit, canonicalize. */
  const sync = () => {
    if (composing) return;
    const text = serialize(root);
    const at = readCaret() ?? text.length;
    if (text !== shown) emit(text, at, pendingKind);
    pendingKind = 'other';
    if (!canonical(text)) render(text, at);
    else markEmpty(text);
  };
  const replaceText = (text: string, at: number, kind: EditKind = 'other') => {
    emit(text, at, kind);
    render(text, at);
  };
  const removeChipEl = (el: HTMLElement, chip: Chip) => {
    const text = serialize(root);
    const start = offsetOf(el, 0) - chip.raw.length;
    const next = removeToken(text, Math.max(0, start), chip.raw);
    replaceText(next.text, next.caret);
    props.onChipRemoved?.(chip);
  };
  /** The chip whose token ends (`back`) or starts (forward) at `at`. */
  const chipAt = (at: number, back: boolean): { chip: Chip; start: number } | undefined => {
    let pos = 0;
    for (const s of segsFor(shown)) {
      const len = 'chip' in s ? s.chip.raw.length : s.text.length;
      if ('chip' in s && (back ? pos + len === at : pos === at)) return { chip: s.chip, start: pos };
      pos += len;
    }
    return undefined;
  };

  // ── events ─────────────────────────────────────────────────────────
  const onBeforeInput = (e: InputEvent) => {
    if (e.inputType === 'historyUndo' || e.inputType === 'historyRedo') {
      if (!props.onHistory) return;
      e.preventDefault();
      if (!composing) props.onHistory(e.inputType === 'historyUndo' ? 'undo' : 'redo');
      return;
    }
    if (composing || e.isComposing) {
      pendingKind = 'type';
      return;
    }
    const type = e.inputType;
    pendingKind = kindOf(type);
    if (type === 'deleteContentBackward' || type === 'deleteContentForward') {
      const sel = document.getSelection();
      if (!sel || !sel.isCollapsed) return;
      const at = readCaret();
      if (at === undefined) return;
      const hit = chipAt(at, type === 'deleteContentBackward');
      if (!hit) return;
      e.preventDefault();
      const next = removeToken(shown, hit.start, hit.chip.raw);
      replaceText(next.text, next.caret);
      props.onChipRemoved?.(hit.chip);
      return;
    }
    if (type === 'insertParagraph' || type === 'insertLineBreak') {
      e.preventDefault();
      insertPlain('\n', 'type');
      return;
    }
    if (type === 'insertFromDrop') e.preventDefault();
  };
  const insertPlain = (plain: string, kind: EditKind = 'other') => {
    const sel = document.getSelection();
    const text = serialize(root);
    let start = readCaret() ?? text.length;
    let end = start;
    if (sel && !sel.isCollapsed && sel.anchorNode && root.contains(sel.anchorNode)) {
      const a = offsetOf(sel.anchorNode, sel.anchorOffset);
      start = Math.min(a, end);
      end = Math.max(a, end);
    }
    const next = text.slice(0, start) + plain + text.slice(end);
    replaceText(next, start + plain.length, kind);
  };
  const onPaste = (e: ClipboardEvent) => {
    e.preventDefault();
    // UNI-117: files on the clipboard (a screenshot, copied files) become attachments when the
    // caller takes them (desktop); otherwise only the text is pasted, as before.
    // UNI-256: WebKitGTK shows neither — the caller reads the clipboard natively (lib/chat/paste.ts).
    const paste = classifyPaste(e.clipboardData, !!props.onPasteNative);
    if (paste.kind === 'files') {
      if (props.onPasteFiles) props.onPasteFiles(paste.files);
      else {
        const plain = (e.clipboardData?.getData('text/plain') ?? '').replace(/\r\n?/g, '\n');
        if (plain) insertPlain(plain);
      }
      return;
    }
    if (paste.kind === 'text') insertPlain(paste.text);
    else if (paste.kind === 'native') {
      const fallback = paste.text;
      void props.onPasteNative!()
        .catch((error) => {
          console.warn('[paste] native clipboard read failed', error);
          return false;
        })
        .then((attached) => {
          if (!attached && fallback) insertPlain(fallback);
        });
    }
  };
  const trackCaret = () => {
    const at = readCaret();
    if (at === undefined) return;
    caret = at;
    props.onCaret?.(at);
  };

  onMount(() => {
    render(props.value);
    props.api?.({
      focus: () => root.focus(),
      setCaret: (offset) => {
        caret = offset;
        root.focus();
        placeCaret(offset);
        props.onCaret?.(offset);
      },
      getCaret: () => (document.activeElement === root ? (readCaret() ?? caret) : caret),
    });
    const onSel = () => {
      if (document.activeElement === root) trackCaret();
    };
    document.addEventListener('selectionchange', onSel);
    onCleanup(() => document.removeEventListener('selectionchange', onSel));
  });

  // Outside changes (switching chats, a picked suggestion, a cleared draft,
  // an attachment finishing its upload): re-render, caret where the caller
  // put it (or the end).
  createEffect(() => {
    const value = props.value;
    const key = keyOf(segsFor(value));
    if (composing) return;
    if (value === shown && key === renderedKey && canonical(value)) return;
    const focused = document.activeElement === root;
    render(value, focused ? Math.min(caret, value.length) : undefined);
  });

  return (
    <div
      ref={root}
      class={`chip-field ${props.class ?? ''}`}
      contentEditable
      role="textbox"
      aria-multiline="true"
      aria-label={props.ariaLabel ?? props.placeholder}
      data-placeholder={props.placeholder}
      enterkeyhint="send"
      autocapitalize="sentences"
      spellcheck
      onBeforeInput={onBeforeInput}
      onInput={() => sync()}
      onCompositionStart={() => (composing = true)}
      onCompositionEnd={() => {
        composing = false;
        pendingKind = 'type';
        // Some engines fire the last input before compositionend, some after.
        queueMicrotask(sync);
      }}
      onPaste={onPaste}
      onDrop={(e) => e.preventDefault()}
      onKeyDown={(e) => {
        const op = props.onHistory && !composing && !e.isComposing ? undoKey(e) : null;
        if (op) {
          e.preventDefault();
          props.onHistory!(op);
          return;
        }
        props.onKeyDown?.(e);
      }}
      onKeyUp={trackCaret}
      onClick={trackCaret}
      onFocus={trackCaret}
    />
  );
}
