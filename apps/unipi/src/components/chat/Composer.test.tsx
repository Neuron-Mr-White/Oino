// @vitest-environment jsdom
import { render, cleanup, fireEvent, waitFor } from '@solidjs/testing-library';
import { createSignal } from 'solid-js';
import { createStore } from 'solid-js/store';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setPlatformOverride } from '../../lib/platform';
import { resetLayout } from '../../lib/layout';
import { Composer, type ComposerApi } from './Composer';
import type { Chat } from '../../lib/chat/store';
import { clearComposerHistory } from '../../lib/chat/undo';

// jsdom has no canvas/createImageBitmap: the shrink step is irrelevant to
// the composer's own attach/chip-state logic, so it's faked out here.
vi.mock('../../lib/imageShrink', () => ({
  // A file whose name says "bad" cannot be read, like a 0-byte paste: the
  // image never gets attached (UNI-285 fixes the orphan chip that left).
  shrinkImage: vi.fn(async (file: Blob & { name?: string }) => {
    if ((file as File).name?.includes('bad')) throw new Error('could not decode');
    return { blob: file, mime: 'image/png', width: 10, height: 10 };
  }),
  blobToBase64: vi.fn(async () => 'ZmFrZQ=='),
  INLINE_MAX_BYTES: 700 * 1024,
}));

beforeEach(() => clearComposerHistory());
afterEach(cleanup);

function fakeChat(overrides?: { running?: boolean; queue?: unknown[]; commands?: unknown[]; argCompletions?: Chat['argCompletions'] }): Chat {
  const [state] = createStore({
    conn: 'live',
    run: { running: overrides?.running ?? false },
    entries: [],
    truncated: false,
    tools: {},
    commands: overrides?.commands ?? [],
    models: [],
    dialogs: [],
    queue: overrides?.queue ?? [],
    pending: [],
    hasOlder: false,
    loadingOlder: false,
    seams: [],
  });
  return {
    state,
    hostId: 'host1',
    session: '42',
    send: vi.fn(),
    abort: vi.fn(),
    files: vi.fn(async () => []),
    argCompletions: overrides?.argCompletions ?? vi.fn(async () => []),
    queueEdit: vi.fn(),
    queueRemove: vi.fn(),
    queuePromote: vi.fn(),
    queueMove: vi.fn(),
    clearRestored: vi.fn(),
  } as unknown as Chat;
}

function pngFile(name = 'shot.png'): File {
  return new File([new Uint8Array([1, 2, 3])], name, { type: 'image/png' });
}

/** jsdom has no DataTransfer; fakes just enough of a FileList for the
 * composer's change handler (`[...files]` + `.length`). */
function setInputFiles(input: HTMLInputElement, files: File[]) {
  const list: Pick<FileList, 'length' | 'item'> & Iterable<File> & Record<number, File> = {
    length: files.length,
    item: (i: number) => files[i] ?? null,
    [Symbol.iterator]: () => files[Symbol.iterator](),
  };
  files.forEach((f, i) => (list[i] = f));
  Object.defineProperty(input, 'files', { value: list, configurable: true });
}

const flush = () => new Promise((r) => setTimeout(r, 0));

describe('chat Composer draft sync (UNI-136/212)', () => {
  it('re-renders the chip field when the draft changes from outside (switching chats)', async () => {
    const drafts: Record<string, string> = { a: 'one\ntwo', b: 'hi' };
    const [key, setKey] = createSignal('a');
    const { container } = render(() => (
      <Composer
        chat={fakeChat()}
        draft={drafts[key()]!}
        setDraft={(t) => (drafts[key()] = t)}
        onHaptic={() => {}}
        onModel={() => {}}
        onCommand={() => false}
        onSideQuestion={() => {}}
        onNewSession={() => {}}
       
      />
    ));
    const field = container.querySelector('.chip-field')!;
    expect(field.textContent).toBe('one\ntwo');
    setKey('b');
    await flush();
    expect(field.textContent).toBe('hi');
  });
});

