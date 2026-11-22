import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, waitFor } from '@solidjs/testing-library';
import { createStore } from 'solid-js/store';
import { imagePathIn, ImageChipRow, imageRefFile, loadImage, resetImageCacheForTests } from './ImageChip';
import type { Chat } from '../../lib/chat/store';
import type { ImageRef } from '../../lib/chat/model';

function fakeChat(media: (ref: string, mime: string) => Promise<{ mime: string; dataUrl: string }>) {
  const [state, setState] = createStore({ conn: 'live' as string });
  return { chat: { state, media: vi.fn(media) } as unknown as Chat & { media: ReturnType<typeof vi.fn> }, setState };
}
const img = (n: number): ImageRef => ({ mime: 'image/png', omitted: true, bytes: 90000, mediaRef: `m1-${n}` }) as ImageRef;

describe('ImageChip thumbnails (UNI-273)', () => {
  beforeEach(() => resetImageCacheForTests());
  afterEach(() => vi.unstubAllGlobals());

  it('fetches omitted images without a tap and shows the thumbnail', async () => {
    const { chat } = fakeChat(async (ref) => ({ mime: 'image/png', dataUrl: `data:image/png;base64,${ref}` }));
    const { container } = render(() => <ImageChipRow chat={chat} images={[img(1), img(2)]} />);
    await waitFor(() => expect(container.querySelectorAll('img.chat-image-chip-img')).toHaveLength(2));
    expect(chat.media).toHaveBeenCalledTimes(2);
    expect(container.querySelector('img')!.getAttribute('src')).toBe('data:image/png;base64,m1-1');
  });

  it('waits until the chip is near the screen when IntersectionObserver exists', async () => {
    let cb: ((e: { isIntersecting: boolean }[]) => void) | undefined;
    vi.stubGlobal('IntersectionObserver', class { constructor(f: typeof cb) { cb = f; } observe() {} disconnect() {} });
    const { chat } = fakeChat(async () => ({ mime: 'image/png', dataUrl: 'data:image/png;base64,AA' }));
    const { container } = render(() => <ImageChipRow chat={chat} images={[img(3)]} />);
    await Promise.resolve();
    expect(chat.media).not.toHaveBeenCalled();
    cb!([{ isIntersecting: true }]);
    await waitFor(() => expect(container.querySelector('img.chat-image-chip-img')).not.toBeNull());
  });

  it('caches by mediaRef and limits parallel fetches to 3', async () => {
    const resolvers: (() => void)[] = [];
    const { chat } = fakeChat((ref) => new Promise((r) => resolvers.push(() => r({ mime: 'image/png', dataUrl: `data:,${ref}` }))));
    const all = [1, 2, 3, 4, 5].map((n) => loadImage(chat, img(n)));
    await Promise.resolve();
    await Promise.resolve();
    expect(chat.media).toHaveBeenCalledTimes(3);
    resolvers.shift()!();
    await waitFor(() => expect(chat.media).toHaveBeenCalledTimes(4));
    while (resolvers.length) resolvers.shift()!();
    await waitFor(() => expect(chat.media).toHaveBeenCalledTimes(5));
    resolvers.shift()?.();
    await Promise.all(all.slice(0, 4));
    expect(await loadImage(chat, img(1))).toBe('data:,m1-1');
    expect(chat.media).toHaveBeenCalledTimes(5);
  });

  it('shows the error badge on failure and retries when the chat reconnects', async () => {
    let fail = true;
    const { chat, setState } = fakeChat(async () => {
      if (fail) throw new Error('Not connected');
      return { mime: 'image/png', dataUrl: 'data:image/png;base64,BB' };
    });
    const { container } = render(() => <ImageChipRow chat={chat} images={[img(9)]} />);
    await waitFor(() => expect(container.querySelector('.chat-image-chip-error')).not.toBeNull());
    fail = false;
    setState('conn', 'closed');
    setState('conn', 'live');
    await waitFor(() => expect(container.querySelector('img.chat-image-chip-img')).not.toBeNull());
    expect(container.querySelector('.chat-image-chip-error')).toBeNull();
  });

  it('inline (small) images render immediately without fetching', () => {
    const { chat } = fakeChat(async () => ({ mime: 'image/png', dataUrl: '' }));
    const { container } = render(() => <ImageChipRow chat={chat} images={[{ mime: 'image/png', omitted: false, bytes: 10, data: 'QQ==' } as ImageRef]} />);
    expect(container.querySelector('img')!.getAttribute('src')).toBe('data:image/png;base64,QQ==');
    expect(chat.media).not.toHaveBeenCalled();
  });
});

describe('imageRefFile (UNI-290: re-attach a sent image)', () => {
  beforeEach(() => resetImageCacheForTests());
  it('an inline image becomes a File with its bytes and mime; a mediaRef is fetched first', async () => {
    const chat = { media: vi.fn(async () => ({ mime: 'image/jpeg', dataUrl: 'data:image/jpeg;base64,AQID' })) } as unknown as Chat;
    const inline = await imageRefFile(chat, { mime: 'image/png', data: 'AAEC' }, 'a.png');
    expect(inline.name).toBe('a.png');
    expect(inline.type).toBe('image/png');
    expect([...new Uint8Array(await inline.arrayBuffer())]).toEqual([0, 1, 2]);
    const fetched = await imageRefFile(chat, { mime: 'image/jpeg', mediaRef: 'm1-1' }, 'b.jpg');
    expect(chat.media).toHaveBeenCalledWith('m1-1', 'image/jpeg');
    expect([...new Uint8Array(await fetched.arrayBuffer())]).toEqual([1, 2, 3]);
  });
});

describe('F-08 screenshot path', () => {
  it('finds the saved image path in a tool result', () => {
    expect(imagePathIn('Saved: /home/me/.cache/shots/t1-1793550901511.png')).toBe('/home/me/.cache/shots/t1-1793550901511.png');
    expect(imagePathIn('…t1-1.pngSaved: /home/me/a b.png and /tmp/x.jpg.')).toBe('/tmp/x.jpg');
    expect(imagePathIn('no image here /etc/passwd')).toBeUndefined();
  });
});
