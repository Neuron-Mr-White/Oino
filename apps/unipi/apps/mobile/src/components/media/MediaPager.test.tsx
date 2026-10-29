// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { MediaPager } from './MediaPager';
import { setBlobTransport, type BlobTransport } from '../../lib/blob';
import type { PreviewRequest } from '../../lib/media/previewerState';

function transportFor(mimes: Record<string, string>) {
  const fetched: string[] = [];
  const transport = {
    async putFile() { throw new Error('unused'); },
    async putBytes() { throw new Error('unused'); },
    async getFile() { throw new Error('unused'); },
    async getBytes(_h: string, ref: { path: string }) {
      const path = ref.path;
      fetched.push(path);
      const mime = mimes[path.split('/').pop() ?? ''] ?? 'application/octet-stream';
      return { mime, bytes: new Uint8Array([104, 105]) };
    },
  } as unknown as BlobTransport;
  return { transport, fetched };
}

afterEach(() => {
  cleanup();
  setBlobTransport(undefined);
  vi.clearAllMocks();
});

function renderPager(paths: string[], index: number, mimes: Record<string, string>) {
  const { transport, fetched } = transportFor(mimes);
  setBlobTransport(transport);
  const rendered: string[] = [];
  const r = render(() => (
    <MediaPager
      hostId="h1"
      paths={paths}
      index={index}
      onClose={() => {}}
      render={(req: PreviewRequest) => {
        rendered.push(req.kind === 'image' ? `image:${req.items[0]?.name}` : req.kind === 'list' ? 'list' : `${req.kind}:${req.name}`);
        return <div data-testid="pager-body">{rendered.at(-1)}</div>;
      }}
    />
  ));
  return { r, rendered, fetched };
}

describe('MediaPager (UNI-168)', () => {
  it('loads the current file and pages across kinds with the chevrons', async () => {
    const { r, rendered, fetched } = renderPager(
      ['/f/a.png', '/f/b.mp4', '/f/c.pdf'],
      0,
      { 'a.png': 'image/png', 'b.mp4': 'video/mp4', 'c.pdf': 'application/pdf' },
    );
    await waitFor(() => expect(r.getByTestId('media-pager-pos').textContent).toBe('1 / 3'));
    expect(rendered[0]).toBe('image:a.png');
    fireEvent.click(screen.getByRole('button', { name: 'Next file' }));
    await waitFor(() => expect(r.getByTestId('media-pager-pos').textContent).toBe('2 / 3'));
    await waitFor(() => expect(rendered[1]).toBe('video:b.mp4'));
    fireEvent.click(screen.getByRole('button', { name: 'Next file' }));
    await waitFor(() => expect(rendered[2]).toBe('pdf:c.pdf'));
    expect(fetched).toEqual(['/f/a.png', '/f/b.mp4', '/f/c.pdf']);
    // Back to the first, and Previous is disabled at the start.
    fireEvent.click(screen.getByRole('button', { name: 'Previous file' }));
    await waitFor(() => expect(r.getByTestId('media-pager-pos').textContent).toBe('2 / 3'));
  });

  it('disables prev at the start and next at the end', async () => {
    const { r } = renderPager(['/f/a.png', '/f/b.png'], 0, { 'a.png': 'image/png', 'b.png': 'image/png' });
    await waitFor(() => expect(r.getByTestId('pager-body')).toBeTruthy());
    expect((screen.getByRole('button', { name: 'Previous file' }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole('button', { name: 'Next file' }) as HTMLButtonElement).disabled).toBe(false);
  });

  it('shows one item without paging controls', async () => {
    const { r } = renderPager(['/f/a.png'], 0, { 'a.png': 'image/png' });
    await waitFor(() => expect(r.getByTestId('pager-body')).toBeTruthy());
    expect(r.queryByTestId('media-pager-pos')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Next file' })).toBeNull();
  });
});

describe('MediaPager — files too large to preview (UNI-225)', () => {
  it('shows the file page (Download / Open with…) instead of an error', async () => {
    setBlobTransport({
      ...transportFor({}).transport,
      async getBytes() {
        throw { code: 'E_UNSUPPORTED', message: 'That file is too large to preview inline', retryable: false };
      },
    } as BlobTransport);
    const seen: PreviewRequest[] = [];
    render(() => (
      <MediaPager hostId="h1" paths={['/v/huge.mp4']} index={0} onClose={() => {}} render={(req) => (seen.push(req), <div data-testid="pager-body" />)} />
    ));
    await screen.findByTestId('pager-body');
    expect(screen.queryByRole('alert')).toBeNull();
    expect(seen.at(-1)).toMatchObject({ kind: 'other', name: 'huge.mp4', reason: 'too-large', source: { hostId: 'h1', path: '/v/huge.mp4' } });
  });

  it('other load errors still show the error', async () => {
    setBlobTransport({
      ...transportFor({}).transport,
      async getBytes() {
        throw { code: 'E_TRANSPORT', message: 'PC gone' };
      },
    } as BlobTransport);
    render(() => <MediaPager hostId="h1" paths={['/a.png']} index={0} onClose={() => {}} render={() => <div />} />);
    expect((await screen.findByRole('alert')).textContent).toBe('PC gone');
  });
});
