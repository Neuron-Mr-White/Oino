// @vitest-environment jsdom
/**
 * Inline chips in the composer (UNI-212): the contenteditable keeps the
 * draft string as its model; chips are atomic (Backspace removes the whole
 * token), an open IME composition is never re-rendered under the keyboard,
 * paste is plain text, attachments land inline at the caret and the sent
 * text keeps the UNI-203 wire format.
 */
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { createSignal } from 'solid-js';
import { createStore } from 'solid-js/store';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ChipEditor } from './ChipEditor';
import { Composer } from './Composer';
import type { Chat } from '../../lib/chat/store';
import { attachmentToken } from '../../lib/chat/chips';
import { clearPendingFiles } from '../../lib/chat/attachments';

let putResolvers: Array<(p: { path: string }) => void> = [];
vi.mock('../../lib/blob', () => ({
  blobTransport: () => ({ putBytes: vi.fn(() => new Promise((r) => putResolvers.push(r))) }),
  uploadPicked: vi.fn(() => new Promise((r) => putResolvers.push(r))),
}));
vi.mock('../../lib/imageShrink', () => ({ shrinkImage: vi.fn(), blobToBase64: vi.fn(), INLINE_MAX_BYTES: 1 }));

afterEach(() => {
  cleanup();
  putResolvers = [];
  clearPendingFiles('host1:42');
});
const flush = () => new Promise((r) => setTimeout(r, 0));

function caretAt(node: Node, offset: number) {
  const range = document.createRange();
  range.setStart(node, offset);
  range.collapse(true);
  const sel = document.getSelection()!;
  sel.removeAllRanges();
  sel.addRange(range);
}
function beforeInput(el: Element, inputType: string, extra: Record<string, unknown> = {}) {
  const ev = new InputEvent('beforeinput', { inputType, bubbles: true, cancelable: true, ...extra });
  el.dispatchEvent(ev);
  return ev;
}

function mountEditor(initial: string, opts = {}) {
  const [value, setValue] = createSignal(initial);
  const changes: string[] = [];
  const removed: unknown[] = [];
  const view = render(() => (
    <ChipEditor
      value={value()}
      onChange={(t) => (changes.push(t), setValue(t))}
      opts={() => opts}
      onChipRemoved={(c) => removed.push(c)}
      placeholder="Message pi"
    />
  ));
  const root = view.container.querySelector('.chip-field') as HTMLDivElement;
  return { root, value, setValue, changes, removed };
}

