import { afterEach, describe, expect, it, vi } from 'vitest';
import { downloadToDevice, openWithExternal, progressLabel, setDownloadTransport, transferMessage, type DownloadTransport } from './filesTransfer';
import { setPlatformOverride } from './platform';
import { setBlobTransport, type BlobTransport } from './blob';

function fakeTransport(mime = 'application/vnd.openxmlformats-officedocument.wordprocessingml.document') {
  const calls: string[] = [];
  const transport: BlobTransport = {
    async putFile() {
      throw new Error('unused');
    },
    async putBytes() {
      throw new Error('unused');
    },
    async getFile() {
      throw new Error('unused');
    },
    async getBytes(_hostId, ref) {
      calls.push('path' in ref ? ref.path : `sha:${ref.sha256}`);
      return { mime, bytes: new Uint8Array([104, 105]) }; // "hi"
    },
  };
  return { transport, calls };
}

type Bridge = { saveDownload: (n: string, m: string, b: string) => string; openWith: (n: string, m: string, b: string) => string };

function installBridge(bridge: Partial<Bridge> | undefined): Bridge | undefined {
  const w = window as unknown as { UniPiFiles?: Bridge };
  delete w.UniPiFiles;
  if (bridge) w.UniPiFiles = bridge as Bridge;
  return w.UniPiFiles;
}

afterEach(() => {
  installBridge(undefined);
  setBlobTransport(undefined);
  setDownloadTransport(undefined);
  setPlatformOverride(undefined);
});

describe('filesTransfer (UNI-169)', () => {
  it('downloads through the native bridge with base64 bytes', async () => {
    const { transport, calls } = fakeTransport();
    setBlobTransport(transport);
    const seen: Array<[string, string, string]> = [];
    installBridge({
      saveDownload: (name, mime, b64) => {
        seen.push([name, mime, b64]);
        return '{"ok":true,"uri":"content://downloads/1"}';
      },
    });
    const result = await downloadToDevice('h1', '/home/u/report.docx');
    expect(result).toEqual({ outcome: 'saved', detail: 'content://downloads/1' });
    expect(calls).toEqual(['/home/u/report.docx']);
    expect(seen[0]).toEqual(['report.docx', 'application/vnd.openxmlformats-officedocument.wordprocessingml.document', btoa('hi')]);
  });

  it('opens with another app through the chooser', async () => {
    const { transport } = fakeTransport('application/zip');
    setBlobTransport(transport);
    installBridge({ openWith: () => '{"ok":true}' });
    const result = await openWithExternal('h1', '/tmp/a.zip');
    expect(result.outcome).toBe('opened');
  });

  it('reports unsupported without the Android bridge', async () => {
    const { transport } = fakeTransport();
    setBlobTransport(transport);
    installBridge(undefined);
    expect((await downloadToDevice('h1', '/a.docx')).outcome).toBe('unsupported');
    expect((await openWithExternal('h1', '/a.docx')).outcome).toBe('unsupported');
  });

  it('surfaces a native error (e.g. no app can open it)', async () => {
    const { transport } = fakeTransport();
    setBlobTransport(transport);
    installBridge({ openWith: () => '{"ok":false,"error":"no app can open a.docx"}' });
    const result = await openWithExternal('h1', '/a.docx');
    expect(result).toEqual({ outcome: 'failed', detail: 'no app can open a.docx' });
  });

  it('surfaces a fetch failure', async () => {
    setBlobTransport({
      ...fakeTransport().transport,
      async getBytes() {
        throw new Error('host gone');
      },
    } as BlobTransport);
    installBridge({ openWith: () => '{"ok":true}' });
    const result = await openWithExternal('h1', '/a.docx');
    expect(result.outcome).toBe('failed');
    expect(result.detail).toContain('host gone');
  });
});

