// @vitest-environment jsdom
import { render, screen, waitFor, cleanup, fireEvent } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { vi, it, describe, expect, afterEach, beforeEach } from 'vitest';
import Cron, { lastRunChip, nextFireText, wakeText } from './Cron';
import { notifyFromForm, notifyModeOf, notifyText } from '../lib/cron';
import { openChatState, closeChat } from '../lib/session/open';
import { HostsProvider } from '../hosts/context';
import { SettingsProvider } from '../settings/context';
import { call } from '../lib/ipc';

vi.mock('../lib/ipc', () => ({ call: vi.fn() }));
// The chat layer (opened by "Open session") talks over this; keep it inert.
vi.mock('../lib/session/link', () => ({
  chatTransport: () => ({
    list: async () => ({ sessions: [] }),
    open: () => ({ send: () => undefined, close: () => undefined }),
  }),
}));
vi.mock('../lib/sharing', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/sharing')>();
  return { ...actual, isThisComputer: () => true };
});

const hosts = [{ id: 'host-1', name: 'coffee', ticket: 't', paired_at: 0, last_path: 'lan' as const }];

const job = {
  id: 'digest',
  name: 'daily digest',
  prompt: "Summarize today's inbox",
  schedule: '0 0 9 * * *',
  tz: 'UTC',
  cwd: '/home/me/jobs/digest',
  enabled: true,
  time_limit_s: 1800,
  created: 1700000000000,
  updated: 1700000000000,
  next_fire: new Date(Date.now() + 3600_000).toISOString(),
  running: false,
  last_run_status: 'ok' as const,
};

function mockIpc(overrides: Record<string, unknown> = {}) {
  vi.mocked(call).mockImplementation(async (cmd: string) =>
    (cmd === 'list_hosts'
      ? { ok: true, value: hosts }
      : cmd === 'cron_list'
        ? overrides.cron_list ?? { ok: true, value: { jobs: [job], paused: false } }
        : { ok: true, value: null }) as never,
  );
}

function mount() {
  return render(() => (
    <HashRouter>
      <Route path="*" component={() => <SettingsProvider><HostsProvider><Cron /></HostsProvider></SettingsProvider>} />
    </HashRouter>
  ));
}

afterEach(() => {
  cleanup();
  vi.resetAllMocks();
  location.hash = '#/';
});

describe('Cron screen (UNI-317)', () => {
  it('no PC paired: the shared no-host state', async () => {
    vi.mocked(call).mockImplementation(async (cmd: string) => (cmd === 'list_hosts' ? { ok: true, value: [] } : { ok: true, value: null }));
    mount();
    expect(await screen.findByTestId('no-host-state')).toHaveTextContent('No PC paired yet');
  });

  it('lists jobs with their schedule, next fire and last run chip', async () => {
    mockIpc();
    mount();
    const row = await screen.findByTestId('cron-job-digest');
    expect(row).toHaveTextContent('daily digest');
    expect(row).toHaveTextContent('0 0 9 * * *');
    expect(row).toHaveTextContent('ok');
  });

  it('empty state offers to add a job', async () => {
    mockIpc({ cron_list: { ok: true, value: { jobs: [], paused: false } } });
    mount();
    expect(await screen.findByTestId('cron-empty-add')).toHaveTextContent('Add a job');
  });

  it('cron feature off (E_UNAUTHORIZED) shows FeatureOff, not an error banner', async () => {
    mockIpc({ cron_list: { ok: false, error: { code: 'E_UNAUTHORIZED', message: 'not shared', retryable: false } } });
    mount();
    expect(await screen.findByTestId('cron-disabled')).toBeInTheDocument();
  });

  it('pause all flips to paused and back', async () => {
    mockIpc();
    vi.mocked(call).mockImplementation(async (cmd: string) => {
      if (cmd === 'list_hosts') return { ok: true, value: hosts };
      if (cmd === 'cron_list') return { ok: true, value: { jobs: [job], paused: false } };
      if (cmd === 'cron_pause_all') return { ok: true, value: null };
      return { ok: true, value: null };
    });
    mount();
    await screen.findByTestId('cron-job-digest');
    const toggle = screen.getByTestId('cron-pause-all').querySelector('input[role=switch]') as HTMLInputElement;
    expect(toggle.checked).toBe(false);
    fireEvent.click(toggle);
    await waitFor(() => expect(call).toHaveBeenCalledWith('cron_pause_all', { hostId: 'host-1' }, expect.anything()));
  });

  it('opening a job shows its detail sheet with prompt and folder', async () => {
    mockIpc();
    vi.mocked(call).mockImplementation(async (cmd: string) => {
      if (cmd === 'list_hosts') return { ok: true, value: hosts };
      if (cmd === 'cron_list') return { ok: true, value: { jobs: [job], paused: false } };
      if (cmd === 'cron_show') return { ok: true, value: { job, runs: [] } };
      return { ok: true, value: null };
    });
    mount();
    const row = await screen.findByTestId('cron-job-digest');
    fireEvent.click(row);
    const sheet = await screen.findByTestId('cron-detail-sheet');
    await waitFor(() => expect(sheet).toHaveTextContent("Summarize today's inbox"));
    expect(sheet).toHaveTextContent('/home/me/jobs/digest');
    expect(screen.getByText('No runs yet.')).toBeInTheDocument();
  });
});