describe('chat Composer image attachments (UNI-157)', () => {
  function mount() {
    const [draft, setDraft] = createSignal('');
    const chat = fakeChat();
    const view = render(() => (
      <Composer chat={chat} draft={draft()} setDraft={setDraft} onHaptic={() => {}} onModel={() => {}} onCommand={() => false} onSideQuestion={() => {}} onNewSession={() => {}} />
    ));
    return { view, chat, draft };
  }

  it('picking a photo shows a chip and enables Send with no text', async () => {
    const { view } = mount();
    const input = view.container.querySelector('input[type=file]') as HTMLInputElement;
    setInputFiles(input, [pngFile()]);
    fireEvent.change(input);
    await waitFor(() => expect(view.container.querySelector('.composer-image-loading')).toBeNull());
    expect(view.container.querySelectorAll('.composer-image-chip').length).toBe(1);
    const sendBtn = view.container.querySelector('.send-btn') as HTMLButtonElement;
    expect(sendBtn.disabled).toBe(false);
  });

  it('does not leave a stuck loading chip after the file input is cleared (regression)', async () => {
    // The <input>'s FileList is live: clearing `input.value` right after
    // picking also clears the FileList the handler was given. `pickImages`
    // must snapshot the files before that happens, or `attaching` never
    // goes back to 0 and a loading placeholder chip is stuck forever.
    const { view } = mount();
    const input = view.container.querySelector('input[type=file]') as HTMLInputElement;
    setInputFiles(input, [pngFile()]);
    fireEvent.change(input);
    await waitFor(() => expect(view.container.querySelector('.composer-image-loading')).toBeNull());
    expect(view.container.querySelectorAll('.composer-image-chip').length).toBe(1);
    expect(view.container.querySelector('.composer-image-remove')).not.toBeNull();
  });

  it('removing a chip revokes its preview and drops it from the strip', async () => {
    const { view } = mount();
    const input = view.container.querySelector('input[type=file]') as HTMLInputElement;
    setInputFiles(input, [pngFile()]);
    fireEvent.change(input);
    await waitFor(() => expect(view.container.querySelector('.composer-image-remove')).not.toBeNull());
    fireEvent.click(view.container.querySelector('.composer-image-remove')!);
    await waitFor(() => expect(view.container.querySelectorAll('.composer-image-chip').length).toBe(0));
  });
});

describe('chat Composer send choice (UNI-202, in-flow since UNI-212)', () => {
  it('busy + text to send: tapping Send opens the Steer/Queue bar in the composer stack (not a floating popover)', async () => {
    const [draft, setDraft] = createSignal('do the thing');
    const chat = fakeChat({ running: true });
    const { container } = render(() => (
      <Composer chat={chat} draft={draft()} setDraft={setDraft} onHaptic={() => {}} onModel={() => {}} onCommand={() => false} onSideQuestion={() => {}} onNewSession={() => {}} />
    ));
    fireEvent.click(container.querySelector('.send-btn')!);
    const bar = container.querySelector('.send-choice-bar');
    expect(bar).not.toBeNull();
    expect(bar!.nextElementSibling?.classList.contains('chat-composer')).toBe(true);
    expect(document.querySelector('[data-testid=anchored-menu-backdrop]')).toBeNull();
    expect(chat.send).not.toHaveBeenCalled();
    expect(container.querySelectorAll('.send-choice-pill').length).toBe(2);
  });

  it('picking a pill sends with that mode, remembers the choice, and closes the bar', async () => {
    const [draft, setDraft] = createSignal('steer this');
    const chat = fakeChat({ running: true });
    const { container } = render(() => (
      <Composer chat={chat} draft={draft()} setDraft={setDraft} onHaptic={() => {}} onModel={() => {}} onCommand={() => false} onSideQuestion={() => {}} onNewSession={() => {}} />
    ));
    fireEvent.click(container.querySelector('.send-btn')!);
    const [steerPill, queuePill] = [...container.querySelectorAll<HTMLButtonElement>('.send-choice-pill')];
    expect(steerPill!.textContent).toMatch(/Steer/);
    expect(queuePill!.textContent).toMatch(/Queue/);
    fireEvent.click(queuePill!);
    expect(container.querySelector('.send-choice-bar')).toBeNull();
    expect(chat.send).toHaveBeenCalledWith('steer this', 'after');
    expect(localStorage.getItem('unipi.chat.sendChoice.host1.42')).toBe('after');
  });

  it('the run ending while the bar is open closes it (nothing to steer any more)', async () => {
    const [draft, setDraft] = createSignal('late');
    const [state, setState] = createStore<any>({ conn: 'live', run: { running: true }, entries: [], commands: [], models: [], dialogs: [], queue: [], pending: [] });
    const chat = { ...fakeChat(), state } as unknown as Chat;
    const { container } = render(() => (
      <Composer chat={chat} draft={draft()} setDraft={setDraft} onHaptic={() => {}} onModel={() => {}} onCommand={() => false} onSideQuestion={() => {}} onNewSession={() => {}} />
    ));
    fireEvent.click(container.querySelector('.send-btn')!);
    expect(container.querySelector('.send-choice-bar')).not.toBeNull();
    setState('run', 'running', false);
    await flush();
    expect(container.querySelector('.send-choice-bar')).toBeNull();
  });

  it('idle: tapping Send still sends right away, no choice', async () => {
    const [draft, setDraft] = createSignal('hello');
    const chat = fakeChat({ running: false });
    const { container } = render(() => (
      <Composer chat={chat} draft={draft()} setDraft={setDraft} onHaptic={() => {}} onModel={() => {}} onCommand={() => false} onSideQuestion={() => {}} onNewSession={() => {}} />
    ));
    fireEvent.click(container.querySelector('.send-btn')!);
    expect(container.querySelector('.send-choice-bar')).toBeNull();
    expect(chat.send).toHaveBeenCalledWith('hello', 'auto');
  });
});

