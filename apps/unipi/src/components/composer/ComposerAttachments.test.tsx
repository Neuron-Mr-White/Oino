// @vitest-environment jsdom
/**
 * Composer attachments (WP-A13 acceptance; inline chips, owner spec
 * 2026-11-03): each attachment is an inline chip at the caret carrying the Z-60 states,
 * the send gate holds Send until everything is ready, a failed upload offers
 * Retry, at most 8 chips fit, and — D-6 — a pending image belongs to its chat:
 * attach in one chat, switch to another, come back, and it is still there
 * while the other chat has only its own.
 *
 * The image shrink/encode is mocked (jsdom has no canvas); file uploads run
 * through a fake blob transport.
 */
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { createRoot } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createMockHost, type MockScenario, type MockWorld } from '../../dev/mock/host';
import { genFsTree, genLiveRun, genSession } from '../../dev/mock/gen';
import { setHostClientFactory } from '../../lib/host/factory';
import { createSession, type SessionHandle } from '../../lib/session/store';
import { resetComposerDrafts } from '../../lib/composer/drafts';
import { resetAttachments } from '../../lib/composer/attachments';
import { setBlobTransport, type BlobPutResult, type BlobTransport } from '../../lib/blob';
import { setDesktopOverride, resetLayout } from '../../lib/layout';
import { Composer } from './Composer';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));
vi.mock('../../lib/imageShrink', () => ({
  MAX_SIDE: 1568,
  QUALITY: 0.8,
  INLINE_MAX_BYTES: 700 * 1024,
  shrinkImage: async (file: Blob) => ({ blob: file, mime: 'image/png', width: 100, height: 80 }),
  blobToBase64: async () => 'QUJD',
}));

const HOST = 'pc-one';
const CWD = '/home/you/projects/unipi';
const A = 's-chat-a';
const B = 's-chat-b';

const WORKFLOW = [
  { name: 'permission', description: 'Permission mode', source: 'extension' },
  { name: 'plan', description: 'Plan mode', source: 'extension' },
];

function scenario(opts: { running?: boolean } = {}): MockScenario {
  const one = genSession(A, { seed: 3, turns: 2, toolsPerTurn: 0, cwd: CWD });
  const two = genSession(B, { seed: 4, turns: 2, toolsPerTurn: 0, cwd: CWD });
  one.snapshot.commands = WORKFLOW as never;
  two.snapshot.commands = WORKFLOW as never;
  if (opts.running) one.events = genLiveRun(A, { seq: 200, running: true, at: 0 });
  return { name: 'composer-attach', hosts: [{ id: HOST, name: 'coffee', conn: { state: 'connected' }, sessions: { [A]: one, [B]: two }, fs: genFsTree(CWD) }] };
}

let world: MockWorld;
let disposers: Array<() => void> = [];

async function open(id: string, opts: { running?: boolean } = {}): Promise<SessionHandle> {
  world = createMockHost(scenario(opts));
  setHostClientFactory(world.client);
  const handle = createRoot((dispose) => {
    disposers.push(dispose);
    return createSession(HOST, id, { client: world.client(HOST) });
  });
  await waitFor(() => expect(handle.state.conn).toBe('live'));
  await new Promise((r) => setTimeout(r, 20));
  return handle;
}

const pngFile = (name = 'shot.png') => new File([new Uint8Array(8)], name, { type: 'image/png' });

function mountComposer(session: SessionHandle) {
  const view = render(() => <Composer session={session} cwd={() => CWD} />);
  const pick = (files: File[]) => fireEvent.change(view.container.querySelector('input[aria-label="Attach photos"]')!, { target: { files } });
  const chips = () => [...view.container.querySelectorAll('.chip-field .ichip[data-kind=image], .chip-field .ichip[data-kind=file]')] as HTMLElement[];
  const field = () => view.container.querySelector('.chip-field') as HTMLDivElement;
  /** Types `text` into the field (the caret ends after it). */
  const type = (text: string) => {
    const f = field();
    f.focus();
    f.append(document.createTextNode(text));
    const r = document.createRange();
    r.setStart(f.lastChild!, (f.lastChild as Text).data.length);
    r.collapse(true);
    document.getSelection()!.removeAllRanges();
    document.getSelection()!.addRange(r);
    fireEvent.input(f);
  };
  /** Puts the caret at draft offset `n` of a plain-text field. */
  const caretAt = (n: number) => {
    const f = field();
    const r = document.createRange();
    r.setStart(f.firstChild!, n);
    r.collapse(true);
    document.getSelection()!.removeAllRanges();
    document.getSelection()!.addRange(r);
    fireEvent.keyUp(f);
  };
  const prompt = () => world.log.filter((e) => e.kind === 'cmd' && e.op === 'prompt').at(-1)?.args as { message: string; images?: Array<{ data: string }> } | undefined;
  return { view, pick, chips, field, type, caretAt, prompt };
}

