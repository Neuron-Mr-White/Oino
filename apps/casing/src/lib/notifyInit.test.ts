// @vitest-environment jsdom
/** A notification deep link opens the right pi and its header gets the real labels (seen on the
 * Redmi: it stayed "<PC> / <PC> ·" — the target was built from placeholders only). */
import { afterEach, describe, expect, it } from 'vitest';
import { findChatTarget, openChatFromDeepLink, resolveHostId } from './notifyInit';
import { closeChat, openChatState } from './chat/open';
import { setChatTransport, type ChatListData, type ChatTransport } from './chat/transport';
import type { HostInfo } from './schemas';

const host: HostInfo = { id: 'host-1', name: 'oi', ticket: 't', paired_at: 1, last_path: 'unknown' };

function listWith(pid: number): ChatListData {
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
              label: 'qachat',
              number: 1,
              focused: false,
              agent_status: 'idle',
              tab_count: 1,
              pane_count: 1,
              tabs: [
                {
                  id: 't1',
                  label: '1',
                  number: 1,
                  focused: false,
                  agent_status: 'idle',
                  panes: [{ id: 'p1', focused: false, agent: { name: 'pi', status: 'idle' }, chat: { pid, session_name: 'Fix the parser', cwd: '/tmp/qachat' } }],
                },
              ],
            },
          ],
        },
      ],
    },
  };
}

let lists = 0;
function transport(data: ChatListData): ChatTransport {
  return {
    list: async () => {
      lists++;
      return data;
    },
    open: () => ({ send: async () => {}, close: () => {} }),
  };
}

afterEach(() => {
  closeChat();
  setChatTransport(undefined);
  lists = 0;
});

describe('openChatFromDeepLink', () => {
  it('opens the pi at once, then fills in its session name and workspace from chat_list', async () => {
    setChatTransport(transport(listWith(4242)));
    openChatFromDeepLink([host], 'unipi://chat?host=host-1&pid=4242');
    expect(openChatState()?.target.pid).toBe(4242);
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));
    expect(lists).toBe(1);
    const t = openChatState()!.target;
    expect(t.title).toBe('Fix the parser');
    expect(t.workspaceLabel).toBe('qachat');
    expect(t.paneId).toBe('p1');
  });

  it('a pi not in the list keeps the placeholders (still opens)', async () => {
    setChatTransport(transport(listWith(1)));
    openChatFromDeepLink([host], 'unipi://chat?host=oi&pid=99');
    await new Promise((r) => setTimeout(r, 0));
    expect(openChatState()?.target.pid).toBe(99);
    expect(openChatState()?.target.title).toBe('oi');
  });

  it('ignores other links and unknown hosts', () => {
    openChatFromDeepLink([host], 'unipi://pair?t=x');
    expect(openChatState()).toBeNull();
    openChatFromDeepLink([], 'unipi://chat?host=x&pid=1');
    expect(openChatState()).toBeNull();
  });
});

describe('helpers', () => {
  it('findChatTarget finds the row by pid', () => {
    expect(findChatTarget('host-1', listWith(7), 7)?.title).toBe('Fix the parser');
    expect(findChatTarget('host-1', listWith(7), 8)).toBeUndefined();
    expect(findChatTarget('host-1', undefined, 7)).toBeUndefined();
  });
  it('resolveHostId: id, then name (any case), then the first host', () => {
    expect(resolveHostId([host], 'host-1')).toBe('host-1');
    expect(resolveHostId([host], 'OI')).toBe('host-1');
    expect(resolveHostId([host], undefined)).toBe('host-1');
  });
});