describe('Cron notify, wake and Open session (UNI-319)', () => {
  const run = { id: 'r1', job_id: 'digest', status: 'ok' as const, fired_at: 1700000000000, summary: 'did it', session_path: '/home/me/.pi/agent/sessions/--x--/r1.jsonl', missed: false };
  const settings = { concurrency: 2, missed_window_s: 3600, retention_runs: 50, retention_days: 30, default_time_limit_s: 1800, paused: false, enabled: true, devices: [], notify_default_events: ['failed'], notify_channels: ['ntfy', 'telegram'] };
  function routes(extra: Record<string, unknown> = {}) {
    vi.mocked(call).mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd in extra) {
        const v = extra[cmd];
        return (typeof v === 'function' ? (v as (a: unknown) => unknown)(args) : v) as never;
      }
      if (cmd === 'list_hosts') return { ok: true, value: hosts } as never;
      if (cmd === 'cron_list') return { ok: true, value: { jobs: [job], paused: false } } as never;
      if (cmd === 'cron_show') return { ok: true, value: { job, runs: [run] } } as never;
      if (cmd === 'cron_settings_get') return { ok: true, value: settings } as never;
      if (cmd === 'cron_log') return { ok: true, value: { job_id: 'digest', run, log: 'hello' } } as never;
      return { ok: true, value: null } as never;
    });
  }
  afterEach(() => closeChat());

  it('the detail shows notify (PC default) and the wake', async () => {
    routes({ cron_show: { ok: true, value: { job: { ...job, wake: { session_file: '/s/planning.jsonl', pid: 77, session_name: 'planning' } }, runs: [] } } });
    mount();
    fireEvent.click(await screen.findByTestId('cron-job-digest'));
    await waitFor(() => expect(screen.getByTestId('cron-detail-notify')).toHaveTextContent('PC default: failed'));
    expect(screen.getByTestId('cron-detail-wake')).toHaveTextContent('planning (open)');
  });

  it('adding a job with chosen outcomes and one channel sends notify', async () => {
    routes({ cron_add: (args: unknown) => ({ ok: true, value: { ...job, ...(args as { job: object }).job } }) });
    mount();
    fireEvent.click(await screen.findByTestId('cron-add'));
    fireEvent.input(screen.getByTestId('cron-form-name'), { target: { value: 'nightly' } });
    fireEvent.input(screen.getByTestId('cron-form-prompt'), { target: { value: 'clean up' } });
    fireEvent.input(screen.getByTestId('cron-form-cwd'), { target: { value: '/tmp' } });
    fireEvent.input(screen.getByTestId('cron-form-schedule'), { target: { value: 'every day at 2:00' } });
    fireEvent.click(screen.getByTestId('cron-notify-choose'));
    await screen.findByTestId('cron-notify-channel-telegram'); // the PC's settings are in
    fireEvent.click(await screen.findByTestId('cron-notify-event-done'));
    fireEvent.click(await screen.findByTestId('cron-notify-channel-telegram')); // untick telegram → ntfy only
    fireEvent.click(screen.getByTestId('cron-save'));
    await waitFor(() => expect(call).toHaveBeenCalledWith('cron_add', expect.objectContaining({ job: expect.objectContaining({ notify: { events: ['done', 'failed'], channels: ['ntfy'] } }) }), expect.anything()));
    const sent = vi.mocked(call).mock.calls.find((c) => c[0] === 'cron_add')![1] as { job: Record<string, unknown> };
    expect(sent.job.wake_pid).toBeUndefined();
  });

  it('editing: Never notify + wake Off clears both', async () => {
    const withBoth = { ...job, notify: { events: ['done'] }, wake: { session_file: '/s/a.jsonl' } };
    routes({ cron_show: { ok: true, value: { job: withBoth, runs: [] } }, cron_edit: { ok: true, value: withBoth } });
    mount();
    fireEvent.click(await screen.findByTestId('cron-job-digest'));
    fireEvent.click(await screen.findByTestId('cron-edit'));
    expect(await screen.findByTestId('cron-wake-label')).toHaveTextContent('not open now');
    fireEvent.click(screen.getByTestId('cron-notify-never'));
    fireEvent.click(screen.getByTestId('cron-wake-off'));
    expect(screen.getByTestId('cron-wake-label')).toHaveTextContent('Off');
    fireEvent.click(screen.getByTestId('cron-save'));
    await waitFor(() =>
      expect(call).toHaveBeenCalledWith('cron_edit', expect.objectContaining({ patch: expect.objectContaining({ notify: { events: [] }, wake_clear: true }) }), expect.anything()),
    );
  });

  it("Open session asks the host to open the run's session and opens that chat", async () => {
    routes({ cron_open_session: { ok: true, value: { session: 'sess-42' } } });
    mount();
    fireEvent.click(await screen.findByTestId('cron-job-digest'));
    fireEvent.click(await screen.findByTestId('cron-run-r1'));
    fireEvent.click(await screen.findByTestId('cron-open-session'));
    await waitFor(() => expect(openChatState()?.target.session).toBe('sess-42'));
    expect(call).toHaveBeenCalledWith('cron_open_session', { hostId: 'host-1', runId: 'r1' }, expect.anything());
    expect(openChatState()?.target.title).toBe('daily digest');
    await waitFor(() => expect(screen.queryByTestId('cron-detail-sheet')).toBeNull());
  });

  it('Open session errors stay in the sheet, in words', async () => {
    routes({
      cron_open_session: { ok: false, error: { code: 'E_NOT_FOUND', message: "This run's session file is gone (deleted or moved).", retryable: false } },
    });
    mount();
    fireEvent.click(await screen.findByTestId('cron-job-digest'));
    fireEvent.click(await screen.findByTestId('cron-run-r1'));
    fireEvent.click(await screen.findByTestId('cron-open-session'));
    expect(await screen.findByTestId('cron-open-session-error')).toHaveTextContent('session file is gone');
    expect(call).toHaveBeenCalledWith('cron_open_session', { hostId: 'host-1', runId: 'r1' }, expect.anything());
    expect(openChatState()).toBeNull();
  });

  it('a run without a saved session has no Open session button', async () => {
    routes({ cron_log: { ok: true, value: { job_id: 'digest', run: { ...run, session_path: undefined }, log: '' } } });
    mount();
    fireEvent.click(await screen.findByTestId('cron-job-digest'));
    fireEvent.click(await screen.findByTestId('cron-run-r1'));
    await screen.findByTestId('cron-run-log-sheet');
    await waitFor(() => expect(screen.getByTestId('cron-run-log-sheet')).toHaveTextContent('(no output)'));
    expect(screen.queryByTestId('cron-open-session')).toBeNull();
  });
});

