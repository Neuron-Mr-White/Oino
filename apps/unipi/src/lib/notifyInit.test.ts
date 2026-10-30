// @vitest-environment jsdom
/** A notification deep link opens the right session and its header gets the real labels (seen on
 * the Redmi: it stayed "<PC> / <PC> ·" — the target was built from placeholders only). */
import { afterEach, describe, expect, it } from 'vitest';
import { findChatTarget, openChatFromDeepLink, resolveHostId } from './notifyInit';
import { closeChat, openChatState } from './session/open';
import { setChatTransport, type SessionListData, type ChatTransport } from './session/link';
import type { HostInfo } from './schemas';

const host: HostInfo = { id: 'host-1', name: 'oi', ticket: 't', paired_at: 1, last_path: 'unknown' };

function listWith(session: string): SessionListData {
  return { sessions: [{ session_id: session, name: 'Fix the parser', cwd: '/tmp/qachat', run_state: 'idle' }] };
}

let lists = 0;
function transport(data: SessionListData): ChatTransport {
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
  it('opens the session at once, then fills in its name and folder from the session list', async () => {
    setChatTransport(transport(listWith('s-4242')));
    openChatFromDeepLink([host], 'unipi://chat?host=host-1&session=s-4242');
    expect(openChatState()?.target.session).toBe('s-4242');
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));
    expect(lists).toBe(1);
    const t = openChatState()!.target;
    expect(t.title).toBe('Fix the parser');
    expect(t.cwd).toBe('/tmp/qachat');
  });

  it('a session not in the list keeps the placeholders (still opens)', async () => {
    setChatTransport(transport(listWith('s1')));
    openChatFromDeepLink([host], 'unipi://chat?host=oi&session=s99');
    await new Promise((r) => setTimeout(r, 0));
    expect(openChatState()?.target.session).toBe('s99');
    expect(openChatState()?.target.title).toBe('oi');
  });

  it('ignores other links and unknown hosts', () => {
    openChatFromDeepLink([host], 'unipi://pair?t=x');
    expect(openChatState()).toBeNull();
    openChatFromDeepLink([], 'unipi://chat?host=x&session=s1');
    expect(openChatState()).toBeNull();
    // A v1 link (bridge pid only) has no session to open.
    openChatFromDeepLink([host], 'unipi://chat?host=host-1&pid=4242');
    expect(openChatState()).toBeNull();
  });
});

describe('helpers', () => {
  it('findChatTarget finds the row by session id', () => {
    expect(findChatTarget('host-1', listWith('s7'), 's7')?.title).toBe('Fix the parser');
    expect(findChatTarget('host-1', listWith('s7'), 's8')).toBeUndefined();
    expect(findChatTarget('host-1', undefined, 's7')).toBeUndefined();
  });
  it('resolveHostId: id, then name (any case), then the first host', () => {
    expect(resolveHostId([host], 'host-1')).toBe('host-1');
    expect(resolveHostId([host], 'OI')).toBe('host-1');
    expect(resolveHostId([host], undefined)).toBe('host-1');
  });
});