describe('chat Composer queue strip (UNI-202/211, compact since UNI-212)', () => {
  function mountWithQueue(queue: unknown[]) {
    const [draft, setDraft] = createSignal('');
    const chat = fakeChat({ running: true, queue });
    const view = render(() => (
      <Composer chat={chat} draft={draft()} setDraft={setDraft} onHaptic={() => {}} onModel={() => {}} onCommand={() => false} onSideQuestion={() => {}} onNewSession={() => {}} />
    ));
    return { view, chat };
  }
  const btn = (root: Element, label: string) => root.querySelector(`[aria-label="${label}"]`) as HTMLButtonElement;

  it('one compact row: text + Edit / Remove / ⋯ icon buttons on the same line; Steer now / Send now live in ⋯', async () => {
    const { view, chat } = mountWithQueue([{ id: 'tui-1', text: 'steer from TUI', mode: 'steer', source: 'tui', editable: true }]);
    const row = view.container.querySelector('.queue-row')!;
    expect([...row.querySelectorAll('.queue-icon-btn')].map((b) => b.getAttribute('aria-label'))).toEqual(['Edit', 'Remove', 'More actions']);
    expect(row.querySelector('.queue-actions')!.parentElement).toBe(row);
    fireEvent.click(btn(row, 'More actions'));
    const menu = document.querySelector('[role=menu]')!;
    expect(menu.textContent).toMatch(/Steer now/);
    expect(menu.textContent).toMatch(/Send now/);
    fireEvent.click([...menu.querySelectorAll('button')].find((b) => b.textContent?.includes('Send now'))!);
    expect(chat.queuePromote).toHaveBeenCalledWith('tui-1', 'now');
  });

  it('bridge-owned rows reorder: Move up inline (when >1), Move down in ⋯', async () => {
    const { view, chat } = mountWithQueue([
      { id: 'q1', text: 'first', mode: 'after', source: 'phone', editable: true },
      { id: 'q2', text: 'second', mode: 'after', source: 'phone', editable: true },
    ]);
    const [r1, r2] = [...view.container.querySelectorAll('.queue-row')];
    expect(btn(r1!, 'Move up').disabled).toBe(true);
    fireEvent.click(btn(r2!, 'Move up'));
    expect(chat.queueMove).toHaveBeenCalledWith('q2', 0);
    fireEvent.click(btn(r1!, 'More actions'));
    fireEvent.click([...document.querySelectorAll('[role=menu] button')].find((b) => b.textContent?.includes('Move down'))!);
    expect(chat.queueMove).toHaveBeenCalledWith('q1', 1);
  });

  it('a single bridge row shows no reorder button', () => {
    const { view } = mountWithQueue([{ id: 'q1', text: 'after it ends', mode: 'after', source: 'phone', editable: true }]);
    expect(btn(view.container.querySelector('.queue-row')!, 'Move up')).toBeNull();
  });

  it('editing a row calls chat.queueEdit with the new text', async () => {
    const { view, chat } = mountWithQueue([{ id: 'tui-1', text: 'original', mode: 'steer', source: 'tui', editable: true }]);
    fireEvent.click(btn(view.container, 'Edit'));
    const field = view.container.querySelector('.queue-edit-field') as HTMLTextAreaElement;
    fireEvent.input(field, { target: { value: 'edited text' } });
    fireEvent.blur(field);
    expect(chat.queueEdit).toHaveBeenCalledWith('tui-1', 'edited text');
  });

  it('Remove calls chat.queueRemove', async () => {
    const { view, chat } = mountWithQueue([{ id: 'tui-1', text: 'drop me', mode: 'followUp', source: 'tui', editable: true }]);
    fireEvent.click(btn(view.container, 'Remove'));
    expect(chat.queueRemove).toHaveBeenCalledWith('tui-1');
  });
});

