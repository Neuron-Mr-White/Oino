/**
 * Composer attachments (WP-A13 acceptance): the Z-60 upload state machine
 * (`queued → uploading % → ready`, one automatic retry, manual retry, max 8,
 * the send gate), the Z-60 concurrency limit, the unipi file mentions, and
 * D-6 — pending images live in a **per-chat** store, so switching chats and
 * coming back keeps them and two chats never share one.
 *
 * The image path's shrink/encode is mocked (jsdom has no canvas); the file
 * path runs `uploadPicked` against a fake blob transport.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setBlobTransport, type BlobPutResult, type BlobTransport } from '../blob';

vi.mock('../imageShrink', () => ({
  MAX_SIDE: 1568,
  QUALITY: 0.8,
  INLINE_MAX_BYTES: 700 * 1024,
  shrinkImage: async (file: Blob) => ({ blob: file, mime: 'image/png', width: 100, height: 80 }),
  blobToBase64: async () => 'QUJD',
}));

import {
  attach,
  attachmentsGate,
  attachmentsReady,
  attachmentRoom,
  addReadyImage,
  clearAttachments,
  drainAttachments,
  formatFileMention,
  MAX_ATTACHMENTS,
  pendingAttachmentsFor,
  PROGRESS_CEILING,
  readyImages,
  removeAttachment,
  replaceImage,
  resetAttachments,
  retryAttachment,
  withAttachmentMentions,
} from './attachments';

const fakeFile = (name: string, size = 1024, type = 'application/octet-stream') => {
  const file = new File([new Uint8Array(Math.min(size, 8))], name, { type });
  Object.defineProperty(file, 'size', { value: size });
  return file;
};
const png = (name = 'shot.png') => new File([new Uint8Array(8)], name, { type: 'image/png' });

/** A blob transport that resolves on demand (or after `delay`) and reports progress. */
function transport(opts: { delay?: number; fail?: (call: number) => boolean } = {}) {
  const calls: Array<{ name: string; progress: (p: { sent: number; total: number }) => void; resolve: () => void; reject: (e: Error) => void }> = [];
  let live = 0;
  let maxLive = 0;
  const t: BlobTransport = {
    async putFile(): Promise<BlobPutResult> {
      throw new Error('not used');
    },
    async putBytes(hostId, _bytes, mime, name, onProgress): Promise<BlobPutResult> {
      const call = calls.length + 1;
      live += 1;
      maxLive = Math.max(maxLive, live);
      onProgress?.({ sent: 1, total: 3 });
      const result = await new Promise<BlobPutResult>((resolve, reject) => {
        calls.push({
          name: name ?? 'file',
          progress: (p) => onProgress?.(p),
          resolve: () => resolve({ sha256: 'sha', size: 3, deduped: false, path: `/blobs/${name}` }),
          reject: (e) => reject(e),
        });
      }).finally(() => {
        live -= 1;
      });
      void hostId;
      void mime;
      void call;
      return result;
    },
    async getFile() {
      throw new Error('not used');
    },
    async getBytes() {
      throw new Error('not used');
    },
  };
  return { t, calls, maxLive: () => maxLive, fail: opts.fail, delay: opts.delay };
}

/** Runs the pending upload jobs the way the transport would, in order. */
async function settle(chatKey: string, harness: ReturnType<typeof transport>) {
  for (let i = 0; i < 100; i++) {
    const call = harness.calls.shift();
    if (call) {
      call.progress({ sent: 2, total: 3 });
      call.resolve();
    }
    await drainAttachments(chatKey);
    if (!harness.calls.length) break;
  }
  await drainAttachments(chatKey);
}

beforeEach(() => {
  resetAttachments();
});

afterEach(() => {
  resetAttachments();
  setBlobTransport(undefined);
  vi.useRealTimers();
});