describe('ChipEditor (UNI-212)', () => {
  it('renders chips inline as atomic contenteditable=false spans between text nodes', () => {
    const { root } = mountEditor(`Hello world, this is the file: ${attachmentToken('a.pdf', 'f1')} and more`);
    const nodes = [...root.childNodes];
    expect(nodes.length).toBe(3);
    expect(nodes[0]!.textContent).toBe('Hello world, this is the file: ');
    const chip = nodes[1] as HTMLElement;
    expect(chip.contentEditable).toBe('false');
    expect(chip.dataset.raw).toBe('[file: a.pdf]@att:f1');
    expect(chip.querySelector('.ichip-label')!.textContent).toBe('a.pdf');
    expect(nodes[2]!.textContent).toBe(' and more');
  });

  it('shows the placeholder only while empty', () => {
    const { root, setValue } = mountEditor('');
    expect(root.dataset.empty).toBe('');
    setValue('x');
    expect(root.dataset.empty).toBeUndefined();
  });

  it('Backspace right after a chip deletes the whole chip token (and one space)', () => {
    const { root, value, removed } = mountEditor(`see ${attachmentToken('a.pdf', 'f1')} ok`);
    root.focus();
    caretAt(root, 2); // just after the chip span
    const ev = beforeInput(root, 'deleteContentBackward');
    expect(ev.defaultPrevented).toBe(true);
    expect(value()).toBe('see ok');
    expect(removed).toEqual([expect.objectContaining({ kind: 'file', attId: 'f1' })]);
    expect(root.querySelector('.ichip')).toBeNull();
  });

  it('Delete right before a chip removes it too; plain Backspace elsewhere is left to the browser', () => {
    const { root, value } = mountEditor(`ab ${attachmentToken('a.pdf', 'f1')}`);
    root.focus();
    caretAt(root.firstChild!, 3);
    expect(beforeInput(root, 'deleteContentForward').defaultPrevented).toBe(true);
    expect(value()).toBe('ab');
    caretAt(root.firstChild!, 1);
    expect(beforeInput(root, 'deleteContentBackward').defaultPrevented).toBe(false);
  });

  it('the chip ✕ removes it', () => {
    const { root, value, removed } = mountEditor(`x ${attachmentToken('a.pdf', 'f1')} y`);
    fireEvent.click(root.querySelector('.ichip-x')!);
    expect(value()).toBe('x y');
    expect(removed.length).toBe(1);
  });

  it("Chrome's placeholder <br> at the end of the field is not a newline (backspacing the last char empties the draft)", () => {
    // Seen on Android WebView: Backspace on the last character leaves `<br>` (no data-tail) as the
    // only child; it used to read back as "\n" — placeholder gone and no Backspace could clear it.
    const { root, value } = mountEditor('a');
    root.focus();
    root.textContent = '';
    root.append(document.createElement('br'));
    caretAt(root, 0);
    fireEvent.input(root);
    expect(value()).toBe('');
    expect(root.dataset.empty).toBe('');
  });

  it("Chrome's placeholder <br> after a trailing chip (space deleted) adds no newline", () => {
    const { root, value } = mountEditor('read @README.md ', { verified: (t: string) => t === '@README.md', requireTrailingSpace: true });
    root.focus();
    const chip = root.querySelector('.ichip')!;
    // Chrome: the trailing ' ' text node emptied and was replaced by a placeholder <br>.
    while (chip.nextSibling) chip.nextSibling.remove();
    root.append(document.createElement('br'));
    caretAt(root, 2);
    fireEvent.input(root);
    expect(value()).toBe('read @README.md');
  });

  it('a <br> in the middle of the field is still a newline', () => {
    const { root, value } = mountEditor('ab');
    root.focus();
    root.textContent = '';
    root.append('a', document.createElement('br'), 'b');
    caretAt(root, 3);
    fireEvent.input(root);
    expect(value()).toBe('a\nb');
  });

  it('typing into a text node reads back the draft without rebuilding the DOM', () => {
    const { root, value } = mountEditor('hello');
    const text = root.firstChild as Text;
    text.data = 'hello there';
    caretAt(text, 11);
    fireEvent.input(root);
    expect(value()).toBe('hello there');
    expect(root.firstChild).toBe(text);
  });

  it('IME/GBoard: no re-render while composing; the text syncs once on compositionend', async () => {
    const opts = { commands: new Set(['unipi:goal']), requireTrailingSpace: true };
    const { root, value, changes } = mountEditor('', opts);
    root.focus();
    fireEvent.compositionStart(root);
    // The keyboard writes the composing word straight into the DOM.
    root.textContent = '';
    const t = document.createTextNode('/unipi:goal');
    root.append(t);
    caretAt(t, 11);
    fireEvent.input(root, { isComposing: true });
    expect(changes).toEqual([]);
    expect(root.firstChild).toBe(t);
    t.data = '/unipi:goal ';
    caretAt(t, 12);
    fireEvent.input(root, { isComposing: true });
    expect(root.firstChild).toBe(t);
    fireEvent.compositionEnd(root);
    await flush();
    expect(value()).toBe('/unipi:goal ');
    // Committed: now it's a finished command → chip.
    expect((root.firstChild as HTMLElement).dataset?.raw).toBe('/unipi:goal');
  });

  it('paste inserts plain text only (no HTML), replacing the selection', () => {
    const { root, value } = mountEditor('hi ');
    root.focus();
    caretAt(root.firstChild!, 3);
    const data = { getData: (t: string) => (t === 'text/plain' ? 'pasted\r\nline' : '<b>bold</b>') };
    const ev = new Event('paste', { bubbles: true, cancelable: true }) as ClipboardEvent;
    Object.defineProperty(ev, 'clipboardData', { value: data });
    root.dispatchEvent(ev);
    expect(ev.defaultPrevented).toBe(true);
    expect(value()).toBe('hi pasted\nline');
    expect(root.querySelector('b')).toBeNull();
  });

  function pasteEvent(root: Element, data: Record<string, string>, files: File[] = []) {
    const ev = new Event('paste', { bubbles: true, cancelable: true }) as ClipboardEvent;
    Object.defineProperty(ev, 'clipboardData', { value: { types: Object.keys(data), getData: (t: string) => data[t] ?? '', files } });
    root.dispatchEvent(ev);
    return ev;
  }
  function mountNative(initial: string, native: () => Promise<boolean>) {
    const [value, setValue] = createSignal(initial);
    const pasted: File[][] = [];
    const view = render(() => (
      <ChipEditor value={value()} onChange={(t) => setValue(t)} opts={() => ({})} onPasteFiles={(f) => pasted.push(f)} onPasteNative={native} />
    ));
    return { root: view.container.querySelector('.chip-field') as HTMLDivElement, value, pasted };
  }

  it('UNI-256: an empty paste (WebKitGTK clipboard image) asks the native clipboard; nothing is typed', async () => {
    const native = vi.fn(async () => true);
    const { root, value } = mountNative('hi ', native);
    root.focus();
    caretAt(root.firstChild!, 3);
    expect(pasteEvent(root, {}).defaultPrevented).toBe(true);
    await flush();
    expect(native).toHaveBeenCalledTimes(1);
    expect(value()).toBe('hi ');
  });

  it('UNI-256: copied files (uri list) go native; when the core finds nothing the text is pasted', async () => {
    const native = vi.fn(async () => false);
    const { root, value } = mountNative('', native);
    root.focus();
    pasteEvent(root, { 'x-special/gnome-copied-files': '', 'text/plain': '/tmp/a.png' });
    await flush();
    expect(native).toHaveBeenCalledTimes(1);
    expect(value()).toBe('/tmp/a.png');
  });

  it('UNI-256: plain text never touches the native clipboard; Files on the event are attached', async () => {
    const native = vi.fn(async () => true);
    const { root, value, pasted } = mountNative('', native);
    root.focus();
    pasteEvent(root, { 'text/plain': 'just text' });
    const png = new File([new Uint8Array(2)], 'a.png', { type: 'image/png' });
    pasteEvent(root, {}, [png]);
    await flush();
    expect(native).not.toHaveBeenCalled();
    expect(value()).toBe('just text');
    expect(pasted).toEqual([[png]]);
  });

  it('UNI-256: a failing native read falls back to the text, never throws', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const { root, value } = mountNative('', async () => {
      throw new Error('no clipboard');
    });
    root.focus();
    pasteEvent(root, { 'text/uri-list': '', 'text/plain': 'fallback' });
    await flush();
    await flush();
    expect(value()).toBe('fallback');
    warn.mockRestore();
  });

  it('UNI-256: after a long paste the field scrolls to the caret at the end', () => {
    const { root, value } = mountEditor('');
    Object.defineProperty(root, 'scrollHeight', { configurable: true, get: () => 900 });
    Object.defineProperty(root, 'clientHeight', { configurable: true, get: () => 168 });
    root.focus();
    pasteEvent(root, { 'text/plain': Array.from({ length: 40 }, (_, i) => `line ${i}`).join('\n') });
    expect(value().split('\n')).toHaveLength(40);
    expect(root.scrollTop).toBe(900);
  });

  it('Enter (insertParagraph) becomes a plain newline in the draft', () => {
    const { root, value } = mountEditor('a');
    root.focus();
    caretAt(root.firstChild!, 1);
    expect(beforeInput(root, 'insertParagraph').defaultPrevented).toBe(true);
    expect(value()).toBe('a\n');
  });

  it('a browser-inserted <div> line is normalized back to the canonical text', () => {
    const { root, value } = mountEditor('one');
    const div = document.createElement('div');
    div.textContent = 'two';
    root.append(div);
    fireEvent.input(root);
    expect(value()).toBe('one\ntwo');
    expect(root.querySelector('div')).toBeNull();
  });
});

