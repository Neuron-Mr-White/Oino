// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
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
const queueMention = vi.fn();
vi.mock('../../lib/chat/mention', async (orig) => ({
  ...(await orig<typeof import('../../lib/chat/mention')>()),
  queueMention: (...a: unknown[]) => queueMention(...a),
}));

import { MdBlock } from './MdBlock';
import { FileRefMenu } from './FileRefMenu';
import { renderBlocks } from '../../lib/chat/markdown';
import { getEditorTarget, clearEditorTarget } from '../../lib/editorTarget';
import { consumeFilesTarget } from '../../lib/filesTarget';
import type { Chat } from '../../lib/chat/store';

const CWD = '/home/u/proj';
const EXIST: Record<string, 'file' | 'dir'> = { 'src/a.ts': 'file', 'packages/dream/': 'dir', 'Composer.tsx': 'file' };
const statPaths = vi.fn(async (paths: string[]) =>
  paths.map((path) => ({ path, resolved: `${CWD}/${path.replace(/\/$/, '')}`, kind: EXIST[path] ?? ('missing' as const) })),
);
const mkChat = () => ({ hostId: 'h1', session: '7', statPaths, cwd: () => CWD }) as unknown as Chat;

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  clearEditorTarget();
  consumeFilesTarget();
});

const mount = (md: string, chat: Chat | undefined = mkChat()) => render(() => <MdBlock html={renderBlocks(md, false).join('')} class="chat-block" chat={chat} />);

describe('MdBlock verified file refs (UNI-204)', () => {
  it('a bare relative path in prose that exists becomes tappable and opens the resolved file', async () => {
    const chat = mkChat();
    const r = mount('changed src/a.ts and src/b.ts just now', chat);
    await waitFor(() => expect(r.container.querySelector('.file-ref')).toBeTruthy());
    expect(r.container.querySelectorAll('.file-ref')).toHaveLength(1); // src/b.ts is missing on the PC
    expect(r.container.textContent?.trim()).toBe('changed src/a.ts and src/b.ts just now');
    fireEvent.click(r.container.querySelector('.file-ref')!);
    expect(openFilePath).toHaveBeenCalledWith(chat, `${CWD}/src/a.ts`);
  });

  it('path:line opens the editor at that line', async () => {
    const r = mount('see src/a.ts:42 for it');
    await waitFor(() => expect(r.container.querySelector('.file-ref')).toBeTruthy());
    expect(r.container.querySelector('.file-ref')!.textContent).toBe('src/a.ts:42');
    fireEvent.click(r.container.querySelector('.file-ref')!);
    expect(getEditorTarget()).toEqual({ hostId: 'h1', path: `${CWD}/src/a.ts`, line: 42 });
    expect(location.hash).toBe('#/files/edit');
    expect(openFilePath).not.toHaveBeenCalled();
  });

  it('a directory opens the Files screen there', async () => {
    const r = mount('new code in `packages/dream/`');
    await waitFor(() => expect(r.container.querySelector('[data-file-ref]')).toBeTruthy());
    fireEvent.click(r.container.querySelector('[data-file-ref]')!);
    expect(consumeFilesTarget()).toEqual({ hostId: 'h1', path: `${CWD}/packages/dream` });
    expect(location.hash).toBe('#/files');
  });

  it('paths in fenced code are linked too', async () => {
    const r = mount('```\n$ cat Composer.tsx\n```');
    await waitFor(() => expect(r.container.querySelector('pre .file-ref')).toBeTruthy());
    expect(r.container.querySelector('pre .file-ref')!.textContent).toBe('Composer.tsx');
  });

  it('nothing is linked without a chat (board/previewer call sites), and nothing is asked', async () => {
    const r = render(() => <MdBlock html={renderBlocks('changed src/a.ts', false).join('')} />);
    await new Promise((res) => setTimeout(res, 20));
    expect(r.container.querySelector('.file-ref')).toBeNull();
    expect(statPaths).not.toHaveBeenCalled();
  });

  it('right-click opens the file menu: Copy path, Open in Files, Mention', async () => {
    const chat = mkChat();
    const r = render(() => (
      <>
        <MdBlock html={renderBlocks('changed src/a.ts', false).join('')} class="chat-block" chat={chat} />
        <FileRefMenu />
      </>
    ));
    await waitFor(() => expect(r.container.querySelector('.file-ref')).toBeTruthy());
    fireEvent.contextMenu(r.container.querySelector('.file-ref')!);
    expect(await screen.findByRole('menu', { name: 'File actions' })).toBeTruthy();
    expect(screen.getByText('Copy path')).toBeTruthy();
    expect(screen.getByText('Open in Files')).toBeTruthy();
    fireEvent.click(screen.getByText('Add to chat'));
    expect(queueMention).toHaveBeenCalledWith('h1:7', '@src/a.ts ');
    expect(screen.queryByRole('menu', { name: 'File actions' })).toBeNull();
  });

  it('long-press (touch held 500 ms) opens the menu and the ending click does not open the file', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const r = render(() => (
        <>
          <MdBlock html={renderBlocks('changed src/a.ts', false).join('')} class="chat-block" chat={mkChat()} />
          <FileRefMenu />
        </>
      ));
      await waitFor(() => expect(r.container.querySelector('.file-ref')).toBeTruthy());
      const ref = r.container.querySelector('.file-ref')!;
      fireEvent.touchStart(ref, { touches: [{ clientX: 5, clientY: 5 }] });
      vi.advanceTimersByTime(600);
      fireEvent.touchEnd(ref);
      fireEvent.click(ref);
      expect(screen.getByRole('menu', { name: 'File actions' })).toBeTruthy();
      expect(openFilePath).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });
});
