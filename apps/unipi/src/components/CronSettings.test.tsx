// @vitest-environment jsdom
/** UNI-318: Settings → This computer → Cron settings. */
import { render, screen, waitFor, cleanup, fireEvent } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CronSettingsCard } from './CronSettings';
import { HostsProvider } from '../hosts/context';
import { SettingsProvider } from '../settings/context';
import { setPlatformOverride } from '../lib/platform';
import { call } from '../lib/ipc';

vi.mock('../lib/ipc', () => ({ call: vi.fn() }));

const hosts = [{ id: 'local-1', name: 'This computer', ticket: 't', paired_at: 0, last_path: 'local' as const }];

let settings: Record<string, unknown>;

function mount() {
  return render(() => <SettingsProvider><HostsProvider><CronSettingsCard /></HostsProvider></SettingsProvider>);
}

beforeEach(() => {
  setPlatformOverride('tauri-desktop');
  settings = {
    concurrency: 2,
    missed_window_s: 300,
    retention_runs: 50,
    retention_days: 30,
    default_time_limit_s: 1800,
    paused: false,
    enabled: true,
    devices: [],
    ntfy_topic: undefined,
    notify_default_events: ['failed'],
  };
  vi.mocked(call).mockImplementation(async (cmd: string, args: unknown) => {
    if (cmd === 'list_hosts') return { ok: true, value: hosts };
    if (cmd === 'cron_settings_get') return { ok: true, value: settings };
    if (cmd === 'cron_settings_set') {
      const patch = (args as { patch: Record<string, unknown> }).patch;
      settings = { ...settings, ...patch };
      return { ok: true, value: settings };
    }
    return { ok: true, value: null };
  });
});
afterEach(() => {
  cleanup();
  setPlatformOverride(undefined);
  vi.clearAllMocks();
});

describe('CronSettingsCard', () => {
  it('loads and shows the fields for this computer', async () => {
    mount();
    await screen.findByTestId('cron-settings-card');
    expect((screen.getByTestId('cron-settings-concurrency') as HTMLInputElement).value).toBe('2');
    expect((screen.getByTestId('cron-settings-retention-days') as HTMLInputElement).value).toBe('30');
    expect(screen.getByTestId('cron-settings-notify-failed')).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByTestId('cron-settings-notify-done')).toHaveAttribute('aria-pressed', 'false');
  });

  it('editing concurrency commits on blur', async () => {
    mount();
    const input = await screen.findByTestId('cron-settings-concurrency');
    fireEvent.input(input, { target: { value: '5' } });
    fireEvent.blur(input);
    await waitFor(() => expect(call).toHaveBeenCalledWith('cron_settings_set', { hostId: 'local-1', patch: { concurrency: 5 } }, expect.anything()));
  });

  it('toggling a notify event flips it', async () => {
    mount();
    await screen.findByTestId('cron-settings-card');
    fireEvent.click(screen.getByTestId('cron-settings-notify-done'));
    await waitFor(() => expect(screen.getByTestId('cron-settings-notify-done')).toHaveAttribute('aria-pressed', 'true'));
  });
});
