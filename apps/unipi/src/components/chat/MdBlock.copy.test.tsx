// @vitest-environment jsdom
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';

const copyText = vi.fn<(text: string) => Promise<boolean>>();
vi.mock('../../lib/board/share', () => ({
  openExternal: () => false,
  copyText: (text: string) => copyText(text),
}));

const openFilePath = vi.fn(async (..._a: unknown[]) => {});
vi.mock('../../lib/media/pathTokens', async (orig) => ({
  ...(await orig<typeof import('../../lib/media/pathTokens')>()),
  openFilePath: (...a: unknown[]) => openFilePath(...a),
}));

import { MdBlock } from './MdBlock';
import { renderBlocks } from '../../lib/chat/markdown';
import type { Chat } from '../../lib/chat/store';

const statPaths = async (paths: string[]) => paths.map((path) => ({ path, resolved: `/home/u/proj/${path}`, kind: path === 'src/a.ts' ? 'file' : 'missing' }));
const chat = { hostId: 'h1', statPaths, cwd: () => '/home/u/proj' } as unknown as Chat;

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('MdBlock inline-code copy (UNI-208)', () => {
  it('tapping non-path inline code copies its text and flashes .code-copied', async () => {
    copyText.mockResolvedValue(true);
    const html = renderBlocks('run `npm install` first', false).join('');
    const r = render(() => <MdBlock html={html} class="chat-block" />);
    const code = r.container.querySelector('code')!;
    fireEvent.click(code);
    expect(copyText).toHaveBeenCalledWith('npm install');
    await waitFor(() => expect(code.classList.contains('code-copied')).toBe(true));
  });

  it('tapping non-path inline code works with no chat at all (board/card call sites)', () => {
    copyText.mockResolvedValue(true);
    const html = renderBlocks('see `hello`', false).join('');
    const r = render(() => <MdBlock html={html} />);
    const code = r.container.querySelector('code')!;
    fireEvent.click(code);
    expect(copyText).toHaveBeenCalledWith('hello');
  });

  it('a fenced code block is untouched by the inline-copy behaviour', () => {
    copyText.mockResolvedValue(true);
    const html = renderBlocks('```\nconst a = 1;\n```', false).join('');
    const r = render(() => <MdBlock html={html} class="chat-block" />);
    const code = r.container.querySelector('pre code')!;
    fireEvent.click(code);
    expect(copyText).not.toHaveBeenCalledWith('const a = 1;');
  });

  it('inline code that is a verified path opens the previewer (not copy) on tap, and gets a copy button beside it', async () => {
    const html = renderBlocks('see `src/a.ts` for it', false).join('');
    const r = render(() => <MdBlock html={html} class="chat-block" chat={chat} />);
    await waitFor(() => expect(r.container.querySelector('.code-copy')).toBeTruthy());
    const code = r.container.querySelector('code')!;
    fireEvent.click(code);
    expect(openFilePath).toHaveBeenCalledWith(chat, '/home/u/proj/src/a.ts');
    expect(copyText).not.toHaveBeenCalled();
    const copyBtn = r.container.querySelector<HTMLButtonElement>('[data-copy-path]')!;
    copyText.mockResolvedValue(true);
    fireEvent.click(copyBtn);
    expect(copyText).toHaveBeenCalledWith('src/a.ts');
    expect(openFilePath).toHaveBeenCalledTimes(1); // the button tap did not also open it
  });

  it('does not preventDefault on contextmenu/touchstart for plain inline code (long-press selection stays)', () => {
    const html = renderBlocks('see `hello` text', false).join('');
    const r = render(() => <MdBlock html={html} class="chat-block" />);
    const code = r.container.querySelector('code')!;
    const ctxEvent = new MouseEvent('contextmenu', { bubbles: true, cancelable: true });
    code.dispatchEvent(ctxEvent);
    expect(ctxEvent.defaultPrevented).toBe(false);
    const touchEvent = new Event('touchstart', { bubbles: true, cancelable: true });
    code.dispatchEvent(touchEvent);
    expect(touchEvent.defaultPrevented).toBe(false);
  });
});