function fakeChat(commands: Array<{ name: string; source: string }> = []): Chat & { send: ReturnType<typeof vi.fn> } {
  const [state] = createStore({ conn: 'live', run: { running: false }, entries: [], commands, models: [], dialogs: [], queue: [], pending: [] });
  return { state, hostId: 'host1', session: '42', send: vi.fn(), abort: vi.fn(), files: vi.fn(async () => []), argCompletions: vi.fn(async () => []) } as unknown as Chat & { send: ReturnType<typeof vi.fn> };
}
function mountComposer(initial = '', commands: Array<{ name: string; source: string }> = []) {
  const [draft, setDraft] = createSignal(initial);
  const chat = fakeChat(commands);
  const view = render(() => <Composer chat={chat} draft={draft()} setDraft={setDraft} onHaptic={() => {}} onModel={() => {}} onCommand={() => false} onSideQuestion={() => {}} onNewSession={() => {}} />);
  const field = view.container.querySelector('.chip-field') as HTMLDivElement;
  return { view, chat, draft, setDraft, field };
}
function attach(view: ReturnType<typeof render>, names: string[]) {
  const input = view.container.querySelector('input[aria-label="Attach a file"]') as HTMLInputElement;
  const files = names.map((n) => new File([new Uint8Array([1])], n, { type: 'text/plain' }));
  Object.defineProperty(input, 'files', { value: Object.assign([...files], { item: (i: number) => files[i] }), configurable: true });
  fireEvent.change(input);
}

