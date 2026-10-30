// @vitest-environment jsdom
/**
 * UNI-270: undo/redo in the chat composer. The field's DOM is rebuilt under
 * the browser (chips, paste, picks), so the composer keeps its own history:
 * Ctrl/⌘+Z, Ctrl/⌘+Shift+Z, Ctrl/⌘+Y and the Edit menu's historyUndo/Redo.
 */
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { createSignal } from 'solid-js';
import { createStore } from 'solid-js/store';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Composer } from './Composer';
import type { Chat } from '../../lib/chat/store';
import { clearComposerHistory } from '../../lib/chat/undo';
import { clearPendingFiles, pendingFilesFor } from '../../lib/chat/attachments';

let putResolvers: Array<(p: { path: string }) => void> = [];
vi.mock('../../lib/blob', () => ({
  blobTransport: () => ({ putBytes: vi.fn(() => new Promise((r) => putResolvers.push(r))) }),
  uploadPicked: vi.fn(() => new Promise((r) => putResolvers.push(r))),
}));
vi.mock('../../lib/imageShrink', () => ({ shrinkImage: vi.fn(), blobToBase64: vi.fn(), INLINE_MAX_BYTES: 1 }));

beforeEach(() => clearComposerHistory());
afterEach(() => {
  cleanup();
  putResolvers = [];
  clearPendingFiles('host1:42');
});

function caretAt(node: Node, offset: number) {
  const range = document.createRange();
  range.setStart(node, offset);
  range.collapse(true);
  const sel = document.getSelection()!;
  sel.removeAllRanges();
  sel.addRange(range);
}
function fakeChat(commands: Array<{ name: string; source: string; description?: string }> = []) {
  const [state] = createStore({ conn: 'live', run: { running: false }, entries: [], commands, models: [], dialogs: [], queue: [], pending: [] });
  return { state, hostId: 'host1', session: '42', send: vi.fn(), abort: vi.fn(), files: vi.fn(async () => []) } as unknown as Chat & { send: ReturnType<typeof vi.fn> };
}
function mount(initial = '', commands: Array<{ name: string; source: string }> = []) {
  const [draft, setDraft] = createSignal(initial);
  const chat = fakeChat(commands);
  const view = render(() => <Composer chat={chat} draft={draft()} setDraft={setDraft} onHaptic={() => {}} onModel={() => {}} onCommand={() => false} onSideQuestion={() => {}} onNewSession={() => {}} />);
  const field = view.container.querySelector('.chip-field') as HTMLDivElement;
  field.focus();
  /** Types like a browser: beforeinput, the text node changes, input. */
  const type = (chars: string) => {
    for (const c of chars) {
      field.dispatchEvent(new InputEvent('beforeinput', { inputType: 'insertText', data: c, bubbles: true, cancelable: true }));
      let node = field.lastChild;
      while (node && node.nodeType !== Node.TEXT_NODE) node = node.previousSibling;
      if (!node || field.lastChild?.nodeType === Node.ELEMENT_NODE && (field.lastChild as HTMLElement).dataset.raw !== undefined) {
        node = document.createTextNode('');
        const tail = field.querySelector('br[data-tail]');
        if (tail) field.insertBefore(node, tail);
        else field.append(node);
      }
      const t = node as Text;
      t.data += c;
      caretAt(t, t.data.length);
      fireEvent.input(field, { inputType: 'insertText', data: c });
    }
  };
  const backspace = () => {
    const ev = new InputEvent('beforeinput', { inputType: 'deleteContentBackward', bubbles: true, cancelable: true });
    field.dispatchEvent(ev);
    if (ev.defaultPrevented) return;
    const t = [...field.childNodes].reverse().find((n) => n.nodeType === Node.TEXT_NODE) as Text;
    t.data = t.data.slice(0, -1);
    caretAt(t, t.data.length);
    fireEvent.input(field, { inputType: 'deleteContentBackward' });
  };
  const key = (k: string, mods: { shift?: boolean; meta?: boolean } = {}) =>
    fireEvent.keyDown(field, { key: k, code: `Key${k.toUpperCase()}`, ctrlKey: !mods.meta, metaKey: !!mods.meta, shiftKey: !!mods.shift });
  const undo = () => key('z');
  const redo = () => key('z', { shift: true });
  const paste = (text: string) => {
    const ev = new Event('paste', { bubbles: true, cancelable: true }) as ClipboardEvent;
    Object.defineProperty(ev, 'clipboardData', { value: { types: ['text/plain'], getData: (t: string) => (t === 'text/plain' ? text : ''), files: [] } });
    field.dispatchEvent(ev);
  };
  return { view, chat, draft, setDraft, field, type, backspace, undo, redo, key, paste };
}