describe('desktop composer keys + paste (UNI-117)', () => {
  beforeEach(() => {
    setPlatformOverride({ mode: 'tauri-desktop', os: 'linux', tauri: false });
    Object.defineProperty(window, 'innerWidth', { value: 1440, configurable: true });
    vi.stubGlobal('matchMedia', (q: string) => ({ matches: q.includes('min-width'), media: q, addEventListener() {}, removeEventListener() {} }));
    resetLayout();
    localStorage.clear();
  });
  afterEach(() => {
    setPlatformOverride(undefined);
    Object.defineProperty(window, 'innerWidth', { value: 1024, configurable: true });
    vi.unstubAllGlobals();
    vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} });
    resetLayout();
  });
  function mount(running = false, text = 'hello') {
    const [draft, setDraft] = createSignal(text);
    const chat = fakeChat({ running });
    const view = render(() => (
      <Composer chat={chat} draft={draft()} setDraft={setDraft} onHaptic={() => {}} onModel={() => {}} onCommand={() => false} onSideQuestion={() => {}} onNewSession={() => {}} />
    ));
    const field = view.container.querySelector('.chip-field') as HTMLElement;
    return { view, chat, field, draft };
  }
  const key = (el: HTMLElement, k: string, m: KeyboardEventInit = {}) => fireEvent.keyDown(el, { key: k, ...m });

  it('Enter sends, Shift+Enter does not', () => {
    const { chat, field } = mount();
    key(field, 'Enter', { shiftKey: true });
    expect(chat.send).not.toHaveBeenCalled();
    key(field, 'Enter');
    expect(chat.send).toHaveBeenCalledWith('hello', 'auto');
  });

  it('pi working: Enter uses the last choice (Steer first)', () => {
    const { chat, field } = mount(true);
    key(field, 'Enter');
    expect(chat.send).toHaveBeenLastCalledWith('hello', 'steer');
  });

  it('pi working: Ctrl+Enter → the choice bar with S/Q keys; Q queues and is remembered', async () => {
    const { chat, field, view } = mount(true);
    key(field, 'Enter', { ctrlKey: true });
    await waitFor(() => expect(view.container.querySelector('.send-choice-bar')).not.toBeNull());
    expect(view.container.querySelectorAll('.send-choice-kbd')).toHaveLength(2);
    key(field, 'ArrowRight');
    expect(view.container.querySelector('.send-choice-pill[data-picked]')!.textContent).toContain('Queue');
    key(field, 'q');
    expect(chat.send).toHaveBeenLastCalledWith('hello', 'after');
    expect(localStorage.getItem('unipi.chat.sendChoice.host1.42')).toBe('after');
  });

  it('Esc while pi works stops it', () => {
    const { chat, field } = mount(true, '');
    key(field, 'Escape');
    expect(chat.abort).toHaveBeenCalled();
  });

  it('a pasted image becomes an attachment chip, not text', async () => {
    const { field, view, draft } = mount(false, '');
    const paste = Object.assign(new Event('paste', { bubbles: true, cancelable: true }), {
      clipboardData: { files: [pngFile('clip.png')], getData: () => '' },
    });
    field.dispatchEvent(paste);
    await waitFor(() => expect(view.container.querySelectorAll('.composer-image-chip:not(.composer-image-loading)').length).toBe(1));
    // UNI-285: the image also gets an inline chip in the draft.
    expect(draft()).toMatch(/^\[image]@img:\S+ $/);
  });
});