describe('attachment states (Z-60)', () => {
  it('queued → uploading with monotonic progress → ready at 100 %', async () => {
    const h = transport();
    setBlobTransport(h.t);
    const { added, refused, overLimit } = attach('chat-a', 'pc', [fakeFile('notes.txt', 3)]);
    expect(refused).toEqual([]);
    expect(overLimit).toBe(0);
    // The job may already have started (the store item is live); the poll
    // below records the transitions either way.
    expect(['queued', 'uploading']).toContain(pendingAttachmentsFor('chat-a')[0]!.status);
    const seen: number[] = [];
    const poll = setInterval(() => seen.push(pendingAttachmentsFor('chat-a')[0]!.progress), 1);
    await new Promise((r) => setTimeout(r, 5));
    expect(pendingAttachmentsFor('chat-a')[0]!.status).toBe('uploading');
    await settle('chat-a', h);
    clearInterval(poll);
    const done = pendingAttachmentsFor('chat-a')[0]!;
    expect(done.status).toBe('ready');
    expect(done.progress).toBe(1);
    expect(done.path).toBe('/blobs/notes.txt');
    for (let i = 1; i < seen.length; i++) expect(seen[i]!).toBeGreaterThanOrEqual(seen[i - 1]!);
    expect(Math.max(...seen.filter((p) => p < 1))).toBeLessThanOrEqual(PROGRESS_CEILING); // capped until commit
  });

  it('at most two uploads in flight (Z-60)', async () => {
    const h = transport();
    setBlobTransport(h.t);
    attach('chat-a', 'pc', [fakeFile('a.txt'), fakeFile('b.txt'), fakeFile('c.txt')]);
    await new Promise((r) => setTimeout(r, 10));
    expect(h.calls.length).toBe(2);
    expect(h.maxLive()).toBe(2);
    await settle('chat-a', h);
    expect(pendingAttachmentsFor('chat-a').every((a) => a.status === 'ready')).toBe(true);
  });

  it('fails, retries once automatically, then waits for the user', async () => {
    vi.useFakeTimers();
    const h = transport();
    setBlobTransport(h.t);
    attach('chat-a', 'pc', [fakeFile('a.txt')]);
    await vi.advanceTimersByTimeAsync(1);
    h.calls.shift()!.reject(new Error('relay dropped'));
    await vi.advanceTimersByTimeAsync(1);
    // auto retry is queued, not failed yet
    expect(pendingAttachmentsFor('chat-a')[0]!.autoRetries).toBe(1);
    expect(pendingAttachmentsFor('chat-a')[0]!.status).toBe('queued');
    await vi.advanceTimersByTimeAsync(600); // 500 ms backoff
    expect(pendingAttachmentsFor('chat-a')[0]!.status).toBe('uploading');
    h.calls.shift()!.reject(new Error('relay dropped again'));
    await vi.advanceTimersByTimeAsync(1);
    const failed = pendingAttachmentsFor('chat-a')[0]!;
    expect(failed.status).toBe('failed');
    expect(failed.error).toBe('relay dropped again');
    expect(attachmentsReady('chat-a')).toBe(false);
  });

  it('a manual retry starts a fresh budget and recovers', async () => {
    vi.useFakeTimers();
    const h = transport();
    setBlobTransport(h.t);
    attach('chat-a', 'pc', [fakeFile('a.txt')]);
    await vi.advanceTimersByTimeAsync(1);
    h.calls.shift()!.reject(new Error('once'));
    await vi.advanceTimersByTimeAsync(600);
    h.calls.shift()!.reject(new Error('twice'));
    await vi.advanceTimersByTimeAsync(1);
    expect(pendingAttachmentsFor('chat-a')[0]!.status).toBe('failed');
    retryAttachment('chat-a', 'pc', pendingAttachmentsFor('chat-a')[0]!.id);
    await vi.advanceTimersByTimeAsync(1);
    h.calls.shift()!.resolve();
    await vi.advanceTimersByTimeAsync(1);
    expect(pendingAttachmentsFor('chat-a')[0]!.status).toBe('ready');
  });

  it('caps a chat at 20 attachments (Z-60)', () => {
    setBlobTransport(transport().t);
    const files = Array.from({ length: 22 }, (_, i) => fakeFile(`f${i}.txt`));
    const { added, overLimit } = attach('chat-a', 'pc', files);
    expect(MAX_ATTACHMENTS).toBe(20);
    expect(added).toHaveLength(20);
    expect(overLimit).toBe(2);
    expect(attachmentRoom('chat-a')).toBe(0);
    expect(attach('chat-a', 'pc', [fakeFile('x.txt')]).overLimit).toBe(1);
  });

  it('the send gate is closed while anything uploads or has failed', async () => {
    const h = transport();
    setBlobTransport(h.t);
    expect(attachmentsReady('chat-a')).toBe(true);
    attach('chat-a', 'pc', [fakeFile('a.txt')]);
    await new Promise((r) => setTimeout(r, 5));
    expect(attachmentsGate('chat-a')).toEqual({ ok: false, uploading: 1, failed: 0 });
    await settle('chat-a', h);
    expect(attachmentsGate('chat-a')).toEqual({ ok: true, uploading: 0, failed: 0 });
    attach('chat-a', 'pc', [fakeFile('huge.bin', 2 * 1024 * 1024 * 1024)]);
    expect(attachmentsGate('chat-a').failed).toBe(1);
    expect(attachmentsReady('chat-a')).toBe(false);
  });

  it('a ready image carries its base64 bytes; a remove drops the chip', () => {
    const image = addReadyImage('chat-a', { mime: 'image/png', data: 'QUJD' })!;
    expect(image.status).toBe('ready');
    expect(image.preview).toBe('data:image/png;base64,QUJD');
    expect(readyImages('chat-a')).toEqual([{ mimeType: 'image/png', data: 'QUJD' }]);
    removeAttachment('chat-a', image.id);
    expect(readyImages('chat-a')).toEqual([]);
  });

  it('an annotated image replaces its bytes in place (D-5)', async () => {
    const image = addReadyImage('chat-a', { mime: 'image/png', data: 'OLD=' })!;
    replaceImage('chat-a', image.id, new Blob([new Uint8Array(4)], { type: 'image/png' }));
    await drainAttachments('chat-a');
    const after = pendingAttachmentsFor('chat-a')[0]!;
    expect(after.id).toBe(image.id);
    expect(after.status).toBe('ready');
    expect(after.data).toBe('QUJD');
  });
});