describe('Composer undo/redo (UNI-270)', () => {
  it('Ctrl+Z undoes typing word by word; Ctrl+Shift+Z and Ctrl+Y redo', () => {
    const c = mount();
    c.type('hello brave world');
    expect(c.draft()).toBe('hello brave world');
    c.undo();
    expect(c.draft()).toBe('hello brave ');
    expect(c.field.textContent).toBe('hello brave ');
    c.undo();
    expect(c.draft()).toBe('hello ');
    c.redo();
    expect(c.draft()).toBe('hello brave ');
    c.key('y');
    expect(c.draft()).toBe('hello brave world');
    c.undo();
    c.undo();
    c.undo();
    expect(c.draft()).toBe('');
    c.undo(); // nothing left: no-op
    expect(c.draft()).toBe('');
  });

  it('⌘+Z / ⌘+Shift+Z work too (macOS), and the key never reaches the browser', () => {
    const c = mount();
    c.type('abc');
    const ev = new KeyboardEvent('keydown', { key: 'z', metaKey: true, bubbles: true, cancelable: true });
    c.field.dispatchEvent(ev);
    expect(ev.defaultPrevented).toBe(true);
    expect(c.draft()).toBe('');
    c.key('z', { meta: true, shift: true });
    expect(c.draft()).toBe('abc');
  });

  it('the Edit menu / context menu Undo (beforeinput historyUndo) goes through the same history', () => {
    const c = mount();
    c.type('one two');
    const ev = new InputEvent('beforeinput', { inputType: 'historyUndo', bubbles: true, cancelable: true });
    c.field.dispatchEvent(ev);
    expect(ev.defaultPrevented).toBe(true);
    expect(c.draft()).toBe('one ');
    c.field.dispatchEvent(new InputEvent('beforeinput', { inputType: 'historyRedo', bubbles: true, cancelable: true }));
    expect(c.draft()).toBe('one two');
  });

  it('new typing after an undo clears redo', () => {
    const c = mount();
    c.type('keep drop');
    c.undo();
    expect(c.draft()).toBe('keep ');
    c.type('new');
    c.redo();
    expect(c.draft()).toBe('keep new');
    c.undo();
    expect(c.draft()).toBe('keep ');
  });

  it('Backspaces coalesce into one step', () => {
    const c = mount();
    c.type('abcdef');
    c.backspace();
    c.backspace();
    c.backspace();
    expect(c.draft()).toBe('abc');
    c.undo();
    expect(c.draft()).toBe('abcdef');
  });

  it('a paste is one step of its own', () => {
    const c = mount();
    c.type('see');
    c.paste(' pasted\nlines');
    expect(c.draft()).toBe('see pasted\nlines');
    c.undo();
    expect(c.draft()).toBe('see');
    c.redo();
    expect(c.draft()).toBe('see pasted\nlines');
  });

  it('a slash-command pick is one step; undo brings the typed query back', async () => {
    const c = mount('', [{ name: 'skill:review', source: 'skill' }]);
    c.type('/rev');
    const row = await waitFor(() => {
      const r = c.view.container.querySelector('.slash-row');
      expect(r).not.toBeNull();
      return r!;
    });
    fireEvent.click(row);
    expect(c.draft()).toBe('/skill:review ');
    await waitFor(() => expect(c.field.querySelector('.ichip')).not.toBeNull());
    c.undo();
    expect(c.draft()).toBe('/rev');
    expect(c.field.querySelector('.ichip')).toBeNull();
    c.redo();
    expect(c.draft()).toBe('/skill:review ');
    expect(c.field.querySelector('.ichip')).not.toBeNull();
  });

  it('removing an attachment chip can be undone, and its upload survives so the chip comes back working', async () => {
    const c = mount('a ');
    caretAt(c.field.firstChild!, 2);
    const input = c.view.container.querySelector('input[aria-label="Attach a file"]') as HTMLInputElement;
    const files = [new File([new Uint8Array([1])], 'notes.txt', { type: 'text/plain' })];
    Object.defineProperty(input, 'files', { value: Object.assign([...files], { item: (i: number) => files[i] }), configurable: true });
    fireEvent.change(input);
    await waitFor(() => expect(c.field.querySelector('.ichip')).not.toBeNull());
    const withChip = c.draft();
    fireEvent.click(c.field.querySelector('.ichip-x')!);
    expect(c.draft()).toBe('a ');
    await waitFor(() => expect(pendingFilesFor('host1:42').length).toBe(1));
    c.undo();
    expect(c.draft()).toBe(withChip);
    expect(c.field.querySelector('.ichip')!.getAttribute('data-status')).toBe('uploading');
    c.undo(); // before the attachment
    expect(c.draft()).toBe('a ');
  });

  it('after a send the field is empty and Ctrl+Z brings the sent text back (once)', () => {
    const c = mount();
    c.type('ship it');
    fireEvent.click(c.view.container.querySelector('.send-btn')!);
    expect(c.chat.send).toHaveBeenCalledWith('ship it', 'auto');
    expect(c.draft()).toBe('');
    c.undo();
    expect(c.draft()).toBe('ship it');
    c.undo();
    expect(c.draft()).toBe('ship it');
    c.redo();
    expect(c.draft()).toBe('');
  });

  it('a draft change from outside (a quote, restored queue) is its own undo step', () => {
    const c = mount();
    c.type('mine');
    c.setDraft('> quoted\nmine');
    c.undo();
    expect(c.draft()).toBe('mine');
  });

  it('Ctrl+Z with an IME composition open is left to the keyboard', () => {
    const c = mount();
    c.type('abc');
    fireEvent.compositionStart(c.field);
    const ev = new KeyboardEvent('keydown', { key: 'z', ctrlKey: true, bubbles: true, cancelable: true });
    c.field.dispatchEvent(ev);
    expect(ev.defaultPrevented).toBe(false);
    expect(c.draft()).toBe('abc');
  });
});