describe('phone composer is unchanged (UNI-117)', () => {
  it('Ctrl+Enter while pi works does not open the choice on the phone layout; paste stays text', async () => {
    setPlatformOverride({ mode: 'tauri-mobile', os: 'android', tauri: false });
    resetLayout();
    const [draft, setDraft] = createSignal('');
    const chat = fakeChat({ running: true });
    const view = render(() => (
      <Composer chat={chat} draft={draft()} setDraft={setDraft} onHaptic={() => {}} onModel={() => {}} onCommand={() => false} onSideQuestion={() => {}} onNewSession={() => {}} />
    ));
    const field = view.container.querySelector('.chip-field') as HTMLElement;
    fireEvent.keyDown(field, { key: 'Escape' });
    expect(chat.abort).not.toHaveBeenCalled();
    const paste = Object.assign(new Event('paste', { bubbles: true, cancelable: true }), { clipboardData: { files: [pngFile('clip.png')], getData: () => 'typed' } });
    field.dispatchEvent(paste);
    await flush();
    expect(view.container.querySelectorAll('.composer-image-chip').length).toBe(0);
    setPlatformOverride(undefined);
    resetLayout();
  });
});

describe('image chips in the draft (UNI-285)', () => {
  const stripImages = (view: { container: HTMLElement }) => view.container.querySelectorAll('.composer-image-chip:not(.composer-image-loading)').length;
  const draftImageChips = (view: { container: HTMLElement }) =>
    [...view.container.querySelectorAll<HTMLElement>('.ichip[data-kind=image]')].map((el) => el.querySelector('.ichip-label')!.textContent);
  const pick = async (view: { container: HTMLElement }, files: File[]) => {
    const input = view.container.querySelector('input[type=file]') as HTMLInputElement;
    setInputFiles(input, files);
    fireEvent.change(input);
  };
  /** jsdom never focuses the field for us, and a chip goes in at the caret. */
  const caretToEnd = (view: { container: HTMLElement }) => {
    const field = view.container.querySelector('.chip-field') as HTMLElement;
    field.focus();
    const node = field.lastChild!;
    const range = document.createRange();
    range.setStart(node, node.textContent!.length);
    range.collapse(true);
    const sel = document.getSelection()!;
    sel.removeAllRanges();
    sel.addRange(range);
    return field;
  };

  it('picking photos inserts an inline chip at the caret, numbered in attach order', async () => {
    const [draft, setDraft] = createSignal('look at these: ');
    const chat = fakeChat();
    const view = render(() => (
      <Composer chat={chat} draft={draft()} setDraft={setDraft} onHaptic={() => {}} onModel={() => {}} onCommand={() => false} onSideQuestion={() => {}} onNewSession={() => {}} />
    ));
    caretToEnd(view);
    await pick(view, [pngFile('one.png'), pngFile('two.png')]);
    await waitFor(() => expect(stripImages(view)).toBe(2));
    expect(draft()).toMatch(/^look at these: \[image]@img:\S+ \[image]@img:\S+ $/);
    await waitFor(() => expect(draftImageChips(view)).toEqual(['image 1', 'image 2']));
    // The two tokens are two different images, in pick order.
    const ids = [...draft().matchAll(/\[image]@img:(\S+)/g)].map((m) => m[1]);
    expect(new Set(ids).size).toBe(2);
  });

  it('removing the image from the strip removes its chip and renumbers the rest', async () => {
    const [draft, setDraft] = createSignal('');
    const chat = fakeChat();
    const view = render(() => (
      <Composer chat={chat} draft={draft()} setDraft={setDraft} onHaptic={() => {}} onModel={() => {}} onCommand={() => false} onSideQuestion={() => {}} onNewSession={() => {}} />
    ));
    await pick(view, [pngFile('one.png'), pngFile('two.png')]);
    await waitFor(() => expect(stripImages(view)).toBe(2));
    await waitFor(() => expect(draftImageChips(view)).toEqual(['image 1', 'image 2']));
    fireEvent.click(view.container.querySelector('.composer-image-remove')!);
    await waitFor(() => expect(stripImages(view)).toBe(1));
    await waitFor(() => expect(draftImageChips(view)).toEqual(['image 1']));
    expect([...draft().matchAll(/\[image]@img:/g)]).toHaveLength(1);
  });

  it('removing the chip drops its token but keeps the image undo can still restore (UNI-270)', async () => {
    const [draft, setDraft] = createSignal('');
    const chat = fakeChat();
    const view = render(() => (
      <Composer chat={chat} draft={draft()} setDraft={setDraft} onHaptic={() => {}} onModel={() => {}} onCommand={() => false} onSideQuestion={() => {}} onNewSession={() => {}} />
    ));
    await pick(view, [pngFile('one.png')]);
    await waitFor(() => expect(stripImages(view)).toBe(1));
    await waitFor(() => expect(draftImageChips(view)).toEqual(['image 1']));
    fireEvent.click(view.container.querySelector('.ichip[data-kind=image] .ichip-x')!);
    await waitFor(() => expect(draft()).toBe(''));
    // Same rule as a file chip: undo can still bring the token back, so the
    // image stays so the chip comes back working.
    expect(stripImages(view)).toBe(1);
  });

  it('Ctrl+Z brings a removed image chip (and its image) back', async () => {
    const [draft, setDraft] = createSignal('');
    const chat = fakeChat();
    const view = render(() => (
      <Composer chat={chat} draft={draft()} setDraft={setDraft} onHaptic={() => {}} onModel={() => {}} onCommand={() => false} onSideQuestion={() => {}} onNewSession={() => {}} />
    ));
    await pick(view, [pngFile('one.png')]);
    await waitFor(() => expect(stripImages(view)).toBe(1));
    await waitFor(() => expect(draftImageChips(view)).toEqual(['image 1']));
    fireEvent.click(view.container.querySelector('.ichip[data-kind=image] .ichip-x')!);
    await waitFor(() => expect(draft()).toBe(''));
    fireEvent.keyDown(view.container.querySelector('.chip-field') as HTMLElement, { key: 'z', code: 'KeyZ', ctrlKey: true });
    await waitFor(() => expect(draftImageChips(view)).toEqual(['image 1']));
  });

  it('send: each chip becomes `[image N]`, images sent in the same order', async () => {
    const [draft, setDraft] = createSignal('compare ');
    const chat = fakeChat();
    const view = render(() => (
      <Composer chat={chat} draft={draft()} setDraft={setDraft} onHaptic={() => {}} onModel={() => {}} onCommand={() => false} onSideQuestion={() => {}} onNewSession={() => {}} />
    ));
    caretToEnd(view);
    await pick(view, [pngFile('one.png'), pngFile('two.png')]);
    await waitFor(() => expect(stripImages(view)).toBe(2));
    await waitFor(() => expect(draftImageChips(view)).toEqual(['image 1', 'image 2']));
    fireEvent.click(view.container.querySelector('.send-btn') as HTMLButtonElement);
    await waitFor(() => expect(chat.send).toHaveBeenCalled());
    const [text, mode, images] = (chat.send as unknown as { mock: { calls: unknown[][] } }).mock.calls[0]!;
    expect(text).toBe('compare [image 1] [image 2]');
    expect(mode).toBe('auto');
    expect(images).toEqual([{ mime: 'image/png', data: 'ZmFrZQ==' }, { mime: 'image/png', data: 'ZmFrZQ==' }]);
    await waitFor(() => expect(draft()).toBe(''));
  });
});

