// @vitest-environment jsdom
/**
 * UserBubble coverage ported from components/chat (WP-A6): file refs
 * (UNI-204), task refs (UNI-181), `[image N]` chips (UNI-285), mention
 * tokens — now against the `RefChat` adapter shape instead of the v1 Chat.
 */
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';

const openTaskFromChat = vi.fn();
vi.mock('../../lib/session/open', () => ({ openTaskFromChat: (...a: unknown[]) => openTaskFromChat(...a) }));
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

import { mentionTokens, UserBubble } from './UserBubble';
import type { RefChat } from '../../lib/chat/pathStat';
import type { ImageRef } from '../../lib/chat/model';

const loadImage = vi.fn(async (_chat: RefChat, image: { data?: string; mime: string }) => `data:${image.mime};base64,${image.data ?? 'eA=='}`);
const openPreviewer = vi.fn();
vi.mock('../chat/ImageChip', async (orig) => ({
  ...(await orig<typeof import('../chat/ImageChip')>()),
  loadImage: (...a: Parameters<typeof loadImage>) => loadImage(...a),
}));

const CWD = '/home/u/proj';
const EXIST: Record<string, 'file' | 'dir'> = { 'src/a.ts': 'file', 'docs/': 'dir', 'Composer.tsx': 'file' };
const commands = () => new Set<string>();
const mkChat = () =>
  ({
    hostId: 'h1',
    session: '1',
    cwd: () => CWD,
    statPaths: vi.fn(async (paths: string[]) => paths.map((path) => ({ path, resolved: `${CWD}/${path.replace(/\/$/, '')}`, kind: EXIST[path] ?? 'missing' }))),
  }) as unknown as RefChat;

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  loadImage.mockClear();
});

describe('user bubble file refs (UNI-204)', () => {
  it('a verified @path becomes a path chip; an unverified one stays plain text', async () => {
    const chat = mkChat();
    const r = render(() => <UserBubble chat={chat} text="look at @src/a.ts and @src/nope.ts please" commands={commands} />);
    await waitFor(() => expect(r.container.querySelector('.ichip[data-kind=path]')).toBeTruthy());
    expect(r.container.querySelectorAll('.ichip')).toHaveLength(1);
    expect(r.container.querySelector('.ichip-label')!.textContent).toBe('a.ts');
    expect(r.container.textContent).toContain('@src/nope.ts');
    fireEvent.click(r.container.querySelector('.ichip')!);
    await waitFor(() => expect(openFilePath).toHaveBeenCalledWith(chat, `${CWD}/src/a.ts`));
  });

  it('bare paths in the bubble text become links when they exist', async () => {
    const r = render(() => <UserBubble chat={mkChat()} text="why does Composer.tsx crash? and/or Foo.tsx" commands={commands} />);
    await waitFor(() => expect(r.container.querySelector('.file-ref')).toBeTruthy());
    expect([...r.container.querySelectorAll('.file-ref')].map((e) => e.textContent)).toEqual(['Composer.tsx']);
    expect(r.container.textContent).toBe('why does Composer.tsx crash? and/or Foo.tsx');
  });

  it('task refs still work next to file refs', async () => {
    const r = render(() => <UserBubble chat={mkChat()} text="UNI-204 is about src/a.ts" commands={commands} />);
    await waitFor(() => expect(r.container.querySelector('.task-ref')).toBeTruthy());
    await waitFor(() => expect(r.container.querySelector('.file-ref')).toBeTruthy());
    expect(r.container.textContent).toBe('UNI-204 is about src/a.ts');
  });

  it('the user message task ref opens the task pane', async () => {
    const r = render(() => <UserBubble chat={mkChat()} text="please do UNI-176 then UNI-177" commands={commands} />);
    await waitFor(() => expect(r.container.querySelectorAll('.task-ref')).toHaveLength(2));
    fireEvent.click(r.getByText('UNI-177'));
    expect(openTaskFromChat).toHaveBeenCalledWith('h1', 'UNI-177', CWD);
  });

  it('mentionTokens finds @tokens, quoted ones, not emails or attachment drafts', () => {
    expect(mentionTokens('see @src/a.ts @"my dir/x.md" me@x.com @att:abc')).toEqual(['@src/a.ts', '@"my dir/x.md"']);
  });
});

describe('`[image N]` chips in a sent bubble (UNI-285)', () => {
  const image = (n: number): ImageRef => ({ mime: 'image/png', data: `ZGF0YQ==`.repeat(n) });

  it('renders `[image N]` as a chip that opens that image in the previewer', async () => {
    const images = [image(1), image(2)];
    const r = render(() => <UserBubble chat={mkChat()} text="compare [image 1] with [image 2]" images={images} commands={commands} />);
    await waitFor(() => expect(r.container.querySelectorAll('.ichip[data-kind=image]')).toHaveLength(2));
    expect([...r.container.querySelectorAll('.ichip[data-kind=image] .ichip-label')].map((e) => e.textContent)).toEqual(['image 1', 'image 2']);
    fireEvent.click(r.container.querySelectorAll('.ichip[data-kind=image]')[1]!);
    await waitFor(() => expect(openPreviewer).toHaveBeenCalled());
    const call = vi.mocked(openPreviewer).mock.calls[0]![0] as { index: number; items: { url: string }[] };
    expect(call.index).toBe(1);
    expect(call.items.map((i) => i.url)).toEqual(['data:image/png;base64,ZGF0YQ==', 'data:image/png;base64,ZGF0YQ==ZGF0YQ==']);
    await waitFor(() => expect(r.container.querySelectorAll('.chat-image-chip')).toHaveLength(2));
  });

  it('an out-of-range `[image 3]` opens nothing', async () => {
    const r = render(() => <UserBubble chat={mkChat()} text="only [image 3]" images={[image(1)]} commands={commands} />);
    await waitFor(() => expect(r.container.querySelector('.ichip[data-kind=image]')).toBeTruthy());
    fireEvent.click(r.container.querySelector('.ichip[data-kind=image]')!);
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(openPreviewer).not.toHaveBeenCalled();
  });
});
