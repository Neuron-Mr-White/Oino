// @vitest-environment jsdom
/**
 * UNI-260 / WP-A1: New chat sheet (PC → folder → first message → start over
 * the host's `session_new`) and the pure helpers behind it. The herdr/tmux
 * "Where" step and the Close chat sheet are gone (D-14).
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const call = vi.fn();
vi.mock('../../lib/ipc', () => ({ call: (...a: unknown[]) => call(...a) }));

import { NewChatSheet } from './NewChatSheet';
import { errorWords, folderChoices, folderName, hereCwd, hideChatFromList, hideKey, newChatHere, newSession, pruneHidden, readHidden, shortPath, unhideChat } from '../../lib/session/newSession';
import { setChatTransport } from '../../lib/session/link';
import { chatLists } from '../../lib/session/list';

function mockHost(opts: { newSession?: unknown } = {}) {
  call.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
    if (cmd === 'fs_roots') return { ok: true, value: { roots: ['/home/u', '/tmp'] } };
    if (cmd === 'fs_list') return { ok: true, value: { entries: [{ name: 'proj', kind: 'dir', size: 0, mtime: 0, ignored: false }, { name: '.git', kind: 'dir', size: 0, mtime: 0, ignored: false }, { name: 'a.txt', kind: 'file', size: 1, mtime: 0, ignored: false }], more: false } };
    if (cmd === 'session_new') return opts.newSession ?? { ok: true, value: { session_id: 'new-1', cwd: args.cwd, run_state: 'idle' } };
    return { ok: true, value: null };
  });
}

beforeEach(() => {
  call.mockReset();
  localStorage.clear();
});
afterEach(() => {
  cleanup();
  setChatTransport(undefined);
  document.body.innerHTML = '';
});

describe('new session helpers', () => {
  it('folder chips: session folders first, then roots; distinct; home dropped unless alone', () => {
    expect(folderChoices([{ cwd: '/home/u/api' }, { cwd: '/home/u/api' }, { cwd: '/srv/x' }], ['/home/u', '/tmp'], '/home/u')).toEqual(['/home/u/api', '/srv/x', '/tmp']);
    expect(folderChoices([], ['/home/u'], '/home/u')).toEqual(['/home/u']);
  });

  it('names and paths', () => {
    expect(folderName('/home/u/proj/')).toBe('proj');
    expect(folderName('~')).toBe('session');
    expect(shortPath('/home/u/proj', '/home/u')).toBe('~/proj');
    expect(shortPath('/home/bob/x')).toBe('~/x');
    expect(hereCwd('~/proj')).toBeUndefined();
    expect(hereCwd('/home/u/proj')).toBe('/home/u/proj');
    expect(hereCwd('C:\\work')).toBe('C:\\work');
  });

  it('hide per PC (by session id), unhide, prune', () => {
    hideChatFromList('h1', { session: 's1' });
    hideChatFromList('h1', { session: 's2' });
    expect(readHidden('h1')).toEqual(['s1', 's2']);
    expect(readHidden('h2')).toEqual([]);
    expect(hideKey({ session: 's1' })).toBe('s1');
    unhideChat('h1', 's1');
    expect(readHidden('h1')).toEqual(['s2']);
    pruneHidden('h1', [{ session: 's9' }]);
    expect(readHidden('h1')).toEqual([]);
  });

  it('newSession asks the host for session_new and returns the row', async () => {
    mockHost();
    const res = await newSession('h1', '/home/u/proj');
    expect(call).toHaveBeenCalledWith('session_new', { hostId: 'h1', cwd: '/home/u/proj' }, expect.anything());
    expect(res.ok && res.value).toMatchObject({ hostId: 'h1', session: 'new-1', title: 'proj', status: 'idle' });
  });

  it('newChatHere passes only real host paths as the folder', async () => {
    mockHost();
    await newChatHere({ hostId: 'h1', cwd: '~/proj' });
    expect(call).toHaveBeenLastCalledWith('session_new', { hostId: 'h1' }, expect.anything());
    await newChatHere({ hostId: 'h1', cwdFull: '/home/u/proj', cwd: '~/proj' });
    expect(call).toHaveBeenLastCalledWith('session_new', { hostId: 'h1', cwd: '/home/u/proj' }, expect.anything());
  });

  it('errorWords: an older host says so; anything else passes through', () => {
    expect(errorWords({ code: 'E_UNSUPPORTED', message: 'x', retryable: false }).text).toMatch(/older UniPi host/);
    expect(errorWords({ code: 'E_INTERNAL', message: 'boom', hint: 'h', retryable: false })).toEqual({ text: 'boom', hint: 'h' });
  });
});

describe('NewChatSheet', () => {
  const pcs = [{ id: 'h1', name: 'Laptop' }];

  it('defaults to the first session folder and starts the session with the first message', async () => {
    mockHost();
    setChatTransport({ list: async () => ({ sessions: [{ session_id: 'a', cwd: '/home/u/api', run_state: 'idle' as const }] }), open: () => ({ send: async () => {}, close: () => {} }) });
    await chatLists().load('h1');
    const onStarted = vi.fn();
    render(() => <NewChatSheet open onClose={() => {}} pcs={pcs} hostId="h1" onStarted={onStarted} />);
    await waitFor(() => expect(screen.getByRole('button', { name: /~\/api/ }).getAttribute('aria-pressed')).toBe('true'));
    expect(screen.queryByText('Where')).toBeNull();
    fireEvent.input(screen.getByLabelText(/First message/), { target: { value: 'fix the tests' } });
    fireEvent.click(screen.getByTestId('new-chat-start'));
    await waitFor(() => expect(onStarted).toHaveBeenCalled());
    expect(call).toHaveBeenCalledWith('session_new', { hostId: 'h1', cwd: '/home/u/api' }, expect.anything());
    expect(onStarted.mock.calls[0]![0]).toMatchObject({ hostId: 'h1', pcName: 'Laptop', first: 'fix the tests', target: { session: 'new-1', title: 'api' } });
  });

  it('browsing picks a folder (dirs only, no dotfiles)', async () => {
    mockHost();
    render(() => <NewChatSheet open onClose={() => {}} pcs={pcs} hostId="h1" onStarted={() => {}} />);
    await waitFor(() => expect(screen.getByText('Browse…')).toBeTruthy());
    fireEvent.click(screen.getByText('Browse…'));
    await waitFor(() => expect(screen.getByText('proj')).toBeTruthy());
    expect(screen.queryByText('.git')).toBeNull();
    expect(screen.queryByText('a.txt')).toBeNull();
    fireEvent.click(screen.getByText('proj'));
    await waitFor(() => expect(call).toHaveBeenCalledWith('fs_list', expect.objectContaining({ path: expect.stringMatching(/\/proj$/) }), expect.anything()));
    fireEvent.click(screen.getByTestId('new-chat-use-folder'));
    await waitFor(() => expect(screen.getByRole('button', { name: /proj$/ }).getAttribute('aria-pressed')).toBe('true'));
  });

  it('shows the host error and keeps the form', async () => {
    mockHost({ newSession: { ok: false, error: { code: 'E_INTERNAL', message: 'the session could not start', hint: 'Check the host log.', retryable: true } } });
    const onStarted = vi.fn();
    render(() => <NewChatSheet open onClose={() => {}} pcs={pcs} hostId="h1" onStarted={onStarted} />);
    await waitFor(() => expect((screen.getByTestId('new-chat-start') as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(screen.getByTestId('new-chat-start'));
    await waitFor(() => expect(screen.getByText(/could not start/)).toBeTruthy());
    expect(screen.getByText(/Check the host log/)).toBeTruthy();
    expect(onStarted).not.toHaveBeenCalled();
  });

  it('several PCs: asks which first', async () => {
    mockHost();
    render(() => <NewChatSheet open onClose={() => {}} pcs={[...pcs, { id: 'h2', name: 'Desk' }]} onStarted={() => {}} />);
    expect(screen.getByText('Which PC should run it?')).toBeTruthy();
    fireEvent.click(screen.getByText('Desk'));
    await waitFor(() => expect(screen.getByText('Change')).toBeTruthy());
    expect(call).toHaveBeenCalledWith('fs_roots', { hostId: 'h2' }, expect.anything());
  });
});