describe('image chip labelling in the draft (UNI-285)', () => {
  const labels = (view: { container: HTMLElement }) =>
    [...view.container.querySelectorAll<HTMLElement>('.ichip[data-kind=image] .ichip-label')].map((el) => el.textContent);
  const stripImages = (view: { container: HTMLElement }) => view.container.querySelectorAll('.composer-image-chip:not(.composer-image-loading)').length;

  it('renders "image N" in the DOM, never the raw token', async () => {
    // The pending image never resolves (a decode failure): the chip must
    // still read "image 1", not "[image]@img:img1234".
    const [draft, setDraft] = createSignal('see [image]@img:ghost1 and [image]@img:ghost2 here');
    const chat = fakeChat();
    const view = render(() => (
      <Composer chat={chat} draft={draft()} setDraft={setDraft} onHaptic={() => {}} onModel={() => {}} onCommand={() => false} onSideQuestion={() => {}} onNewSession={() => {}} />
    ));
    await waitFor(() => expect(view.container.querySelectorAll('.ichip[data-kind=image]')).toHaveLength(2));
    expect(labels(view)).toEqual(['image 1', 'image 2']);
    expect(view.container.textContent).not.toContain('@img:');
  });

  it('an image that fails to attach takes its chip back out of the draft', async () => {
    const [draft, setDraft] = createSignal('');
    const chat = fakeChat();
    const view = render(() => (
      <Composer chat={chat} draft={draft()} setDraft={setDraft} onHaptic={() => {}} onModel={() => {}} onCommand={() => false} onSideQuestion={() => {}} onNewSession={() => {}} />
    ));
    const input = view.container.querySelector('input[type=file]') as HTMLInputElement;
    setInputFiles(input, [pngFile('bad.png'), pngFile('one.png')]);
    fireEvent.change(input);
    // Only the image that survived has a token; the failed one's chip is gone.
    await waitFor(() => expect([...draft().matchAll(/\[image]@img:/g)]).toHaveLength(1));
    await waitFor(() => expect(stripImages(view)).toBe(1));
    await waitFor(() => expect(labels(view)).toEqual(['image 1']));
  });
});

