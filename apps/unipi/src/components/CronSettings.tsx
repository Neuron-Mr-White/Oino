/**
 * Settings → This computer → Cron (UNI-318, app part): concurrency,
 * retention, default time limit, paused, plus the C3 notification fields
 * (ntfy topic, default events) — just stored and shown here, read/written
 * through `cron_settings_get` / `cron_settings_set` against this PC's own
 * host id (the `last_path === 'local'` entry from `useHosts`).
 */
import { createEffect, createSignal, For, Show } from 'solid-js';
import { Clock, RefreshCw } from 'lucide-solid';
import { useHostsOptional } from '../hosts/context';
import { cronSettingsGet, cronSettingsSet, type CronSettingsData } from '../lib/cron';
import type { AppError } from '@unipi/protocol';
import { can } from '../lib/platform';

/** The host's notify events (UNI-316): `done` = finished ok, `failed` covers timeouts, `skipped` = overlap. */
const NOTIFY_EVENTS = ['done', 'failed', 'skipped'] as const;

function NumberRow(props: {
  label: string;
  description: string;
  value: number;
  disabled?: boolean;
  onCommit: (v: number) => void;
  testId: string;
}) {
  const [draft, setDraft] = createSignal(String(props.value));
  createEffect(() => setDraft(String(props.value)));
  return (
    <div class="settings-row">
      <div class="min-w-0">
        <p class="text-[15px] text-text">{props.label}</p>
        <p class="text-xs text-text-dim mt-0.5">{props.description}</p>
      </div>
      <div class="shrink-0">
        <input
          type="number"
          min="0"
          class="rounded-input bg-surface-2 border border-line px-3 py-1.5 text-sm text-text w-24"
          value={draft()}
          disabled={props.disabled}
          data-testid={props.testId}
          onInput={(e) => setDraft(e.currentTarget.value)}
          onBlur={() => {
            const n = Number(draft());
            if (Number.isFinite(n) && n >= 0) props.onCommit(Math.round(n));
            else setDraft(String(props.value));
          }}
        />
      </div>
    </div>
  );
}

export function CronSettingsCard() {
  if (!can('localSharing')) return null;
  const hosts = useHostsOptional();
  const localId = () => hosts?.hosts().find((h) => h.last_path === 'local')?.id;
  const [data, setData] = createSignal<CronSettingsData | undefined>();
  const [error, setError] = createSignal<AppError | undefined>();
  const [busy, setBusy] = createSignal(false);
  const [topic, setTopic] = createSignal('');

  const load = async () => {
    const id = localId();
    if (!id) return;
    setBusy(true);
    const r = await cronSettingsGet(id);
    setBusy(false);
    if (r.ok) {
      setData(r.value);
      setTopic(r.value.ntfy_topic ?? '');
      setError(undefined);
    } else setError(r.error);
  };
  createEffect(() => {
    if (localId()) void load();
  });

  async function set(patch: Partial<CronSettingsData>) {
    const id = localId();
    if (!id) return;
    setBusy(true);
    const r = await cronSettingsSet(id, patch);
    setBusy(false);
    if (r.ok) {
      setData(r.value);
      setTopic(r.value.ntfy_topic ?? '');
      setError(undefined);
    } else setError(r.error);
  }

  return (
    <Show when={localId() && data()}>
      <div data-testid="cron-settings-card">
        <div class="settings-row">
          <span class="settings-icon"><Clock size={18} /></span>
          <div class="min-w-0">
            <p class="text-[15px] text-text">Cron</p>
            <p class="text-xs text-text-dim mt-0.5">This PC's scheduled, unattended pi runs.</p>
          </div>
          <div class="shrink-0">
            <button type="button" class="btn btn-ghost btn-sm" disabled={busy()} onClick={() => void load()} aria-label="Refresh cron settings">
              <RefreshCw size={16} class={busy() ? 'refresh-spinning' : ''} />
            </button>
          </div>
        </div>
        <NumberRow
          label="Concurrency"
          description="How many runs at once."
          value={data()!.concurrency}
          disabled={busy()}
          testId="cron-settings-concurrency"
          onCommit={(v) => void set({ concurrency: v })}
        />
        <NumberRow
          label="Missed window"
          description="Seconds a missed fire is still run late within."
          value={data()!.missed_window_s}
          disabled={busy()}
          testId="cron-settings-missed-window"
          onCommit={(v) => void set({ missed_window_s: v })}
        />
        <NumberRow
          label="Keep runs"
          description="How many past runs to keep per job."
          value={data()!.retention_runs}
          disabled={busy()}
          testId="cron-settings-retention-runs"
          onCommit={(v) => void set({ retention_runs: v })}
        />
        <NumberRow
          label="Keep days"
          description="How many days of past runs to keep."
          value={data()!.retention_days}
          disabled={busy()}
          testId="cron-settings-retention-days"
          onCommit={(v) => void set({ retention_days: v })}
        />
        <NumberRow
          label="Default time limit"
          description="Seconds a run gets when a job doesn't set its own."
          value={data()!.default_time_limit_s}
          disabled={busy()}
          testId="cron-settings-default-time-limit"
          onCommit={(v) => void set({ default_time_limit_s: v })}
        />
        <div class="settings-row">
          <div class="min-w-0">
            <p class="text-[15px] text-text">Notify topic (ntfy)</p>
            <p class="text-xs text-text-dim mt-0.5">The ntfy.sh topic run notifications are posted to, if any.</p>
          </div>
          <div class="shrink-0">
            <input
              type="text"
              class="rounded-input bg-surface-2 border border-line px-3 py-1.5 text-sm text-text w-40"
              placeholder="none"
              value={topic()}
              disabled={busy()}
              data-testid="cron-settings-ntfy-topic"
              onInput={(e) => setTopic(e.currentTarget.value)}
              onBlur={() => void set({ ntfy_topic: topic().trim() === '' ? undefined : topic().trim() })}
            />
          </div>
        </div>
        <div class="settings-row">
          <div class="min-w-0">
            <p class="text-[15px] text-text">Notify on</p>
            <p class="text-xs text-text-dim mt-0.5">Which run outcomes are notified about by default.</p>
          </div>
          <div class="shrink-0 flex gap-2">
            <For each={NOTIFY_EVENTS}>
              {(ev) => {
                const on = () => (data()?.notify_default_events ?? []).includes(ev);
                return (
                  <button
                    type="button"
                    class="seg-option rounded-pill px-3 py-1 text-xs"
                    classList={{ 'bg-surface-3 text-text font-semibold': on(), 'text-text-muted': !on() }}
                    aria-pressed={on()}
                    disabled={busy()}
                    data-testid={`cron-settings-notify-${ev}`}
                    onClick={() => {
                      const cur = data()?.notify_default_events ?? [];
                      const next = on() ? cur.filter((e) => e !== ev) : [...cur, ev];
                      void set({ notify_default_events: next });
                    }}
                  >
                    {ev}
                  </button>
                );
              }}
            </For>
          </div>
        </div>
        <Show when={error()}>
          <p class="preview-field-error" role="alert">{error()!.message}</p>
        </Show>
      </div>
    </Show>
  );
}
