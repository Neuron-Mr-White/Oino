// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it } from 'vitest';
import { TaskDetail } from './TaskDetail';
import { setBoardTransport } from '../../lib/board/transport';
import { fakeBoard, refused, task } from '../../lib/board/testing';
import { resetBoardStore, boardStore } from '../../lib/board/store';
import { resetBoardUi, boardUi } from '../../lib/board/ui';
import { SettingsProvider } from '../../settings/context';
import { HostsProvider } from '../../hosts/context';

const fullText = 'The complete activity report that is not present in the list row.';
const trimmedReport = 'The start of the report that the brief list row carried.';
const fullReport = 'The complete report the show call returns, beyond the trimmed list text.';
afterEach(() => { cleanup(); setBoardTransport(undefined); resetBoardStore(); resetBoardUi(); });

describe('TaskDetail trimmed activity', () => {
  it('fetches one full show task and renders its full activity text', async () => {
    const row = task({ id: 'A-1', activity: [{ at: new Date().toISOString(), actor: 'agent', text: 'truncated…' }], activityTrimmed: true });
    const fake = fakeBoard({ handlers: { 'list --all': () => ({ tasks: [row], problems: [] }), 'show A-1': () => task({ ...row, activity: [{ at: new Date().toISOString(), actor: 'agent', text: fullText }], activityTrimmed: undefined }) } });
    setBoardTransport(fake.transport);
    const store = boardStore();
    await store.open('pc', 'demo');
    boardUi().openTask('A-1');
    render(() => <SettingsProvider><HostsProvider><TaskDetail /></HostsProvider></SettingsProvider>);
    await waitFor(() => expect(screen.getByText(fullText)).toBeTruthy());
    expect(fake.calls.filter((call) => call.args.join(' ') === 'show A-1')).toHaveLength(1);
  });

  it('fetches the full report when only the status report was trimmed', async () => {
    const row = task({ id: 'A-2', status: 'in_review', statusReport: { kind: 'review', text: trimmedReport, actor: 'agent', at: new Date().toISOString() }, statusReportTrimmed: true });
    const fake = fakeBoard({ handlers: { 'list --all': () => ({ tasks: [row], problems: [] }), 'show A-2': () => task({ ...row, statusReport: { kind: 'review', text: fullReport, actor: 'agent', at: row.statusReport!.at }, statusReportTrimmed: undefined }) } });
    setBoardTransport(fake.transport);
    const store = boardStore();
    await store.open('pc', 'demo');
    boardUi().openTask('A-2');
    render(() => <SettingsProvider><HostsProvider><TaskDetail /></HostsProvider></SettingsProvider>);
    await waitFor(() => expect(screen.getByText(fullReport)).toBeTruthy());
    expect(fake.calls.filter((call) => call.args.join(' ') === 'show A-2')).toHaveLength(1);
  });

  it('keeps the trimmed activity and retries a failed full fetch', async () => {
    const row = task({ id: 'A-3', activity: [{ at: new Date().toISOString(), actor: 'agent', text: 'truncated…' }], activityTrimmed: true });
    const fake = fakeBoard({ handlers: { 'list --all': () => ({ tasks: [row], problems: [] }), 'show A-3': () => refused('the board is unreadable', 'io') } });
    setBoardTransport(fake.transport);
    const store = boardStore();
    await store.open('pc', 'demo');
    boardUi().openTask('A-3');
    render(() => <SettingsProvider><HostsProvider><TaskDetail /></HostsProvider></SettingsProvider>);
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('Couldn’t load the full history'));
    expect(screen.getByText('truncated…')).toBeTruthy();

    fake.handlers['show A-3'] = () => task({ ...row, activity: [{ at: new Date().toISOString(), actor: 'agent', text: fullText }], activityTrimmed: undefined });
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    await waitFor(() => expect(screen.getByText(fullText)).toBeTruthy());
    expect(screen.queryByRole('alert')).toBeNull();
    expect(fake.calls.filter((call) => call.args.join(' ') === 'show A-3')).toHaveLength(2);
  });
});