describe('restore a message into the draft (UNI-290 "Start from here")', () => {
  const draftImageChips = (view: { container: HTMLElement }) =>
    [...view.container.querySelectorAll<HTMLElement>('.ichip[data-kind=image]')].map((el) => el.querySelector('.ichip-label')!.textContent);

  it('replaces the draft, focuses the field and puts the caret at the end', async () => {
    const [draft, setDraft] = createSignal('');
    let api: ComposerApi | undefined;
    const view = render(() => (
      <Composer chat={fakeChat()} draft={draft()} setDraft={setDraft} onHaptic={() => {}} onModel={() => {}} onCommand={() => false} onSideQuestion={() => {}} onNewSession={() => {}} api={(a) => (api = a)} />
    ));
    api!.restore('fix the login bug');
    expect(draft()).toBe('fix the login bug');
    const field = view.container.querySelector('.chip-field') as HTMLElement;
    await waitFor(() => expect(document.activeElement).toBe(field));
    const sel = document.getSelection()!;
    expect(sel.rangeCount).toBe(1);
    const range = sel.getRangeAt(0);
    const pre = document.createRange();
    pre.selectNodeContents(field);
    pre.setEnd(range.endContainer, range.endOffset);
    expect(pre.toString().length).toBe('fix the login bug'.length);
  });

  it("the message's images come back as image chips in the strip and the text", async () => {
    const [draft, setDraft] = createSignal('');
    let api: ComposerApi | undefined;
    const chat = fakeChat();
    const view = render(() => (
      <Composer chat={chat} draft={draft()} setDraft={setDraft} onHaptic={() => {}} onModel={() => {}} onCommand={() => false} onSideQuestion={() => {}} onNewSession={() => {}} api={(a) => (api = a)} />
    ));
    api!.restore('compare [image 1] with [image 2]', [Promise.resolve(pngFile('a.png')), Promise.resolve(pngFile('b.png'))]);
    expect(draft()).toMatch(/^compare \[image]@img:\S+ with \[image]@img:\S+$/);
    await waitFor(() => expect(view.container.querySelectorAll('.composer-image-chip:not(.composer-image-loading)').length).toBe(2));
    await waitFor(() => expect(draftImageChips(view)).toEqual(['image 1', 'image 2']));
    // Sending it again sends the same message, images in the same order.
    fireEvent.click(view.container.querySelector('button[aria-label="Send"]') as HTMLButtonElement);
    await waitFor(() => expect(chat.send).toHaveBeenCalled());
    const [text, , imgs] = (chat.send as ReturnType<typeof vi.fn>).mock.calls[0]!;
    expect(text).toBe('compare [image 1] with [image 2]');
    expect(imgs).toHaveLength(2);
  });

  it('an image that cannot be fetched any more drops its chip, the text stays', async () => {
    const [draft, setDraft] = createSignal('');
    let api: ComposerApi | undefined;
    render(() => (
      <Composer chat={fakeChat()} draft={draft()} setDraft={setDraft} onHaptic={() => {}} onModel={() => {}} onCommand={() => false} onSideQuestion={() => {}} onNewSession={() => {}} api={(a) => (api = a)} />
    ));
    api!.restore('see [image 1]', [Promise.resolve(undefined)]);
    await waitFor(() => expect(draft()).toBe('see'));
  });
});