describe('Composer inline attachments (UNI-212)', () => {
  it('attaching puts the chip INSIDE the text at the caret, and the sent text is the UNI-203 wire mention in place', async () => {
    const { view, chat, draft, field } = mountComposer('Hello world, this is the file:  and more');
    field.focus();
    caretAt(field.firstChild!, 31);
    attach(view, ['report.pdf']);
    await waitFor(() => expect(field.querySelector('.ichip')).not.toBeNull());
    expect(draft()).toMatch(/^Hello world, this is the file: \[file: report\.pdf\]@att:\S+ and more$/);
    expect(field.querySelector('.ichip')!.getAttribute('data-status')).toBe('uploading');
    expect((view.container.querySelector('.send-btn') as HTMLButtonElement).disabled).toBe(false);
    await waitFor(() => expect(putResolvers.length).toBe(1));
    putResolvers.forEach((r) => r({ path: '/home/u/.unipi/app-host/blobs/abc' }));
    await waitFor(() => expect(field.querySelector('.ichip')!.getAttribute('data-status')).toBe('done'));
    fireEvent.click(view.container.querySelector('.send-btn')!);
    expect(chat.send).toHaveBeenCalledWith('Hello world, this is the file: [file: report.pdf]@/home/u/.unipi/app-host/blobs/abc and more', 'auto');
    expect(draft()).toBe('');
  });

  it('8 attachments stay inline chips in the field (no tall strip above the composer)', async () => {
    const { view, field } = mountComposer('');
    attach(view, Array.from({ length: 8 }, (_, i) => `file-${i + 1}.txt`));
    await waitFor(() => expect(field.querySelectorAll('.ichip').length).toBe(8));
    expect(view.container.querySelector('.composer-file-strip')).toBeNull();
  });

  it('a file still uploading is held back: the rest is sent, its chip stays in the draft', async () => {
    const { view, chat, draft, field } = mountComposer('note ');
    field.focus();
    caretAt(field.firstChild!, 5);
    attach(view, ['slow.bin']);
    await waitFor(() => expect(field.querySelector('.ichip')).not.toBeNull());
    fireEvent.click(view.container.querySelector('.send-btn')!);
    expect(chat.send).toHaveBeenCalledWith('note', 'auto');
    expect(draft()).toMatch(/^\[file: slow\.bin\]@att:\S+ $/);
  });

  it('a session command typed at the start becomes a chip; unknown ones stay plain; the text sent is unchanged', async () => {
    const { view, chat, field, setDraft } = mountComposer('', [{ name: 'skill:review', source: 'skill' }]);
    setDraft('/skill:review the diff');
    await flush();
    expect(field.querySelector('.ichip')?.getAttribute('data-kind')).toBe('skill');
    setDraft('/skill:unknown the diff');
    await flush();
    expect(field.querySelector('.ichip')).toBeNull();
    setDraft('/skill:review the diff');
    await flush();
    fireEvent.click(view.container.querySelector('.send-btn')!);
    expect(chat.send).toHaveBeenCalledWith('/skill:review the diff', 'auto');
  });

  it('an @path picked from the @ menu chips (it exists on the PC); a typed unknown one stays plain', async () => {
    const { view, chat, field, setDraft } = mountComposer('');
    (chat.files as ReturnType<typeof vi.fn>).mockImplementation(async (q: string) => (q === 'src/a' ? [{ value: '@src/a.ts', label: 'a.ts', path: 'src/a.ts', dir: false }] : []));
    setDraft('read @src/a');
    await flush();
    field.focus();
    caretAt(field.firstChild!, 11);
    fireEvent.keyUp(field);
    await waitFor(() => expect(view.container.querySelector('.slash-row')).not.toBeNull(), { timeout: 1500 });
    fireEvent.click(view.container.querySelector('.slash-row')!);
    await waitFor(() => expect(field.querySelector('.ichip[data-kind=path]')).not.toBeNull());
    expect(field.querySelector('.ichip-label')!.textContent).toBe('a.ts');
  });

  it('UNI-276: `/cmd ` shows the command\'s argument suggestions; tapping one fills it in, Enter never replaces typed text', async () => {
    const { view, chat, field, setDraft, draft } = mountComposer('', [{ name: 'unipi:dream', source: 'extension' }]);
    (chat.argCompletions as ReturnType<typeof vi.fn>).mockImplementation(async (_c: string, p: string) => [{ value: 'status', label: 'status', description: 'last run' }, { value: 'report', label: 'report' }].filter((x) => x.value.startsWith(p)));
    setDraft('/unipi:dream ');
    await flush();
    field.focus();
    caretAt(field.lastChild!, (field.lastChild!.textContent ?? '').length);
    fireEvent.keyUp(field);
    const menu = await waitFor(() => view.getByTestId('arg-menu'), { timeout: 1500 });
    expect(menu.textContent).toContain('status');
    expect(chat.argCompletions).toHaveBeenCalledWith('unipi:dream', '');
    fireEvent.click(menu.querySelectorAll('.slash-row')[1]!);
    await waitFor(() => expect(draft()).toBe('/unipi:dream report'));
  });
});
