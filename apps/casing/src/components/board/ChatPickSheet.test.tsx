// @vitest-environment jsdom
/** Board → chat picker (UNI-185). */
import { render, screen, waitFor, cleanup } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ChatPickSheet } from './ChatPickSheet';
import { setChatTransport, type ChatListData } from '../../lib/chat/transport';
import { rankChatsForProject } from '../../lib/board/portal';

function listing(panes: Array<{ id: string; cwd: string }>): ChatListData {
  return {
    herdr: {
      available: true,
      sessions: [
        {
          name: 'default',
          running: true,
          default: true,
          workspaces: [
            {
              id: 'w1',
              label: 'main',
              number: 1,
              focused: true,
              agent_status: 'idle',
              tab_count: 1,
              pane_count: panes.length,
              tabs: [
                {
                  id: 'w1:t1',
                  label: 't',
                  number: 1,
                  focused: true,
                  agent_status: 'idle',
                  panes: panes.map((p, i) => ({
                    id: p.id,
                    focused: false,
                    agent: { name: 'pi', status: 'idle' as const },
                    chat: { pid: 100 + i, session_name: p.id, cwd: p.cwd },
                  })),
                },
              ],
            },
          ],
        },
      ],
    },
  } as unknown as ChatListData;
}

afterEach(() => {
  cleanup();
  setChatTransport(undefined);
});

describe('rankChatsForProject', () => {
  it('chats under the project root come first; ~ and absolute forms match', () => {
    const ranked = rankChatsForProject([{ cwd: '~/other' }, { cwd: '~/proj/sub' }, { cwd: '~/proj' }, { cwd: '~/projx' }], '/home/u/proj');
    expect(ranked.map((c) => [c.cwd, c.inProject])).toEqual([
      ['~/proj/sub', true],
      ['~/proj', true],
      ['~/other', false],
      ['~/projx', false],
    ]);
  });
});

describe('ChatPickSheet', () => {
  it('asks which chat when several exist, project chats first', async () => {
    setChatTransport({ list: async () => listing([{ id: 'elsewhere', cwd: '/home/u/other' }, { id: 'a', cwd: '/home/u/proj' }, { id: 'b', cwd: '/home/u/proj/app' }]) } as never);
    const onPick = vi.fn();
    render(() => <ChatPickSheet open hostId="host-pick-1" root="/home/u/proj" label="Go to a chat" onPick={onPick} onClose={() => {}} />);
    await waitFor(() => expect(screen.getAllByTestId('chat-pick-row').length).toBe(3));
    const titles = screen.getAllByTestId('chat-pick-row').map((r) => r.querySelector('.truncate')?.textContent);
    expect(titles).toEqual(['a', 'b', 'elsewhere']);
    expect(screen.getByText('In this project')).toBeTruthy();
    expect(screen.getByText('Other chats')).toBeTruthy();
    expect(onPick).not.toHaveBeenCalled();
    screen.getAllByTestId('chat-pick-row')[1]!.click();
    expect(onPick.mock.calls[0]![0].title).toBe('b');
  });

  it('a single chat opens straight away', async () => {
    setChatTransport({ list: async () => listing([{ id: 'only', cwd: '/home/u/proj' }]) } as never);
    const onPick = vi.fn();
    render(() => <ChatPickSheet open hostId="host-pick-2" root="/home/u/proj" label="Go to a chat" onPick={onPick} onClose={() => {}} />);
    await waitFor(() => expect(onPick).toHaveBeenCalledTimes(1));
    expect(onPick.mock.calls[0]![0].title).toBe('only');
  });
});