/** A blob transport whose uploads wait for the test to settle them. */
function pendingTransport() {
  const calls: Array<{ resolve: () => void; reject: (e: Error) => void }> = [];
  const t: BlobTransport = {
    async putFile(): Promise<BlobPutResult> {
      throw new Error('not used');
    },
    async putBytes(_host, _bytes, _mime, name, onProgress): Promise<BlobPutResult> {
      onProgress?.({ sent: 1, total: 2 });
      return new Promise<BlobPutResult>((resolve, reject) => {
        calls.push({ resolve: () => resolve({ sha256: 'sha', size: 2, deduped: false, path: `/blobs/${name}` }), reject });
      });
    },
    async getFile() {
      throw new Error('not used');
    },
    async getBytes() {
      throw new Error('not used');
    },
  };
  return { t, calls };
}

beforeEach(() => {
  localStorage.clear();
  setDesktopOverride(true);
  resetAttachments();
});

afterEach(() => {
  cleanup();
  for (const dispose of disposers) dispose();
  disposers = [];
  resetComposerDrafts();
  resetAttachments();
  setBlobTransport(undefined);
  setDesktopOverride(undefined);
  resetLayout();
  setHostClientFactory(undefined);
  localStorage.clear();
});

describe('inline attachment chips (Z-60 states, owner spec 2026-11-03)', () => {
  it('an attached image becomes an inline "image 1" chip with its thumbnail', async () => {
    const s = await open(A);
    const c = mountComposer(s);
    c.pick([pngFile()]);
    await waitFor(() => expect(c.chips()).toHaveLength(1));
    await waitFor(() => expect(c.chips()[0]!.getAttribute('data-status')).toBe('done'));
    const chip = c.chips()[0]!;
    expect(chip.contentEditable).toBe('false');
    expect(chip.querySelector('img.ichip-thumb')?.getAttribute('src')).toBe('data:image/png;base64,QUJD');
    expect(chip.querySelector('.ichip-label')!.textContent).toBe('image 1');
    // No separate attachment row any more.
    expect(c.view.container.querySelector('[data-testid="attachment-chip"]')).toBeNull();
  });

  it('lands at the caret, and the model gets [image N] at that place with the images in text order', async () => {
    const s = await open(A);
    const c = mountComposer(s);
    c.type('compare  with');
    c.caretAt(8);
    c.pick([pngFile('one.png')]);
    await waitFor(() => expect(c.chips()[0]?.getAttribute('data-status')).toBe('done'));
    // Put the second one at the end.
    const f = c.field();
    const r = document.createRange();
    r.selectNodeContents(f);
    r.collapse(false);
    document.getSelection()!.removeAllRanges();
    document.getSelection()!.addRange(r);
    fireEvent.keyUp(f);
    c.pick([pngFile('two.png')]);
    await waitFor(() => expect(c.chips()).toHaveLength(2));
    await waitFor(() => expect(c.chips()[1]!.getAttribute('data-status')).toBe('done'));
    expect(c.chips().map((x) => x.querySelector('.ichip-label')!.textContent)).toEqual(['image 1', 'image 2']);
    fireEvent.click(c.view.container.querySelector('[data-testid="send-send"]')!);
    await waitFor(() => expect(c.prompt()).toBeDefined());
    expect(c.prompt()!.message).toBe('compare [image 1] with [image 2]');
    expect(c.prompt()!.images).toHaveLength(2);
  });

  it('Backspace after an image chip removes it whole and it is not sent; Undo brings it back', async () => {
    const s = await open(A);
    const c = mountComposer(s);
    c.type('see');
    c.pick([pngFile()]);
    await waitFor(() => expect(c.chips()[0]?.getAttribute('data-status')).toBe('done'));
    const f = c.field();
    const r = document.createRange();
    r.setStartAfter(c.chips()[0]!);
    r.collapse(true);
    document.getSelection()!.removeAllRanges();
    document.getSelection()!.addRange(r);
    f.dispatchEvent(new InputEvent('beforeinput', { inputType: 'deleteContentBackward', bubbles: true, cancelable: true }));
    await waitFor(() => expect(c.chips()).toHaveLength(0));
    fireEvent.keyDown(f, { key: 'z', ctrlKey: true });
    await waitFor(() => expect(c.chips()).toHaveLength(1));
    expect(c.chips()[0]!.getAttribute('data-status')).toBe('done');
    // Removed again → Send carries no image.
    const r2 = document.createRange();
    r2.setStartAfter(c.chips()[0]!);
    r2.collapse(true);
    document.getSelection()!.removeAllRanges();
    document.getSelection()!.addRange(r2);
    f.dispatchEvent(new InputEvent('beforeinput', { inputType: 'deleteContentBackward', bubbles: true, cancelable: true }));
    await waitFor(() => expect(c.chips()).toHaveLength(0));
    fireEvent.click(c.view.container.querySelector('[data-testid="send-send"]')!);
    await waitFor(() => expect(c.prompt()).toBeDefined());
    expect(c.prompt()!.message).toBe('see');
    expect(c.prompt()!.images ?? []).toHaveLength(0);
  });

  it('Send stays disabled until the file is uploaded, then sends [file: name]@<host path> in place', async () => {
    const transport = pendingTransport();
    setBlobTransport(transport.t);
    const s = await open(A);
    const c = mountComposer(s);
    c.type('look at this');
    fireEvent.change(c.view.container.querySelector('input[aria-label="Attach a file"]')!, {
      target: { files: [new File([new Uint8Array(4)], 'notes.txt', { type: 'text/plain' })] },
    });
    await waitFor(() => expect(transport.calls).toHaveLength(1));
    await waitFor(() => expect(c.chips()[0]?.getAttribute('data-status')).toBe('uploading'));
    expect(c.chips()[0]!.querySelector('.ichip-label')!.textContent).toBe('notes.txt');
    expect((c.view.container.querySelector('[data-testid="send-send"]') as HTMLButtonElement).disabled).toBe(true);

    transport.calls[0]!.resolve();
    await waitFor(() => expect(c.chips()[0]?.getAttribute('data-status')).toBe('done'));
    const send = c.view.container.querySelector('[data-testid="send-send"]') as HTMLButtonElement;
    expect(send.disabled).toBe(false);
    fireEvent.click(send);
    await waitFor(() => expect(c.prompt()).toBeDefined());
    expect(c.prompt()!.message).toBe('look at this [file: notes.txt]@/blobs/notes.txt');
    await waitFor(() => expect(c.chips()).toHaveLength(0));
  });

  it('Stop stays available while an attachment uploads (only Send is gated)', async () => {
    const transport = pendingTransport();
    setBlobTransport(transport.t);
    const s = await open(A, { running: true });
    await waitFor(() => expect(s.state.run.running).toBe(true));
    const c = mountComposer(s);
    fireEvent.change(c.view.container.querySelector('input[aria-label="Attach a file"]')!, {
      target: { files: [new File([new Uint8Array(4)], 'notes.txt', { type: 'text/plain' })] },
    });
    await waitFor(() => expect(transport.calls).toHaveLength(1));
    await waitFor(() => expect(c.chips()[0]?.getAttribute('data-status')).toBe('uploading'));
    expect(c.view.container.querySelector('[data-testid="send-send"]')).toBeNull();
    const stop = c.view.container.querySelector('[data-testid="send-stop"]') as HTMLButtonElement;
    expect(stop.disabled).toBe(false);
    transport.calls.shift()!.resolve();
    await waitFor(() => expect(c.chips()[0]?.getAttribute('data-status')).toBe('done'));
  });

  it('a failed upload shows the reason on the chip and a click retries', async () => {
    const transport = pendingTransport();
    setBlobTransport(transport.t);
    const s = await open(A);
    const c = mountComposer(s);
    fireEvent.change(c.view.container.querySelector('input[aria-label="Attach a file"]')!, {
      target: { files: [new File([new Uint8Array(4)], 'big.bin', { type: 'application/octet-stream' })] },
    });
    await waitFor(() => expect(transport.calls).toHaveLength(1));
    transport.calls.shift()!.reject(new Error('relay dropped'));
    // One automatic retry after 500 ms, which we fail too.
    await waitFor(() => expect(transport.calls).toHaveLength(1), { timeout: 2000 });
    transport.calls.shift()!.reject(new Error('relay dropped'));
    await waitFor(() => expect(c.chips()[0]?.getAttribute('data-status')).toBe('error'));
    expect(c.chips()[0]!.title).toContain('relay dropped');
    fireEvent.click(c.chips()[0]!);
    await waitFor(() => expect(transport.calls).toHaveLength(1));
    transport.calls.shift()!.resolve();
    await waitFor(() => expect(c.chips()[0]?.getAttribute('data-status')).toBe('done'));
  });

  it('caps the chips at 8', async () => {
    const transport = pendingTransport();
    setBlobTransport(transport.t);
    const s = await open(A);
    const c = mountComposer(s);
    const files = Array.from({ length: 10 }, (_, i) => new File([new Uint8Array(4)], `f${i}.bin`, { type: 'application/octet-stream' }));
    fireEvent.change(c.view.container.querySelector('input[aria-label="Attach a file"]')!, { target: { files } });
    await waitFor(() => expect(c.chips().length).toBe(8));
  });
});

describe('D-6: pending images are per chat', () => {
  it('attach → switch chat → back: the image chip is still there, the other chat is its own', async () => {
    const a = await open(A);
    const first = mountComposer(a);
    first.pick([pngFile('a.png')]);
    await waitFor(() => expect(first.chips()[0]?.getAttribute('data-status')).toBe('done'));
    first.view.unmount();

    // Another chat: nothing carried over.
    const b = await open(B);
    const second = mountComposer(b);
    await waitFor(() => expect(second.view.container.querySelector('[data-testid="composer-toolbar"]')).not.toBeNull());
    expect(second.chips()).toHaveLength(0);
    second.pick([pngFile('b.png')]);
    await waitFor(() => expect(second.chips()).toHaveLength(1));
    second.view.unmount();

    // Back to the first chat: its image chip survived the unmount, still one chip.
    const again = mountComposer(a);
    await waitFor(() => expect(again.chips()).toHaveLength(1));
    expect(again.chips()[0]!.getAttribute('data-status')).toBe('done');
    expect(again.chips()[0]!.querySelector('img.ichip-thumb')).not.toBeNull();
  });
});
