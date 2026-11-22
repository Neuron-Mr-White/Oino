// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from '@solidjs/testing-library';
import { afterEach, describe, expect, it } from 'vitest';
import { TaskDetail } from './TaskDetail';
import { setBoardTransport } from '../../lib/board/transport';
import { fakeBoard, task } from '../../lib/board/testing';
import { resetBoardStore, boardStore } from '../../lib/board/store';
import { resetBoardUi, boardUi } from '../../lib/board/ui';
import type { Activity, Task } from '../../lib/board/api';
import { SettingsProvider } from '../../settings/context';
import { HostsProvider } from '../../hosts/context';

afterEach(() => { cleanup(); setBoardTransport(undefined); resetBoardStore(); resetBoardUi(); });

const ago = (ms: number) => new Date(Date.now() - ms).toISOString();

/** A one-task board whose `note` appends to the task's activity like the CLI does (actor `user`). */
async function open(status: Task['status'], activity: Activity[] = []) {
  let current = task({ id: 'C-1', status, activity });
  const fake = fakeBoard({
    handlers: {
      'list --all': () => ({ tasks: [current], problems: [] }),
      'show C-1': () => current,
      'note C-1': ({ args }) => {
        current = { ...current, activity: [...current.activity, { at: new Date().toISOString(), actor: 'user', text: args.at(-1)! }] };
        return current;
      },
    },
  });
  setBoardTransport(fake.transport);
  await boardStore().open('pc', 'demo');
  boardUi().openTask('C-1');
  render(() => <SettingsProvider><HostsProvider><TaskDetail /></HostsProvider></SettingsProvider>);
  await screen.findByRole('list', { name: 'Activity' });
  return fake;
}

describe('TaskDetail comments (OIN-45)', () => {
  it.each(['todo', 'in_progress', 'done'] as const)('a %s task has an "Add a note" field that posts a note with the typed text', async (status) => {
    const fake = await open(status);
    const field = screen.getByLabelText('Add a note') as HTMLTextAreaElement;
    const send = screen.getByRole('button', { name: 'Send note' });
    expect(send).toBeDisabled();
    fireEvent.input(field, { target: { value: 'Use **the cache**, not a copy' } });
    expect(send).toBeEnabled();
    fireEvent.click(send);
    await waitFor(() => expect(fake.sent('note C-1')).toHaveLength(1));
    expect(fake.sent('note C-1')[0]!.args).toEqual(['note', 'C-1', '--', 'Use **the cache**, not a copy']);
    await waitFor(() => expect(field.value).toBe(''));
    // The comment lands in the activity: author, relative time, markdown.
    const timeline = screen.getByRole('list', { name: 'Activity' });
    await waitFor(() => expect(within(timeline).getByText('the cache').tagName).toBe('STRONG'));
    expect(within(timeline).getByText('user')).toBeInTheDocument();
    expect(within(timeline).getByText('just now')).toBeInTheDocument();
  });

  it('Ctrl+Enter in the field sends the note', async () => {
    const fake = await open('in_progress');
    fireEvent.input(screen.getByLabelText('Add a note'), { target: { value: 'quick one' } });
    fireEvent.keyDown(screen.getByLabelText('Add a note'), { key: 'Enter', ctrlKey: true });
    await waitFor(() => expect(fake.sent('note C-1')).toHaveLength(1));
  });

  it('comments by the owner and an agent list newest first with author, relative time and markdown', async () => {
    await open('in_progress', [
      { at: ago(3 * 3_600_000), actor: 'agent', text: 'Started on the `parser`' },
      { at: ago(5 * 60_000), actor: 'user', text: 'Looks _good_' },
    ]);
    const entries = within(screen.getByRole('list', { name: 'Activity' })).getAllByRole('listitem');
    expect(entries).toHaveLength(2);
    expect(within(entries[0]!).getByText('user')).toBeInTheDocument();
    expect(within(entries[0]!).getByText('5m ago')).toBeInTheDocument();
    expect(within(entries[0]!).getByText('good').tagName).toBe('EM');
    expect(within(entries[1]!).getByText('agent')).toBeInTheDocument();
    expect(within(entries[1]!).getByText('3h ago')).toBeInTheDocument();
    expect(within(entries[1]!).getByText('parser').tagName).toBe('CODE');
  });

  it('a blocked task keeps a Comment button next to the answer action', async () => {
    const fake = await open('blocked');
    fireEvent.input(screen.getByLabelText('Add a note'), { target: { value: 'FYI only' } });
    fireEvent.click(screen.getByRole('button', { name: /Comment/ }));
    await waitFor(() => expect(fake.sent('note C-1')).toHaveLength(1));
    expect(fake.sent('move C-1')).toHaveLength(0);
  });
});
