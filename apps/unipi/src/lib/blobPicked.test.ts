// @vitest-environment jsdom
/** UNI-278: a big picked file is streamed into the app cache in base64
 * chunks, then uploaded from there; small ones stay one putBytes call. */
import { afterEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a), Channel: class { onmessage?: (m: unknown) => void } }));
vi.mock('./platform', () => ({ isTauri: () => true }));

import { STREAM_UPLOAD_ABOVE, tauriBlobTransport, uploadPicked } from './blob';

afterEach(() => invoke.mockReset());

function fileOf(size: number, name = 'big.bin') {
  const bytes = new Uint8Array(size);
  for (let i = 0; i < size; i += 4096) bytes[i] = i % 251;
  return new File([bytes], name, { type: 'application/zip' });
}

describe('streamed upload of a picked file', () => {
  it('stages in ≤ 4 MiB chunks, uploads the staged path, drops it, reports progress to 100%', async () => {
    const staged: number[] = [];
    invoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
      if (cmd === 'upload_stage_begin') return '/cache/upload-staging/1-big.bin';
      if (cmd === 'upload_stage_chunk') {
        staged.push(atob(String(args.data)).length);
        return staged.reduce((a, b) => a + b, 0);
      }
      if (cmd === 'blob_put_file') {
        expect(args.path).toBe('/cache/upload-staging/1-big.bin');
        expect(args.mime).toBe('application/zip');
        (args.onProgress as { onmessage?: (m: unknown) => void }).onmessage?.({ kind: 'progress', sent: 5_000_000, total: 5_000_000 });
        return { sha256: 'a'.repeat(64), size: 5_000_000, deduped: false, path: '/x/blobs/a' };
      }
      if (cmd === 'upload_stage_drop') return null;
      throw new Error(cmd);
    });
    const seen: number[] = [];
    const put = await tauriBlobTransport.putPicked!('h', fileOf(5_000_000), (p) => seen.push(p.sent / p.total));
    expect(put.path).toBe('/x/blobs/a');
    expect(staged).toEqual([4 * 1024 * 1024, 5_000_000 - 4 * 1024 * 1024]);
    expect(seen.at(-1)).toBeCloseTo(1, 5);
    expect(seen.every((v, i) => i === 0 || v >= seen[i - 1]!)).toBe(true);
    expect(invoke.mock.calls.at(-1)?.[0]).toBe('upload_stage_drop');
    expect(invoke.mock.calls.some((c) => c[0] === 'blob_put_bytes')).toBe(false);
  });

  it('a failed upload still drops the staged file', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'upload_stage_begin') return '/cache/upload-staging/2-x';
      if (cmd === 'upload_stage_chunk') return 1;
      if (cmd === 'blob_put_file') throw { code: 'E_TRANSPORT', message: 'cut off' };
      return null;
    });
    await expect(tauriBlobTransport.putPicked!('h', fileOf(10))).rejects.toMatchObject({ code: 'E_TRANSPORT' });
    await new Promise((r) => setTimeout(r, 0));
    expect(invoke.mock.calls.at(-1)?.[0]).toBe('upload_stage_drop');
  });

  it('uploadPicked: small files stay one putBytes; big ones stream', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'upload_stage_begin' ? '/s' : cmd === 'upload_stage_chunk' ? 1 : { sha256: 'b'.repeat(64), size: 1, deduped: false, path: '/p' }));
    await uploadPicked('h', fileOf(1000, 'small.txt'));
    expect(invoke.mock.calls.map((c) => c[0])).toEqual(['blob_put_bytes']);
    invoke.mockClear();
    await uploadPicked('h', fileOf(STREAM_UPLOAD_ABOVE + 1));
    expect(invoke.mock.calls[0]?.[0]).toBe('upload_stage_begin');
    expect(invoke.mock.calls.some((c) => c[0] === 'blob_put_bytes')).toBe(false);
  });
});