/** UNI-225: Download streams into the cache (any size) and hands the file off; never the 25 MB preview read. */
describe('streamed download (UNI-225)', () => {
  function fakeDownload(over: Partial<DownloadTransport> = {}) {
    const calls: string[] = [];
    const t: DownloadTransport = {
      async fetch(_h, path, onProgress) {
        calls.push(`fetch ${path}`);
        onProgress?.({ done: 40 * 1024 ** 2, total: 80 * 1024 ** 2 });
        onProgress?.({ done: 80 * 1024 ** 2, total: 80 * 1024 ** 2 });
        return { localPath: '/cache/pushes/dl-1/big.zip', size: 80 * 1024 ** 2, mime: 'application/zip' };
      },
      async saveAs(local, name) {
        calls.push(`saveAs ${local} ${name}`);
        return '/home/u/Downloads/big.zip';
      },
      async open(local) {
        calls.push(`open ${local}`);
      },
      ...over,
    };
    return { t, calls };
  }
  const noPreviewRead = () => {
    setBlobTransport({
      ...fakeTransport().transport,
      async getBytes() {
        throw { code: 'E_UNSUPPORTED', message: 'That file is too large to preview inline' };
      },
    } as BlobTransport);
  };

  it('Android: a big file is fetched to the cache, then saved by the bridge (no in-memory read)', async () => {
    setPlatformOverride('tauri-mobile');
    noPreviewRead();
    const { t, calls } = fakeDownload();
    setDownloadTransport(t);
    const saveFile = vi.fn(() => '{"ok":true,"uri":"content://downloads/9"}');
    const saveDownload = vi.fn(() => '{"ok":true}');
    installBridge({ saveFile, saveDownload, openWith: () => '{"ok":true}' } as never);
    const progress: number[] = [];
    const result = await downloadToDevice('h1', '/data/big.zip', (p) => progress.push(p.done));
    expect(result).toEqual({ outcome: 'saved', detail: 'content://downloads/9' });
    expect(calls).toEqual(['fetch /data/big.zip']);
    expect(saveFile).toHaveBeenCalledWith('/cache/pushes/dl-1/big.zip', 'big.zip', 'application/zip');
    expect(saveDownload).not.toHaveBeenCalled();
    expect(progress).toEqual([40 * 1024 ** 2, 80 * 1024 ** 2]);
  });

  it('Android: Open with… goes through openFile', async () => {
    setPlatformOverride('tauri-mobile');
    const { t } = fakeDownload();
    setDownloadTransport(t);
    const openFile = vi.fn(() => '{"ok":true}');
    installBridge({ openFile, saveDownload: () => '', openWith: () => '' } as never);
    expect((await openWithExternal('h1', '/data/big.zip')).outcome).toBe('opened');
    expect(openFile).toHaveBeenCalledWith('/cache/pushes/dl-1/big.zip', 'big.zip', 'application/zip');
  });

  it('desktop: Save dialog (saved / cancelled) and Open', async () => {
    setPlatformOverride('tauri-desktop');
    const { t, calls } = fakeDownload();
    setDownloadTransport(t);
    expect(await downloadToDevice('h1', '/data/big.zip')).toEqual({ outcome: 'saved', detail: '/home/u/Downloads/big.zip' });
    expect(calls).toContain('saveAs /cache/pushes/dl-1/big.zip big.zip');
    setDownloadTransport(fakeDownload({ saveAs: async () => null }).t);
    expect((await downloadToDevice('h1', '/data/big.zip')).outcome).toBe('cancelled');
    const opened = fakeDownload();
    setDownloadTransport(opened.t);
    expect((await openWithExternal('h1', '/data/big.zip')).outcome).toBe('opened');
    expect(opened.calls).toContain('open /cache/pushes/dl-1/big.zip');
  });

  it('a failed fetch is reported, never as "preview not available"', async () => {
    setPlatformOverride('tauri-desktop');
    setDownloadTransport(fakeDownload({ fetch: async () => { throw { code: 'E_TRANSPORT', message: 'The download was cut off' }; } }).t);
    const result = await downloadToDevice('h1', '/data/big.zip');
    expect(result).toEqual({ outcome: 'failed', detail: 'The download was cut off' });
  });

  it('an old APK bridge (no saveFile) falls back to the in-memory path', async () => {
    setPlatformOverride('tauri-mobile');
    const { transport } = fakeTransport('text/plain');
    setBlobTransport(transport);
    setDownloadTransport(fakeDownload().t);
    const saveDownload = vi.fn(() => '{"ok":true}');
    installBridge({ saveDownload, openWith: () => '' });
    expect((await downloadToDevice('h1', '/a.txt')).outcome).toBe('saved');
    expect(saveDownload).toHaveBeenCalled();
  });

  it('progress and result wording', () => {
    expect(progressLabel('Downloading', undefined)).toBe('Downloading…');
    expect(progressLabel('Downloading', { done: 512, total: 1024 })).toBe('Downloading… 50%');
    expect(progressLabel('Downloading', { done: 12 * 1024 ** 2, total: 80 * 1024 ** 2 })).toBe('Downloading… 15% · 12.0 of 80.0 MB');
    expect(transferMessage({ outcome: 'saved' }, 'a.zip', false)).toBe('Saved to Download/unipi/a.zip');
    expect(transferMessage({ outcome: 'saved', detail: '/home/u/Downloads/a.zip' }, 'a.zip', true)).toBe('Saved to /home/u/Downloads/a.zip');
    expect(transferMessage({ outcome: 'cancelled' }, 'a.zip', true)).toBe('Download cancelled');
  });
});