describe('UNI-308: an unknown /command never falls through to a plain prompt', () => {
  it('a /name neither the session nor onCommand knows about calls onUnknownCommand, not chat.send', async () => {
    const [draft, setDraft] = createSignal('/made-up-command some args');
    const chat = fakeChat();
    const onUnknownCommand = vi.fn();
    const view = render(() => (
      <Composer
        chat={chat}
        draft={draft()}
        setDraft={setDraft}
        onHaptic={() => {}}
        onModel={() => {}}
        onCommand={() => false}
        onUnknownCommand={onUnknownCommand}
        onSideQuestion={() => {}}
        onNewSession={() => {}}
       
      />
    ));
    fireEvent.click(view.container.querySelector('button[aria-label="Send"]') as HTMLButtonElement);
    await flush();
    expect(onUnknownCommand).toHaveBeenCalledWith('made-up-command');
    expect(chat.send).not.toHaveBeenCalled();
  });

  it('a /name in the session list (an extension/skill/prompt command) is sent as text — pi expands/runs it itself', async () => {
    const [draft, setDraft] = createSignal('/skill:find-docs how do I');
    const chat = fakeChat({ commands: [{ name: 'skill:find-docs', source: 'skill' }] });
    const onUnknownCommand = vi.fn();
    const view = render(() => (
      <Composer
        chat={chat}
        draft={draft()}
        setDraft={setDraft}
        onHaptic={() => {}}
        onModel={() => {}}
        onCommand={() => false}
        onUnknownCommand={onUnknownCommand}
        onSideQuestion={() => {}}
        onNewSession={() => {}}
       
      />
    ));
    fireEvent.click(view.container.querySelector('button[aria-label="Send"]') as HTMLButtonElement);
    await flush();
    expect(onUnknownCommand).not.toHaveBeenCalled();
    expect(chat.send).toHaveBeenCalled();
  });

  it("a pi built-in (e.g. /reload) is sent through onCommand, not onUnknownCommand, even though it isn't a session command", async () => {
    const [draft, setDraft] = createSignal('/reload');
    const chat = fakeChat();
    const onUnknownCommand = vi.fn();
    const onCommand = vi.fn(() => true);
    render(() => (
      <Composer
        chat={chat}
        draft={draft()}
        setDraft={setDraft}
        onHaptic={() => {}}
        onModel={() => {}}
        onCommand={onCommand}
        onUnknownCommand={onUnknownCommand}
        onSideQuestion={() => {}}
        onNewSession={() => {}}
       
      />
    ));
    fireEvent.click(document.querySelector('button[aria-label="Send"]') as HTMLButtonElement);
    await flush();
    expect(onCommand).toHaveBeenCalledWith('reload', undefined);
    expect(onUnknownCommand).not.toHaveBeenCalled();
    expect(chat.send).not.toHaveBeenCalled();
  });
});
