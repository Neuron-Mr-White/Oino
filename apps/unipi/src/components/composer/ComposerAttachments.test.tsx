// @vitest-environment jsdom
/**
 * Composer attachments (WP-A13 acceptance): the chips carry the Z-60 states,
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
  const chips = () => [...view.container.querySelectorAll('[data-testid="attachment-chip"]')] as HTMLElement[];
  return { view, pick, chips };
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

describe('attachment chips (Z-60)', () => {
  it('an attached image becomes a ready chip with its thumbnail', async () => {
    const s = await open(A);
    const c = mountComposer(s);
    c.pick([pngFile()]);
    await waitFor(() => expect(c.chips()).toHaveLength(1));
    await waitFor(() => expect(c.chips()[0]!.getAttribute('data-status')).toBe('ready'));
    const chip = c.chips()[0]!;
    expect(chip.querySelector('img')?.getAttribute('src')).toBe('data:image/png;base64,QUJD');
    expect(chip.textContent).toContain('shot.png');
  });

  it('Send stays disabled until the attachment is ready, then sends its bytes', async () => {
    const transport = pendingTransport();
    setBlobTransport(transport.t);
    const s = await open(A);
    const c = mountComposer(s);
    // Type something so Send would otherwise be enabled.
    const field = c.view.container.querySelector('.chip-field') as HTMLDivElement;
    field.focus();
    field.append(document.createTextNode('look at this'));
    fireEvent.input(field);
    fireEvent.change(c.view.container.querySelector('input[aria-label="Attach a file"]')!, {
      target: { files: [new File([new Uint8Array(4)], 'notes.txt', { type: 'text/plain' })] },
    });
    await waitFor(() => expect(transport.calls).toHaveLength(1));
    expect(c.chips()[0]?.getAttribute('data-status')).toBe('uploading');
    expect((c.view.container.querySelector('[data-testid="send-send"]') as HTMLButtonElement).disabled).toBe(true);

    transport.calls[0]!.resolve();
    await waitFor(() => expect(c.chips()[0]?.getAttribute('data-status')).toBe('ready'));
    const send = c.view.container.querySelector('[data-testid="send-send"]') as HTMLButtonElement;
    expect(send.disabled).toBe(false);
    fireEvent.click(send);
    await waitFor(() => expect(world.log.some((e) => e.kind === 'cmd' && e.op === 'prompt')).toBe(true));
    const prompt = world.log.find((e) => e.kind === 'cmd' && e.op === 'prompt')!.args as { message: string };
    expect(prompt.message).toBe('look at this\n\n[file: notes.txt]@/blobs/notes.txt');
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
    expect(c.chips()[0]?.getAttribute('data-status')).toBe('uploading');
    expect(c.view.container.querySelector('[data-testid="send-send"]')).toBeNull();
    const stop = c.view.container.querySelector('[data-testid="send-stop"]') as HTMLButtonElement;
    expect(stop.disabled).toBe(false);
    transport.calls.shift()!.resolve();
    await waitFor(() => expect(c.chips()[0]?.getAttribute('data-status')).toBe('ready'));
  });

  it('a failed upload shows the reason and a Retry that recovers', async () => {
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
    await waitFor(() => expect(c.chips()[0]?.getAttribute('data-status')).toBe('failed'));
    expect(c.view.container.querySelector('[data-testid="attachment-state"]')?.textContent).toContain('relay dropped');
    fireEvent.click(c.view.container.querySelector('[data-testid="attachment-retry"]')!);
    await waitFor(() => expect(transport.calls).toHaveLength(1));
    transport.calls.shift()!.resolve();
    await waitFor(() => expect(c.chips()[0]?.getAttribute('data-status')).toBe('ready'));
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
  it('attach → switch chat → back: the image is still there, the other chat is its own', async () => {
    const a = await open(A);
    const first = mountComposer(a);
    first.pick([pngFile('a.png')]);
    await waitFor(() => expect(first.chips()[0]?.getAttribute('data-status')).toBe('ready'));
    first.view.unmount();

    // Another chat: nothing carried over.
    const b = await open(B);
    const second = mountComposer(b);
    await waitFor(() => expect(second.view.container.querySelector('[data-testid="composer-toolbar"]')).not.toBeNull());
    expect(second.chips()).toHaveLength(0);
    second.pick([pngFile('b.png')]);
    await waitFor(() => expect(second.chips()).toHaveLength(1));
    expect(second.chips()[0]!.textContent).toContain('b.png');
    second.view.unmount();

    // Back to the first chat: its image survived the unmount.
    const again = mountComposer(a);
    await waitFor(() => expect(again.chips()).toHaveLength(1));
    expect(again.chips()[0]!.textContent).toContain('a.png');
    expect(again.chips()[0]!.getAttribute('data-status')).toBe('ready');
  });
});
