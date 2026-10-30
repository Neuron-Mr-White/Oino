// @vitest-environment jsdom
/**
 * UNI-118: the Dream sheet (unavailable / older bridge / off + Run now /
 * runs list / running detail with live steps + Stop confirm / finished
 * detail with markdown report + Approve/Reject confirm / errors) and the
 * header ⋯ "Dream" row.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createStore } from 'solid-js/store';
import { DreamSheet, dreamHint } from './DreamSheet';
import { ChatHeaderMenu } from './SessionMenus';
import type { Chat } from '../../lib/chat/store';
import type { DreamRun, DreamStatus } from '../../lib/chat/dream';

afterEach(() => {
  cleanup();
  document.body.innerHTML = '';
});

const finished: DreamRun = {
  id: 'staging-2',
  status: 'finished',
  startedAt: Date.now() - 3_600_000,
  endedAt: Date.now() - 3_400_000,
  manual: false,
  sessions: 6,
  events: 14,
  pending: 1,
  hasReport: true,
  proposals: [
    { id: 'staging-2/skill-kanboard-writes', name: 'kanboard-writes', kind: 'skill', decision: 'pending' },
    { id: 'staging-2/check-npm.md', name: 'check-npm', kind: 'check', decision: 'approved' },
  ],
  summary: ['3 memory edits', '2 proposals', '1 skipped'],
};
const running: DreamRun = { id: 'staging-3', status: 'running', startedAt: Date.now() - 65_000, manual: true, sessions: 4, events: 9, pending: 0, hasReport: false, proposals: [] };
const status = (extra: Partial<DreamStatus> = {}): DreamStatus => ({ v: 1, enabled: false, minSessions: 5, minGapHours: 12, lastRunAt: 0, running: false, runs: [], ...extra });

function fakeChat(extra: Record<string, unknown> = {}, opts: { action?: (...a: unknown[]) => Promise<void> } = {}) {
  const [state, setState] = createStore<Record<string, unknown>>({ conn: 'live', run: { running: false }, work: [], ...extra });
  const refreshDream = vi.fn();
  const dreamAction = vi.fn(opts.action ?? (async () => {}));
  const dreamDetail = vi.fn(async (id: string) =>
    id === 'staging-2'
      ? { id, report: '# Dream\n\n## Memory edits (applied)\n- **lesson** one\n\n## Proposals\n1. kanboard-writes', steps: [{ kind: 'tool', text: 'read INDEX.json' }], log: [] }
      : { id, report: '', steps: [{ kind: 'user', text: 'dream prompt sent' }, { kind: 'tool', text: 'memory_store lesson_a' }, { kind: 'error', text: 'bash failed: boom' }], log: ['x'] },
  );
  return { chat: { state, refreshDream, dreamAction, dreamDetail } as unknown as Chat, setState, refreshDream, dreamAction, dreamDetail };
}

describe('DreamSheet', () => {
  it('no dream module: unavailable state; asks the bridge once on open', () => {
    const f = fakeChat();
    render(() => <DreamSheet open onClose={() => {}} chat={f.chat} />);
    expect(screen.getByTestId('dream-empty').textContent).toContain('isn’t available');
    expect(f.refreshDream).toHaveBeenCalledTimes(1);
  });

  it('older bridge: says to update unipi', () => {
    const f = fakeChat({ dreamSupported: false });
    render(() => <DreamSheet open onClose={() => {}} chat={f.chat} />);
    expect(screen.getByTestId('dream-empty').textContent).toContain('needs a newer unipi');
  });

  it('off with no runs: status Off, turn-on hint, no-runs empty state, Run now sends run', async () => {
    const f = fakeChat({ dream: status() });
    render(() => <DreamSheet open onClose={() => {}} chat={f.chat} />);
    expect(screen.getByTestId('dream-status-line').textContent).toBe('Background dreaming is off');
    expect(screen.getByTestId('dream-off-hint').textContent).toContain('/unipi:settings');
    expect(screen.getByTestId('dream-no-runs')).toBeTruthy();
    fireEvent.click(screen.getByTestId('dream-run'));
    expect(f.dreamAction).toHaveBeenCalledWith('run');
    await waitFor(() => expect(screen.getByTestId('dream-flash').textContent).toContain('started'));
  });

  it('run failure shows the reason as an error', async () => {
    const f = fakeChat({ dream: status() }, { action: async () => Promise.reject(new Error('A dream is already running (pid 9).')) });
    render(() => <DreamSheet open onClose={() => {}} chat={f.chat} />);
    fireEvent.click(screen.getByTestId('dream-run'));
    await waitFor(() => expect(screen.getByTestId('dream-flash').getAttribute('data-tone')).toBe('error'));
    expect(screen.getByTestId('dream-flash').textContent).toContain('already running');
  });

  it('on: schedule hint; runs list running first with facts and a waiting badge; Run now disabled while one runs', () => {
    const f = fakeChat({ dream: status({ enabled: true, running: true, due: { due: false, reason: 'needs 3 more new sessions' }, lastRunAt: Date.now() - 3_600_000, runs: [finished, running] }) });
    render(() => <DreamSheet open onClose={() => {}} chat={f.chat} />);
    expect(screen.queryByTestId('dream-off-hint')).toBeNull();
    expect(screen.getByText(/after 5 new sessions, at most every 12 h/)).toBeTruthy();
    const rows = screen.getByTestId('dream-runs').querySelectorAll('.dream-run-row');
    expect(rows[0]!.getAttribute('data-status')).toBe('running');
    expect(rows[1]!.textContent).toContain('6 sessions · 1 waiting');
    expect(rows[1]!.querySelector('.dream-badge')!.textContent).toBe('1');
    expect((screen.getByTestId('dream-run') as HTMLButtonElement).disabled).toBe(true);
  });

  it('running detail: live steps (polled), Stop asks first then stops', async () => {
    const f = fakeChat({ dream: status({ running: true, runs: [running] }) });
    render(() => <DreamSheet open onClose={() => {}} chat={f.chat} />);
    fireEvent.click(screen.getByTestId('dream-runs').querySelector('.dream-run-row')!);
    await waitFor(() => expect(screen.getByTestId('dream-steps').querySelectorAll('li')).toHaveLength(3));
    expect(screen.getByTestId('dream-steps').querySelector('li[data-kind="error"]')!.textContent).toContain('boom');
    expect(screen.getByText('Written at the end of the dream.')).toBeTruthy();
    fireEvent.click(screen.getByTestId('dream-stop'));
    expect(f.dreamAction).not.toHaveBeenCalled();
    expect(screen.getByTestId('dream-confirm').textContent).toContain('Stop this dream?');
    fireEvent.click(screen.getByTestId('dream-confirm-go'));
    await waitFor(() => expect(f.dreamAction).toHaveBeenCalledWith('stop', 'staging-3'));
  });

  it('finished detail: markdown report, summary, proposals; Approve needs a confirm; Back returns to the list', async () => {
    const f = fakeChat({ dream: status({ runs: [finished] }) });
    render(() => <DreamSheet open onClose={() => {}} chat={f.chat} />);
    fireEvent.click(screen.getByTestId('dream-runs').querySelector('.dream-run-row')!);
    await waitFor(() => expect(screen.getByTestId('dream-report').querySelector('strong')?.textContent).toBe('lesson'));
    expect(screen.getByText('3 memory edits · 2 proposals · 1 skipped')).toBeTruthy();
    const props = screen.getByTestId('dream-proposals').querySelectorAll('.dream-proposal');
    expect(props[0]!.getAttribute('data-decision')).toBe('pending');
    expect(props[1]!.querySelector('button')).toBeNull();
    fireEvent.click(screen.getByText('Approve'));
    expect(screen.getByTestId('dream-confirm').textContent).toContain('Approve skill “kanboard-writes”?');
    expect(screen.getByTestId('dream-confirm').textContent).toContain('craft-skill check');
    fireEvent.click(screen.getByTestId('dream-confirm-go'));
    await waitFor(() => expect(f.dreamAction).toHaveBeenCalledWith('approve', 'staging-2', 'staging-2/skill-kanboard-writes'));
    await waitFor(() => expect(screen.getByTestId('dream-flash').textContent).toContain('Approved kanboard-writes'));
    fireEvent.click(screen.getByLabelText('Back to all dreams'));
    expect(screen.getByTestId('dream-list')).toBeTruthy();
  });

  it('Reject confirm can be cancelled; Dismiss sends dismiss and goes back', async () => {
    const f = fakeChat({ dream: status({ runs: [finished] }) });
    render(() => <DreamSheet open onClose={() => {}} chat={f.chat} />);
    fireEvent.click(screen.getByTestId('dream-runs').querySelector('.dream-run-row')!);
    fireEvent.click(screen.getByText('Reject'));
    fireEvent.click(screen.getByText('Cancel'));
    expect(f.dreamAction).not.toHaveBeenCalled();
    fireEvent.click(screen.getByTestId('dream-dismiss'));
    await waitFor(() => expect(f.dreamAction).toHaveBeenCalledWith('dismiss', 'staging-2'));
    await waitFor(() => expect(screen.getByTestId('dream-list')).toBeTruthy());
  });

  it('failed run shows its error', async () => {
    const failed: DreamRun = { ...running, id: 'staging-9', status: 'failed', endedAt: Date.now(), error: 'timed out (max runtime)' };
    const f = fakeChat({ dream: status({ runs: [failed] }) });
    render(() => <DreamSheet open onClose={() => {}} chat={f.chat} />);
    fireEvent.click(screen.getByTestId('dream-runs').querySelector('.dream-run-row')!);
    expect(screen.getByText('timed out (max runtime)')).toBeTruthy();
    await waitFor(() => expect(screen.getByText('This dream left no report.')).toBeTruthy());
  });
});

describe('Dream menu row', () => {
  it('shows a running / waiting hint; opens the sheet', () => {
    const f = fakeChat({ dream: status({ runs: [finished] }) });
    expect(dreamHint(f.chat)).toBe('1');
    f.setState('dream', status({ running: true, runs: [running] }));
    expect(dreamHint(f.chat)).toBe('running');
    expect(dreamHint(fakeChat().chat)).toBeUndefined();
    const onDream = vi.fn();
    render(() => (
      <ChatHeaderMenu anchor={new DOMRect(0, 0, 10, 10)} onDismiss={() => {}} onNew={() => {}} onResume={() => {}} onTree={() => {}} onRename={() => {}} canRename onOpenSection={() => {}} onDream={onDream} dreamHint="2" />
    ));
    const row = screen.getByRole('menuitem', { name: /Dream/ });
    expect(row.textContent).toContain('2');
    fireEvent.click(row);
    expect(onDream).toHaveBeenCalled();
  });
});
