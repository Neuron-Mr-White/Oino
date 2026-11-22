// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { TaskDetail } from './TaskDetail';
import type { Attachment } from '../../lib/board/api';
import { resetFileUrlCache } from '../../lib/board/api';
import { setDownloadTransport } from '../../lib/filesTransfer';
import { setPlatformOverride } from '../../lib/platform';
import { setBoardTransport } from '../../lib/board/transport';
import { fakeBoard, task } from '../../lib/board/testing';
import { resetBoardStore, boardStore } from '../../lib/board/store';
import { resetBoardUi, boardUi } from '../../lib/board/ui';
import { setHostClientFactory } from '../../lib/host/factory';
import type { HostClient } from '../../lib/host/client';
import { SettingsProvider } from '../../settings/context';
import { HostsProvider } from '../../hosts/context';

const att = (name: string, kind: Attachment['kind'], mime: string): Attachment => ({
  name: `ab12cd34-${name}`, original: name, ref: `att:A-1/ab12cd34-${name}`, path: `/x/${name}`, size: 2048, mime, kind,
  markdown: kind === 'image' ? `![${name}](att:A-1/ab12cd34-${name})` : `[${name}](att:A-1/ab12cd34-${name})`,
});

const HOME = '/home/me/.oino/state/unipi.kanboard';

async function open(files: (path: string) => unknown) {
  const row = task({ id: 'A-1', attachments: [att('shot.png', 'image', 'image/png'), att('spec.pdf', 'pdf', 'application/pdf')] });
  const fake = fakeBoard({ handlers: { 'list --all': () => ({ tasks: [row], problems: [] }), status: () => ({ ok: true, home: HOME }), 'show A-1': () => row } });
  setBoardTransport(fake.transport);
  const paths: string[] = [];
  const client = { request: vi.fn(async (op: string, args?: { path?: string }) => {
    if (op !== 'blob_get_bytes') return undefined;
    paths.push(args!.path!);
    return files(args!.path!);
  }) } as unknown as HostClient;
  setHostClientFactory(() => client);
  await boardStore().open('pc', 'demo');
  boardUi().openTask('A-1');
  render(() => <SettingsProvider><HostsProvider><TaskDetail /></HostsProvider></SettingsProvider>);
  return { paths };
}

beforeEach(() => {
  URL.createObjectURL = vi.fn(() => 'blob:tile');
});
afterEach(() => {
  cleanup(); setPlatformOverride(undefined); setDownloadTransport(undefined); setBoardTransport(undefined); setHostClientFactory(undefined); resetBoardStore(); resetBoardUi(); resetFileUrlCache();
});

describe('task attachments (OIN-92: tiles were blank)', () => {
  it('an image tile shows its thumbnail, fetched from the board home; a PDF shows its file row with name, kind and size', async () => {
    const { paths } = await open(() => ({ mime: 'image/png', bytes: [1, 2, 3] }));
    const tile = await screen.findByRole('button', { name: 'View shot.png' });
    await waitFor(() => expect(tile.querySelector('img')?.getAttribute('src')).toBe('blob:tile'));
    expect(paths).toEqual([`${HOME}/projects/demo/attachments/A-1/ab12cd34-shot.png`]);
    const pdf = screen.getByRole('button', { name: 'Open spec.pdf' });
    expect(pdf.textContent).toContain('spec.pdf');
    expect(pdf.textContent).toContain('PDF');
    expect(pdf.textContent).toContain('2 KB');
  });

  it('an image that cannot be fetched shows its name, not an endless shimmer', async () => {
    await open(() => { throw new Error('blob not found'); });
    const tile = await screen.findByRole('button', { name: 'View shot.png' });
    await waitFor(() => expect(tile.querySelector('.att-thumb-missing')).not.toBeNull());
    expect(tile.querySelector('[data-testid=att-caption]')?.textContent).toContain('shot.png');
    expect(tile.querySelector('.skeleton')).toBeNull();
  });
  it('every image tile names its file and size below the picture, so a dark or tall screenshot is never an unlabeled blank (D4)', async () => {
    await open(() => ({ mime: 'image/png', bytes: [1, 2, 3] }));
    const tile = await screen.findByRole('button', { name: 'View shot.png' });
    await waitFor(() => expect(tile.querySelector('img')).not.toBeNull());
    const caption = tile.querySelector('[data-testid=att-caption]')!;
    expect(caption.textContent).toContain('shot.png');
    expect(caption.textContent).toContain('2 KB');
    expect(tile.querySelector('.att-thumb-frame img')).not.toBeNull();
  });
  it('thumbnails come back after a reload: a fresh mount (empty caches) fetches and shows them again', async () => {
    await open(() => ({ mime: 'image/png', bytes: [1, 2, 3] }));
    const first = await screen.findByRole('button', { name: 'View shot.png' });
    await waitFor(() => expect(first.querySelector('img')).not.toBeNull());
    cleanup();
    resetFileUrlCache(); // a reload forgets the object URLs and the board home
    const { paths } = await open(() => ({ mime: 'image/png', bytes: [1, 2, 3] }));
    const again = await screen.findByRole('button', { name: 'View shot.png' });
    await waitFor(() => expect(again.querySelector('img')?.getAttribute('src')).toBe('blob:tile'));
    expect(paths).toHaveLength(1);
  });

  it('in the desktop app a PDF opens in the default app (streamed from the host), not a "saving is not available" toast', async () => {
    setPlatformOverride('tauri-desktop');
    const calls: string[] = [];
    setDownloadTransport({
      fetch: async (_host, path) => { calls.push(`fetch ${path}`); return { localPath: '/cache/spec.pdf', size: 2048, mime: 'application/pdf' }; },
      saveAs: async () => null,
      open: async (local) => { calls.push(`open ${local}`); },
    });
    await open(() => ({ mime: 'image/png', bytes: [1] }));
    fireEvent.click(await screen.findByRole('button', { name: 'Open spec.pdf' }));
    await waitFor(() => expect(calls).toEqual([`fetch ${HOME}/projects/demo/attachments/A-1/ab12cd34-spec.pdf`, 'open /cache/spec.pdf']));
    expect(boardUi().toasts().map((toast) => toast.message).join(' ')).not.toMatch(/isn't available/);
  });

  it('when the default app cannot be started the user is told, with the reason', async () => {
    setPlatformOverride('tauri-desktop');
    setDownloadTransport({
      fetch: async () => ({ localPath: '/cache/spec.pdf', size: 1, mime: 'application/pdf' }),
      saveAs: async () => null,
      open: async () => { throw new Error('no application knows how to open it'); },
    });
    await open(() => ({ mime: 'image/png', bytes: [1] }));
    fireEvent.click(await screen.findByRole('button', { name: 'Open spec.pdf' }));
    await waitFor(() => expect(boardUi().toasts().map((toast) => toast.message).join(' ')).toContain("Couldn't open spec.pdf — no application knows how to open it"));
  });
});
