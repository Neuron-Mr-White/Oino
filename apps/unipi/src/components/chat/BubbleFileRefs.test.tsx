// @vitest-environment jsdom
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../lib/board/portal', async (orig) => ({
  ...(await orig<typeof import('../../lib/board/portal')>()),
  knownPrefixes: async () => new Set(['UNI']),
}));
const openFilePath = vi.fn(async (..._a: unknown[]) => {});
vi.mock('../../lib/media/pathTokens', async (orig) => ({
  ...(await orig<typeof import('../../lib/media/pathTokens')>()),
  openFilePath: (...a: unknown[]) => openFilePath(...a),
}));
vi.mock('../../lib/media/previewerState', async (orig) => ({
  ...(await orig<typeof import('../../lib/media/previewerState')>()),
  openPreviewer: (...a: unknown[]) => openPreviewer(...a),
}));

import { UserBubble, mentionTokens } from './Transcript';
import type { Chat } from '../../lib/chat/store';

const loadImage = vi.fn(async (_chat: Chat, image: { data?: string; mime: string }) => `data:${image.mime};base64,${image.data ?? 'eA=='}`);
const openPreviewer = vi.fn();
vi.mock('./ImageChip', async (orig) => ({
  ...(await orig<typeof import('./ImageChip')>()),
  loadImage: (...a: Parameters<typeof loadImage>) => loadImage(...a),
}));

const CWD = '/home/u/proj';
const EXIST: Record<string, 'file' | 'dir'> = { 'src/a.ts': 'file', 'docs/': 'dir', 'Composer.tsx': 'file' };
const mkChat = () =>
  ({
    hostId: 'h1',
    session: '1',
    state: { session: { cwd: CWD }, commands: [] },
    statPaths: vi.fn(async (paths: string[]) => paths.map((path) => ({ path, resolved: `${CWD}/${path.replace(/\/$/, '')}`, kind: EXIST[path] ?? 'missing' }))),
  }) as unknown as Chat;

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  loadImage.mockClear();
});

describe('user bubble file refs (UNI-204)', () => {
  it('a verified @path becomes a path chip; an unverified one stays plain text', async () => {
    const chat = mkChat();
    const r = render(() => <UserBubble chat={chat} text="look at @src/a.ts and @src/nope.ts please" images={[]} />);
    await waitFor(() => expect(r.container.querySelector('.ichip[data-kind=path]')).toBeTruthy());
    expect(r.container.querySelectorAll('.ichip')).toHaveLength(1);
    expect(r.container.querySelector('.ichip-label')!.textContent).toBe('a.ts');
    expect(r.container.textContent).toContain('@src/nope.ts');
    fireEvent.click(r.container.querySelector('.ichip')!);
    await waitFor(() => expect(openFilePath).toHaveBeenCalledWith(chat, `${CWD}/src/a.ts`));
  });

  it('bare paths in the bubble text become links when they exist', async () => {
    const chat = mkChat();
    const r = render(() => <UserBubble chat={chat} text="why does Composer.tsx crash? and/or Foo.tsx" images={[]} />);
    await waitFor(() => expect(r.container.querySelector('.file-ref')).toBeTruthy());
    expect([...r.container.querySelectorAll('.file-ref')].map((e) => e.textContent)).toEqual(['Composer.tsx']);
    expect(r.container.textContent).toBe('why does Composer.tsx crash? and/or Foo.tsx');
  });

  it('task refs still work next to file refs', async () => {
    const r = render(() => <UserBubble chat={mkChat()} text="UNI-204 is about src/a.ts" images={[]} />);
    await waitFor(() => expect(r.container.querySelector('.task-ref')).toBeTruthy());
    await waitFor(() => expect(r.container.querySelector('.file-ref')).toBeTruthy());
    expect(r.container.textContent).toBe('UNI-204 is about src/a.ts');
  });

  it('mentionTokens finds @tokens, quoted ones, not emails or attachment drafts', () => {
    expect(mentionTokens('see @src/a.ts @"my dir/x.md" me@x.com @att:abc')).toEqual(['@src/a.ts', '@"my dir/x.md"']);
  });
});

describe('`[image N]` chips in a sent bubble (UNI-285)', () => {
  const image = (n: number) => ({ mime: 'image/png', data: `ZGF0YQ==`.repeat(n) });

  it('renders `[image N]` as a chip that opens that image in the previewer', async () => {
    const chat = mkChat();
    const images = [image(1), image(2)];
    const r = render(() => <UserBubble chat={chat} text="compare [image 1] with [image 2]" images={images} />);
    await waitFor(() => expect(r.container.querySelectorAll('.ichip[data-kind=image]')).toHaveLength(2));
    expect([...r.container.querySelectorAll('.ichip[data-kind=image] .ichip-label')].map((e) => e.textContent)).toEqual(['image 1', 'image 2']);
    expect(r.container.textContent).toBe('compare image 1 with image 2');
    fireEvent.click(r.container.querySelectorAll('.ichip[data-kind=image]')[1]!);
    await waitFor(() => expect(openPreviewer).toHaveBeenCalled());
    // The second image: index 1, both siblings resolved for swipe.
    const call = vi.mocked(openPreviewer).mock.calls[0]![0] as { index: number; items: { url: string }[] };
    expect(call.index).toBe(1);
    expect(call.items.map((i) => i.url)).toEqual(['data:image/png;base64,ZGF0YQ==', 'data:image/png;base64,ZGF0YQ==ZGF0YQ==']);
    // Both thumbs are rendered next to the bubble as before.
    await waitFor(() => expect(r.container.querySelectorAll('.chat-image-chip')).toHaveLength(2));
  });

  it('an out-of-range `[image 3]` opens nothing', async () => {
    const chat = mkChat();
    const r = render(() => <UserBubble chat={chat} text="only [image 3]" images={[image(1)]} />);
    await waitFor(() => expect(r.container.querySelector('.ichip[data-kind=image]')).toBeTruthy());
    fireEvent.click(r.container.querySelector('.ichip[data-kind=image]')!);
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(openPreviewer).not.toHaveBeenCalled();
  });
});
