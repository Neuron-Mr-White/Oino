// @vitest-environment jsdom
/** UNI-275: chat search sheet + the entry → row lookup used by "navigate to". */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createStore } from 'solid-js/store';
import { SearchSheet } from './SearchSheet';
import { buildRows, rowIndexForEntry, toolResultOwners } from '../../lib/chat/model';
import type { Chat, SearchHit } from '../../lib/chat/store';

afterEach(() => {
  cleanup();
  document.body.innerHTML = '';
});

const hit = (id: string, role: string, snippet: string, marks: Array<[number, number]>): SearchHit => ({ id, role, snippet, marks, score: 1 });

function fakeChat(search: (q: string) => Promise<SearchHit[] | null>) {
  const [state] = createStore({ conn: 'live' });
  return { state, search: vi.fn(search) } as unknown as Chat & { search: ReturnType<typeof vi.fn> };
}

describe('SearchSheet', () => {
  it('searches as you type (debounced), marks matched words, picks a result', async () => {
    const chat = fakeChat(async () => [hit('u1', 'user', 'fix the flaky login test', [[14, 19]]), hit('t1', 'tool', 'npm test -- login', [[12, 17]])]);
    const onPick = vi.fn();
    render(() => <SearchSheet open onClose={() => {}} chat={chat} onPick={onPick} />);
    fireEvent.input(screen.getByLabelText('Search this chat'), { target: { value: 'login' } });
    const list = await waitFor(() => screen.getByTestId('search-results'), { timeout: 1500 });
    expect(chat.search).toHaveBeenCalledTimes(1);
    expect(chat.search).toHaveBeenCalledWith('login');
    expect([...list.querySelectorAll('mark')].map((m) => m.textContent)).toEqual(['login', 'login']);
    expect(list.textContent).toContain('You');
    expect(list.textContent).toContain('Ran');
    fireEvent.click(list.querySelectorAll('.search-row')[1]!);
    expect(onPick).toHaveBeenCalledWith(expect.objectContaining({ id: 't1' }));
  });

  it('no match → empty state; older pi → says so', async () => {
    const chat = fakeChat(async (q) => (q === 'zzz' ? [] : null));
    render(() => <SearchSheet open onClose={() => {}} chat={chat} onPick={() => {}} />);
    fireEvent.input(screen.getByLabelText('Search this chat'), { target: { value: 'zzz' } });
    await waitFor(() => screen.getByTestId('search-empty'), { timeout: 1500 });
    fireEvent.input(screen.getByLabelText('Search this chat'), { target: { value: 'old' } });
    await waitFor(() => expect(screen.getByText(/needs a newer unipi/)).toBeTruthy(), { timeout: 1500 });
  });
});

describe('rowIndexForEntry', () => {
  const entries = [
    { type: 'message', id: 'u1', message: { role: 'user', content: 'hi' } },
    { type: 'message', id: 'a1', message: { role: 'assistant', content: [{ type: 'text', text: 'running it' }, { type: 'toolCall', id: 'c1', name: 'bash', arguments: { command: 'ls' } }], stopReason: 'toolUse' } },
    { type: 'message', id: 'r1', message: { role: 'toolResult', toolCallId: 'c1', content: [{ type: 'text', text: 'a.txt' }] } },
    { type: 'message', id: 'a2', message: { role: 'assistant', content: [{ type: 'text', text: 'done' }], stopReason: 'stop' } },
  ] as never;
  const rows = buildRows(entries, undefined, {}, false);
  it('finds user, prose, tool-call and tool-result rows', () => {
    const owners = toolResultOwners(entries);
    expect(owners.get('r1')).toBe('a1');
    expect(rows[rowIndexForEntry(rows, 'u1')]!.kind).toBe('user');
    expect(rows[rowIndexForEntry(rows, 'a2')]!.kind).toBe('prose');
    expect(rows[rowIndexForEntry(rows, 'r1', owners)]!.key.includes('a1')).toBe(true);
    expect(rowIndexForEntry(rows, 'nope')).toBe(-1);
  });
});
