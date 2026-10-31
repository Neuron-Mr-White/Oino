// @vitest-environment jsdom
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';

const openTaskFromChat = vi.fn();
vi.mock('../../lib/session/open', () => ({ openTaskFromChat: (...a: unknown[]) => openTaskFromChat(...a) }));
vi.mock('../../lib/board/portal', async (orig) => ({
  ...(await orig<typeof import('../../lib/board/portal')>()),
  knownPrefixes: async () => new Set(['UNI']),
}));

import { MdBlock } from './MdBlock';
import { renderBlocks } from '../../lib/chat/markdown';
import type { RefChat } from '../../lib/chat/pathStat';

const chat = { hostId: 'h1', cwd: () => '/home/u/unipi' } as unknown as RefChat;

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('task refs in chat (UNI-181)', () => {
  it('an agent message ref opens the task pane', async () => {
    const html = renderBlocks('Done with UNI-172, see `UNI-181` next. Not UTF-8.', false).join('');
    const r = render(() => <MdBlock html={html} class="chat-block" chat={chat} />);
    await waitFor(() => expect(r.container.querySelectorAll('[data-task]')).toHaveLength(2));
    fireEvent.click(r.container.querySelector('[data-task="UNI-172"]')!);
    expect(openTaskFromChat).toHaveBeenCalledWith('h1', 'UNI-172', '/home/u/unipi');
    fireEvent.click(r.container.querySelector('[data-task="UNI-181"]')!);
    expect(openTaskFromChat).toHaveBeenLastCalledWith('h1', 'UNI-181', '/home/u/unipi');
  });

  it('no refs without a chat (board/previewer call sites)', async () => {
    const html = renderBlocks('UNI-172', false).join('');
    const r = render(() => <MdBlock html={html} />);
    await new Promise((res) => setTimeout(res, 10));
    expect(r.container.querySelector('[data-task]')).toBeNull();
  });
});