describe('notify + wake helpers (UNI-319)', () => {
  it('notifyModeOf', () => {
    expect(notifyModeOf(undefined)).toBe('default');
    expect(notifyModeOf({ channels: ['ntfy'] })).toBe('default');
    expect(notifyModeOf({ events: [] })).toBe('never');
    expect(notifyModeOf({ events: ['done'] })).toBe('choose');
  });
  it('notifyFromForm: every channel = default (absent); never drops channels; order follows the outcome list', () => {
    expect(notifyFromForm('default', [], ['ntfy', 'telegram'], ['ntfy', 'telegram'])).toEqual({});
    expect(notifyFromForm('choose', ['failed', 'done'], ['ntfy'], ['ntfy', 'telegram'])).toEqual({ events: ['done', 'failed'], channels: ['ntfy'] });
    expect(notifyFromForm('never', ['done'], ['ntfy'], ['ntfy', 'telegram'])).toEqual({ events: [] });
    expect(notifyFromForm('choose', ['done'], [], ['ntfy'])).toEqual({ events: ['done'] });
  });
  it('notifyText + wakeText', () => {
    expect(notifyText(undefined)).toBe('PC default: failed');
    expect(notifyText(undefined, ['done', 'failed'])).toBe('PC default: done, failed');
    expect(notifyText({ events: ['skipped'], channels: ['telegram'] })).toBe('skipped via telegram');
    expect(notifyText({ events: [] })).toBe('never');
    expect(wakeText({})).toBe('no chat');
    expect(wakeText({ wake: { session_file: '/a/b/2026_planning.jsonl' } })).toBe('2026_planning — not open now; skipped until it is');
    expect(wakeText({ wake: { session_id: 'abc', pid: 3, session_name: 'x' } })).toBe('x (open)');
  });
});

describe('pure helpers', () => {
  it('nextFireText: disabled, no next run, in N min/h/d', () => {
    expect(nextFireText({ next_fire: undefined, enabled: false })).toBe('disabled');
    expect(nextFireText({ next_fire: undefined, enabled: true })).toBe('no next run');
    expect(nextFireText({ next_fire: new Date(Date.now() + 5 * 60_000).toISOString(), enabled: true })).toBe('in 5 min');
  });

  it('lastRunChip: running wins over last status', () => {
    expect(lastRunChip({ running: true, last_run_status: 'ok' })).toBe('running');
    expect(lastRunChip({ running: false, last_run_status: 'failed' })).toBe('failed');
    expect(lastRunChip({ running: false, last_run_status: undefined })).toBeUndefined();
  });
});
