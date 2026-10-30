import { describe, it, expect, beforeEach } from 'vitest';
import { resolvePromptImages, uploadGenericFile, MAX_GENERIC_FILE_BYTES, type PendingImage } from './attachments';
import { setBlobTransport, type BlobTransport } from '../blob';

function pending(name: string, bytes: number, mime = 'image/webp'): PendingImage {
  const blob = new Blob([new Uint8Array(bytes)], { type: mime });
  return { id: name, name, mime, blob, previewUrl: 'blob:fake' };
}

describe('resolvePromptImages', () => {
  let putCalls: Array<{ mime: string; size: number }> = [];
  const fakeTransport: BlobTransport = {
    async putFile() {
      throw new Error('not used');
    },
    async putBytes(_hostId, bytes, mime) {
      putCalls.push({ mime, size: bytes.length });
      return { sha256: 'deadbeef', size: bytes.length, deduped: false, path: `/blobs/${mime.replace('/', '-')}` };
    },
    async getFile() {
      throw new Error('not used');
    },
    async getBytes() {
      throw new Error('not used');
    },
  };

  beforeEach(() => {
    putCalls = [];
    setBlobTransport(fakeTransport);
  });

  it('small images combined go inline as base64', async () => {
    const images = [pending('a.webp', 100), pending('b.webp', 200)];
    const out = await resolvePromptImages('host1', images);
    expect(out).toHaveLength(2);
    expect(out.every((i) => 'data' in i)).toBe(true);
    expect(putCalls).toHaveLength(0);
  });

  it('big combined total uploads every image via the blob channel', async () => {
    const images = [pending('a.webp', 500 * 1024), pending('b.webp', 400 * 1024)];
    const out = await resolvePromptImages('host1', images);
    expect(out).toHaveLength(2);
    expect(out.every((i) => 'path' in i)).toBe(true);
    expect(putCalls).toHaveLength(2);
  });

  it('a single image right at the inline cap stays inline', async () => {
    const images = [pending('a.webp', 700 * 1024)];
    const out = await resolvePromptImages('host1', images);
    expect(out).toHaveLength(1);
    expect('data' in out[0]!).toBe(true);
  });

  it('empty input resolves to an empty array', async () => {
    expect(await resolvePromptImages('host1', [])).toEqual([]);
  });
});

describe('uploadGenericFile', () => {
  let putCalls: Array<{ mime: string; size: number; name?: string }> = [];
  const fakeTransport: BlobTransport = {
    async putFile() {
      throw new Error('not used');
    },
    async putBytes(_hostId, bytes, mime, name) {
      putCalls.push({ mime, size: bytes.length, name });
      return { sha256: 'deadbeef', size: bytes.length, deduped: false, path: `/home/u/.unipi/app-host/blobs/deadbeef` };
    },
    async getFile() {
      throw new Error('not used');
    },
    async getBytes() {
      throw new Error('not used');
    },
  };

  beforeEach(() => {
    putCalls = [];
    setBlobTransport(fakeTransport);
  });

  it('uploads the file and returns its host path', async () => {
    const file = new File([new Uint8Array(1000)], 'notes.pdf', { type: 'application/pdf' });
    const path = await uploadGenericFile('host1', file);
    expect(path).toBe('/home/u/.unipi/app-host/blobs/deadbeef');
    expect(putCalls).toEqual([{ mime: 'application/pdf', size: 1000, name: 'notes.pdf' }]);
  });

  it('defaults the mime when the File has none', async () => {
    const file = new File([new Uint8Array(10)], 'blob', { type: '' });
    await uploadGenericFile('host1', file);
    expect(putCalls[0]!.mime).toBe('application/octet-stream');
  });

  it('rejects a file over the 1 GB cap without uploading', async () => {
    const file = new File([new Uint8Array(4)], 'big.bin');
    Object.defineProperty(file, 'size', { value: MAX_GENERIC_FILE_BYTES + 1 });
    await expect(uploadGenericFile('host1', file)).rejects.toThrow(/larger than 1 GB/);
    expect(putCalls).toHaveLength(0);
  });
});