describe('file mentions (unipi wire)', () => {
  it('appends one mention line per ready file only', () => {
    expect(formatFileMention({ name: 'a.txt', path: '/blobs/aa' })).toBe('[file: a.txt]@/blobs/aa');
    expect(withAttachmentMentions('chat-a', 'hi')).toBe('hi');
    addReadyImage('chat-a', { mime: 'image/png', data: 'QUJD' });
    expect(withAttachmentMentions('chat-a', 'hi')).toBe('hi'); // images travel as image parts
  });
});

describe('D-6: pending attachments are per chat', () => {
  it('switching chats keeps each chat’s images, and they never mix', () => {
    addReadyImage('pc/one', { mime: 'image/png', data: 'AAA=' });
    addReadyImage('pc/two', { mime: 'image/png', data: 'BBB=' });
    addReadyImage('pc/two', { mime: 'image/png', data: 'CCC=' });
    // back to the first chat: still exactly its own image
    expect(readyImages('pc/one')).toEqual([{ mimeType: 'image/png', data: 'AAA=' }]);
    expect(readyImages('pc/two')).toHaveLength(2);
    clearAttachments('pc/two');
    expect(readyImages('pc/two')).toEqual([]);
    expect(readyImages('pc/one')).toHaveLength(1);
  });
});
