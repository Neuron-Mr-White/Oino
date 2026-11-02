// @vitest-environment jsdom
import { render, screen, waitFor, cleanup, fireEvent } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { vi, it, describe, expect, afterEach, beforeEach } from 'vitest';
import Cron, { humanSchedule, jobState, lastRunChip, nextFireText, runDurationText, runWhenText, scheduleText, wakeText } from './Cron';
import { notifyFromForm, notifyModeOf, notifyText, parseCronRunLog } from '../lib/cron';
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
    expect(row).toHaveTextContent('Every day at 9:00');
    // The last run is an icon chip with a short label.
    expect(screen.getByTestId('cron-job-last-digest')).toHaveTextContent('OK');
    expect(screen.getByTestId('cron-job-last-digest').querySelector('svg')).toBeTruthy();
  });

  it('the row toggle disables / enables the job in place', async () => {
    mockIpc();
    mount();
    const toggle = await screen.findByTestId('cron-job-toggle-digest');
    const input = toggle.querySelector('input')!;
    expect(input.checked).toBe(true);
    fireEvent.click(input);
    await waitFor(() => expect(call).toHaveBeenCalledWith('cron_disable', expect.objectContaining({ id: 'digest' }), expect.anything()));
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

  it("Open session routes to the run's session (the conversation route, G-12)", async () => {
    routes({ cron_open_session: { ok: true, value: { session: 'sess-42' } } });
    mount();
    fireEvent.click(await screen.findByTestId('cron-job-digest'));
    fireEvent.click(await screen.findByTestId('cron-run-r1'));
    fireEvent.click(await screen.findByTestId('cron-open-session'));
    await waitFor(() => expect(location.hash).toBe('#/s/host-1/sess-42'));
    expect(call).toHaveBeenCalledWith('cron_open_session', { hostId: 'host-1', runId: 'r1' }, expect.anything());
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

describe('WP-A23: form preview and the /cron add entry', () => {
  it('the add form previews the resolved schedule (debounced cron_parse)', async () => {
    vi.mocked(call).mockImplementation(async (cmd: string) =>
      (cmd === 'list_hosts'
        ? { ok: true, value: hosts }
        : cmd === 'cron_list'
          ? { ok: true, value: { jobs: [job], paused: false } }
          : cmd === 'cron_settings_get'
            ? { ok: true, value: { concurrency: 2, missed_window_s: 3600, retention_runs: 50, retention_days: 30, default_time_limit_s: 1800, paused: false, enabled: true, devices: [], notify_default_events: ['failed'], notify_channels: ['ntfy', 'telegram'] } }
            : cmd === 'cron_parse'
              ? { ok: true, value: { expression: '0 0 9 * * *', one_time: false, tz: 'Europe/Berlin', next_fires: ['2026-01-01T09:00:00+01:00', '2026-01-02T09:00:00+01:00', '2026-01-03T09:00:00+01:00'] } }
              : { ok: true, value: null }) as never,
    );
    mount();
    fireEvent.click(await screen.findByTestId('cron-add'));
    fireEvent.input(screen.getByTestId('cron-form-schedule'), { target: { value: 'every day at 9:00' } });
    // The debounce (300 ms) folds typing; only after it settles does the
    // preview land.
    const preview = await screen.findByTestId('cron-form-preview', undefined, { timeout: 2000 });
    expect(preview).toHaveTextContent('0 0 9 * * *');
    expect(preview).toHaveTextContent('2026-01-01T09:00:00+01:00');
    const parse = vi.mocked(call).mock.calls.filter((c) => c[0] === 'cron_parse').at(-1);
    expect(parse?.[1]).toMatchObject({ schedule: 'every day at 9:00' });
  });

  it('?form=new (the /cron add route) opens the add form once', async () => {
    location.hash = '#/cron?form=new';
    mockIpc();
    mount();
    expect(await screen.findByTestId('cron-form-sheet')).toBeInTheDocument();
    // The param is consumed: back/refresh is the plain list.
    await waitFor(() => expect(location.hash).toBe('#/cron'));
  });
});

describe('humanSchedule (D-33)', () => {
  it('says common schedules in words and leaves the rest as written', async () => {
    expect(humanSchedule('0 0 9 * * *')).toBe('Every day at 9:00');
    expect(humanSchedule('30 8 * * 1')).toBe('Every Monday at 8:30');
    expect(humanSchedule('0 0 7 * * 1-5')).toBe('Weekdays at 7:00');
    expect(humanSchedule('*/5 * * * *')).toBe('*/5 * * * *');
  });
});

describe('G-07 / D-32: readable failed cron run sheet & accessible pickers', () => {
  it('parses JSONL cron logs into cause + readable transcript without raw JSON events', () => {
    const rawLog = [
      'oreground.\\n- Resume a finished subagent","toolsAdded":[{"name":"read","parameters":{"type":"object"}}]}',
      '{"type":"message_end","message":{"role":"user","content":[{"type":"text","text":"Reply with exactly: cron-ok"}]}}',
      '{"type":"message_end","message":{"role":"custom","customType":"unipi-memory-recall-reminder","content":"## Memory"}}',
      '{"type":"message_end","message":{"role":"assistant","content":[],"provider":"openrouter","model":"moonshotai/kimi-k2.6","stopReason":"error","errorMessage":"401: {\\"message\\":\\"User not found.\\",\\"code\\":401}"}}',
      '{"type":"agent_end","messages":[{"role":"system","sections":{"preamble":"huge system prompt',
      '401: {"message":"User not found.","code":401}',
    ].join('\n');
    const parsed = parseCronRunLog(rawLog, 'oino exited with status exit status: 1', 'openrouter/moonshotai/kimi-k2.6');
    expect(parsed.cause).toBe('OpenRouter rejected the key (401)');
    expect(parsed.transcript).toContain('Prompt: Reply with exactly: cron-ok');
    expect(parsed.transcript).not.toContain('toolsAdded');
    expect(parsed.transcript).not.toContain('huge system prompt');
    expect(parsed.transcript).not.toContain('{"type"');
  });

  it('titles the failed run sheet with the job name, shows the cause banner, and exposes accessible form controls', async () => {
    const failedRun = {
      id: 'c1a199b7d9810000',
      job_id: 'digest',
      status: 'failed' as const,
      fired_at: 1700000000000,
      reason: 'oino exited with status exit status: 1',
      session_path: '/tmp/s.jsonl',
      missed: false,
    };
    const rawLog = [
      '{"type":"message_end","message":{"role":"user","content":[{"type":"text","text":"Summarize inbox"}]}}',
      '{"type":"message_end","message":{"role":"assistant","content":[],"provider":"openrouter","stopReason":"error","errorMessage":"401: {\\"message\\":\\"User not found.\\",\\"code\\":401}"}}',
    ].join('\n');
    vi.mocked(call).mockImplementation(async (cmd: string) =>
      (cmd === 'list_hosts'
        ? { ok: true, value: hosts }
        : cmd === 'cron_list'
          ? { ok: true, value: { jobs: [job], paused: false } }
          : cmd === 'cron_show'
            ? { ok: true, value: { job, runs: [failedRun] } }
            : cmd === 'cron_log'
              ? { ok: true, value: { job_id: 'digest', run: failedRun, log: rawLog } }
              : { ok: true, value: null }) as never,
    );
    mount();
    fireEvent.click(await screen.findByTestId('cron-job-digest'));
    const historyRow = await screen.findByTestId('cron-run-c1a199b7d9810000');
    expect(historyRow).toHaveTextContent('Exited with code 1');
    fireEvent.click(historyRow);
    const sheet = await screen.findByTestId('cron-run-log-sheet');
    expect(sheet).toHaveTextContent('daily digest');
    expect(sheet).not.toHaveTextContent('Run c1a199b7d9810000');
    const cause = await screen.findByTestId('cron-run-cause');
    expect(cause).toHaveTextContent('Failed — OpenRouter rejected the key (401)');
    expect(screen.getByTestId('cron-log-text')).not.toHaveTextContent('{"type"');

    // D-32: accessible names + Folder & Model pickers + empty-form Save error in footer.
    cleanup();
    mockIpc();
    mount();
    fireEvent.click(await screen.findByTestId('cron-add'));
    expect(screen.getByLabelText('Name')).toBeInTheDocument();
    expect(screen.getByLabelText('Prompt')).toBeInTheDocument();
    expect(screen.getByLabelText('Folder')).toBeInTheDocument();
    expect(screen.getByLabelText('Schedule')).toBeInTheDocument();
    expect(screen.getByLabelText('Model (optional)')).toBeInTheDocument();
    expect(screen.getByTestId('workspace-chip')).toBeInTheDocument();
    expect(screen.getByTestId('cron-model-picker-btn')).toBeInTheDocument();
    // The ONE model dialog, titled by the field, with "Use default" in its footer.
    fireEvent.click(screen.getByTestId('cron-model-picker-btn'));
    const modelDialog = await screen.findByTestId('cron-model-menu');
    // Desktop: the centred dialog; phone: the full-screen sheet — both titled "Job model".
    expect(modelDialog.closest('[role="dialog"]')?.textContent).toContain('Job model');
    expect(screen.getByTestId('model-picker-foot').contains(screen.getByTestId('cron-model-default'))).toBe(true);
    expect(screen.queryByTestId('model-use-startup')).toBeNull();
    fireEvent.click(modelDialog.closest('[role="dialog"]')!.querySelector('button[aria-label="Close"]')!);
    await waitFor(() => expect(screen.queryByTestId('cron-model-menu')).toBeNull());
    fireEvent.click(screen.getByTestId('cron-save'));
    expect(await screen.findByTestId('cron-form-error')).toHaveTextContent('Name, prompt, folder and schedule are all required.');
  });
});

describe('[ui-chat-bits] cron words', () => {
  const now = new Date(2026, 10, 2, 21, 40).getTime();
  it('scheduleText: one-shots, steps, weekday lists, monthly; the rest as written', () => {
    expect(scheduleText('0 0 9 * * *')).toBe('Every day at 9:00');
    expect(scheduleText('0 */15 * * * *')).toBe('Every 15 min');
    expect(scheduleText('0 0 */6 * * *')).toBe('Every 6 h');
    expect(scheduleText('0 30 8 * * 1,3,5')).toBe('Mon, Wed, Fri at 8:30');
    expect(scheduleText('0 0 10 * * 0,6')).toBe('Weekends at 10:00');
    expect(scheduleText('0 0 7 1 * *')).toBe('Monthly on day 1 at 7:00');
    expect(scheduleText(`at:${new Date(2026, 10, 1, 19, 40).toISOString()}`, now)).toBe('Once, Nov 1, 19:40');
    expect(scheduleText('0 0 9 1-7 * 1')).toBe('0 0 9 1-7 * 1');
  });
  it('runWhenText: short date + 24h time and a relative part', () => {
    expect(runWhenText(new Date(2026, 10, 1, 19, 40).getTime(), now)).toEqual({ abs: 'Nov 1, 19:40', rel: '26 h ago' });
    expect(runWhenText(now - 5 * 60_000, now).rel).toBe('5 min ago');
    expect(runWhenText(now, now).rel).toBe('just now');
    expect(runWhenText(new Date(2025, 0, 3, 7, 5).getTime(), now).abs).toBe('Jan 3, 2025, 07:05');
  });
  it('runDurationText and jobState', () => {
    expect(runDurationText({ started_at: 0, ended_at: 42_000 })).toBe('42s');
    expect(runDurationText({ started_at: 0, ended_at: 185_000 })).toBe('3m 05s');
    expect(runDurationText({ started_at: 0 })).toBeUndefined();
    expect(jobState({ enabled: true, running: true })).toBe('running');
    expect(jobState({ enabled: false, running: false, last_run_status: 'failed' })).toBe('disabled');
    expect(jobState({ enabled: true, running: false, last_run_status: 'timeout' })).toBe('failed');
    expect(jobState({ enabled: true, running: false, last_run_status: 'ok' })).toBe('scheduled');
  });
});
