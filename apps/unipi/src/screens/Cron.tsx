/**
 * Cron (WP-A23, G-17/Z-112): the PC's scheduled, unattended runs — job files
 * the `unipi.cron` extension or this screen write, fired by `unipi-host
 * serve`'s embedded scheduler. List → detail (prompt, folder, run history) →
 * add/edit form with a live schedule preview (`cron_parse`). Pause all
 * switch. `/cron` in the composer opens this page (`?form=new` from
 * `/cron add`). Needs the `cron` sharing feature on a phone (FeatureOff,
 * same pattern as Browser/Forward); the PC's own app always has it.
 */
import { createEffect, createMemo, createSignal, For, Show, type JSX } from 'solid-js';
import { useNavigate, useSearchParams } from '@solidjs/router';
import { Bell, ChevronRight, Clock, MessageSquare, Pause, Play, Plus, RefreshCw, Square, Trash2 } from 'lucide-solid';
import type { AppError } from '@unipi/protocol';
import type { CronJob, CronRun } from '../lib/cron';
import {
  cronAdd,
  cronDisable,
  cronEdit,
  cronEnable,
  cronList,
  cronLog,
  cronParse,
  cronPauseAll,
  cronRemove,
  cronResumeAll,
  cronOpenSession,
  cronRun,
  cronSettingsGet,
  cronShow,
  cronStop,
  NOTIFY_EVENT_LABEL,
  NOTIFY_EVENTS,
  notifyFromForm,
  notifyModeOf,
  notifyText,
  type NotifyMode,
} from '../lib/cron';
import { folderName } from '../lib/session/newSession';
import { openSessionRoute } from '../lib/notifyInit';
import { useHosts } from '../hosts/context';
import { PageHeader } from '../components/PageHeader';
import { PcChip } from '../components/PcChip';
import { EmptyState, NoHostState, OfflineState, isUnreachable } from '../components/EmptyState';
import { ErrorBanner } from '../components/ErrorBanner';
import { setAddHostOpen } from '../components/Pairing';
import { FeatureOff } from '../components/FeatureOff';
import { BoardSheet } from '../components/board/BoardSheet';
import { SwitchField } from '../components/Switch';
import type { Result } from '../lib/ipc';
import './cron.css';

type CommandBoxComp = (props: { command: string }) => JSX.Element;
function CommandBox(props: { command: string }) {
  return <code class="cron-command">{props.command}</code>;
}

const STATUS_LABEL: Record<string, string> = {
  running: 'running',
  ok: 'ok',
  failed: 'failed',
  timeout: 'timed out',
  skipped: 'skipped',
  queued: 'queued',
};

/** Plain words for the schedule: the host already resolved it; here we just
 * show the next fire, relatively, when there is one. */
export function nextFireText(job: Pick<CronJob, 'next_fire' | 'enabled'>): string {
  if (!job.enabled) return 'disabled';
  if (!job.next_fire) return 'no next run';
  const ms = Date.parse(job.next_fire);
  if (Number.isNaN(ms)) return job.next_fire;
  const diff = ms - Date.now();
  if (diff <= 0) return 'due';
  const min = Math.round(diff / 60_000);
  if (min < 1) return 'in <1 min';
  if (min < 60) return `in ${min} min`;
  const h = Math.round(min / 60);
  if (h < 48) return `in ${h} h`;
  return `in ${Math.round(h / 24)} d`;
}

export function lastRunChip(job: Pick<CronJob, 'last_run_status' | 'running'>): string | undefined {
  if (job.running) return 'running';
  return job.last_run_status ? STATUS_LABEL[job.last_run_status] ?? job.last_run_status : undefined;
}

type View =
  | { kind: 'loading' }
  | { kind: 'disabled' }
  | { kind: 'error'; error: AppError }
  | { kind: 'ready'; jobs: CronJob[]; paused: boolean };

function viewFrom(result: Result<{ jobs: CronJob[]; paused: boolean }> | undefined): View {
  if (!result) return { kind: 'loading' };
  if (!result.ok) {
    if (result.error.code === 'E_UNAUTHORIZED') return { kind: 'disabled' };
    return { kind: 'error', error: result.error };
  }
  return { kind: 'ready', jobs: result.value.jobs, paused: result.value.paused };
}

export default function Cron() {
  const hosts = useHosts();
  const navigate = useNavigate();
  const [searchParams, setSearchParams] = useSearchParams();
  const [hostId, setHostId] = createSignal<string | undefined>(hosts.selectedId());
  createEffect(() => {
    if (!hostId() && hosts.selectedId()) setHostId(hosts.selectedId());
  });
  const hostName = (id: string | undefined) => hosts.hosts().find((h) => h.id === id)?.name ?? 'this PC';

  const [result, setResult] = createSignal<Result<{ jobs: CronJob[]; paused: boolean }> | undefined>();
  const [refreshing, setRefreshing] = createSignal(false);
  const [hostSheetOpen, setHostSheetOpen] = createSignal(false);
  const [detail, setDetail] = createSignal<string | undefined>(undefined);
  const [formOpen, setFormOpen] = createSignal<CronJob | 'new' | undefined>(undefined);

  // `/cron add` lands here with ?form=new: open the add form once, then clear
  // the param so Back/refresh from the form is the plain list (G-17).
  createEffect(() => {
    if (searchParams.form === 'new') {
      setFormOpen('new');
      void setSearchParams({ form: undefined }, { replace: true });
    }
  });

  async function refresh() {
    const id = hostId();
    if (!id) return;
    setRefreshing(true);
    const r = await cronList(id);
    setRefreshing(false);
    if (id === hostId()) setResult(r);
  }
  createEffect(() => {
    hostId();
    setResult(undefined);
    void refresh();
  });

  const view = createMemo<View>(() => viewFrom(result()));

  return (
    <Show when={hosts.hosts().length > 0} fallback={<div class="page-screen"><PageHeader title="Cron" /><NoHostState what="scheduled runs" onAdd={() => setAddHostOpen(true)} /></div>}>
      <div class="page-screen cron-screen" data-testid="cron-screen">
        <PageHeader
          title="Cron"
          lead={<PcChip name={hosts.hosts().find((h) => h.id === hostId())?.name} onClick={() => setHostSheetOpen(true)} />}
          tools={
            <>
              <button type="button" class="icon-btn" aria-label="Refresh" onClick={() => void refresh()}>
                <RefreshCw size={18} class={refreshing() ? 'refresh-spinning' : undefined} />
              </button>
              <Show when={view().kind === 'ready'}>
                <button type="button" class="icon-btn" aria-label="Add a job" onClick={() => setFormOpen('new')} data-testid="cron-add">
                  <Plus size={18} />
                </button>
              </Show>
            </>
          }
        />

        <Show when={view().kind === 'loading'}>
          <div class="preview-loading" data-testid="cron-loading"><RefreshCw size={18} class="refresh-spinning" /> Checking cron on {hostName(hostId())}…</div>
        </Show>

        <Show when={view().kind === 'disabled'}>
          <div data-testid="cron-disabled">
            <FeatureOff feature="cron" hostId={hostId()!} hostName={hostName(hostId())} CommandBox={CommandBox as CommandBoxComp} onChanged={() => void refresh()} />
          </div>
        </Show>

        <Show when={view().kind === 'error'}>
          {(() => {
            const v = view() as { kind: 'error'; error: AppError };
            return isUnreachable(v.error.code) ? (
              <OfflineState pc={hostName(hostId())} onRetry={() => void refresh()} />
            ) : (
              <ErrorBanner error={v.error} retry={() => void refresh()} />
            );
          })()}
        </Show>

        <Show when={view().kind === 'ready'}>
          {(() => {
            const v = view() as { kind: 'ready'; jobs: CronJob[]; paused: boolean };
            return (
              <div class="cron-body">
                <div class="cron-pause-row settings-row">
                  <span class="settings-icon">{v.paused ? <Play size={18} /> : <Pause size={18} />}</span>
                  <div class="min-w-0 flex-1">
                    <p class="text-[15px] text-text">Pause all</p>
                    <p class="text-xs text-text-dim mt-0.5">No new runs start; running ones finish.</p>
                  </div>
                  <SwitchField
                    label="Pause all"
                    checked={v.paused}
                    testId="cron-pause-all"
                    onChange={(checked) => {
                      const id = hostId();
                      if (!id) return;
                      void (checked ? cronPauseAll(id) : cronResumeAll(id)).then(() => void refresh());
                    }}
                  />
                </div>

                <Show when={v.jobs.length === 0} fallback={<For each={v.jobs}>{(job) => <JobRow job={job} onOpen={() => setDetail(job.id)} />}</For>}>
                  <EmptyState
                    title="No cron jobs yet"
                    body="Schedule an unattended agent run — a daily digest, a nightly cleanup, a weekly report."
                    action={
                      <button type="button" class="empty-add bg-shell text-face rounded-pill" onClick={() => setFormOpen('new')} data-testid="cron-empty-add">
                        Add a job
                      </button>
                    }
                  />
                </Show>
              </div>
            );
          })()}
        </Show>

        <HostSheetCron open={hostSheetOpen()} hostIds={hosts.hosts().map((h) => h.id)} names={hosts.hosts()} current={hostId()} onPick={(id) => (setHostId(id), setHostSheetOpen(false))} onClose={() => setHostSheetOpen(false)} />

        <Show when={detail()}>
          {(id) => (
            <JobDetailSheet
              hostId={hostId()!}
              hostName={hostName(hostId())}
              id={id()}
              onClose={() => setDetail(undefined)}
              onEdit={(job) => (setDetail(undefined), setFormOpen(job))}
              onChanged={() => void refresh()}
            />
          )}
        </Show>

        <Show when={formOpen()}>
          {(job) => (
            <JobFormSheet
              hostId={hostId()!}
              job={job() === 'new' ? undefined : (job() as CronJob)}
              onClose={() => setFormOpen(undefined)}
              onSaved={() => {
                setFormOpen(undefined);
                void refresh();
              }}
            />
          )}
        </Show>
      </div>
    </Show>
  );
}

function HostSheetCron(props: { open: boolean; onClose: () => void; hostIds: string[]; names: { id: string; name: string }[]; current?: string; onPick: (id: string) => void }) {
  return (
    <BoardSheet open={props.open} onClose={props.onClose} label="Choose a PC" testId="cron-host-sheet">
      <div class="settings-group rounded-card bg-surface-2/60">
        <For each={props.names}>
          {(host) => (
            <button type="button" class="picker-row" aria-current={props.current === host.id ? 'true' : undefined} onClick={() => props.onPick(host.id)}>
              <span class="min-w-0 flex-1 text-left truncate">{host.name}</span>
            </button>
          )}
        </For>
        <Show when={props.names.length === 0}>
          <p class="p-4 text-text-muted text-sm">Pair a PC first (Settings → Add PC).</p>
        </Show>
      </div>
    </BoardSheet>
  );
}

function JobRow(props: { job: CronJob; onOpen: () => void }) {
  const chip = () => lastRunChip(props.job);
  return (
    <button type="button" class="cron-row" data-testid={`cron-job-${props.job.id}`} onClick={props.onOpen}>
      <span class="settings-icon"><Clock size={18} /></span>
      <div class="min-w-0 flex-1">
        <p class="text-[15px] text-text truncate">{props.job.name}</p>
        <p class="text-xs text-text-dim mt-0.5 truncate">
          {props.job.schedule} · {nextFireText(props.job)}
          <Show when={chip()}> · <span class="cron-chip" data-status={props.job.last_run_status ?? (props.job.running ? 'running' : undefined)}>{chip()}</span></Show>
        </p>
      </div>
      <Show when={!props.job.enabled}><span class="cron-off-badge">off</span></Show>
      <ChevronRight size={16} class="text-text-muted" />
    </button>
  );
}

function JobDetailSheet(props: { hostId: string; hostName: string; id: string; onClose: () => void; onEdit: (job: CronJob) => void; onChanged: () => void }) {
  const [data, setData] = createSignal<Result<{ job: CronJob; runs: CronRun[] }> | undefined>();
  const [pcDefault, setPcDefault] = createSignal<string[]>([]);
  void cronSettingsGet(props.hostId).then((r) => r.ok && r.value && setPcDefault(r.value.notify_default_events ?? []));
  const [busy, setBusy] = createSignal(false);
  const [openRun, setOpenRun] = createSignal<CronRun | undefined>(undefined);

  async function load() {
    setData(await cronShow(props.hostId, props.id));
  }
  createEffect(() => void load());

  async function run(fn: () => Promise<unknown>) {
    setBusy(true);
    await fn();
    setBusy(false);
    await load();
    props.onChanged();
  }

  return (
    <BoardSheet open={true} onClose={props.onClose} label="Job detail" testId="cron-detail-sheet" title={(() => { const d = data(); return d?.ok ? d.value.job.name : 'Job'; })()}>
      <Show when={data()?.ok} fallback={<p class="p-4 text-text-muted text-sm">{(() => { const d = data(); return d && !d.ok ? d.error.message : 'Loading…'; })()}</p>}>
        {(() => {
          const d = data()!;
          if (!d.ok) return null;
          const { job, runs } = d.value;
          return (
            <div class="cron-detail">
              <p class="cron-detail-prompt">{job.prompt}</p>
              <dl class="cron-detail-fields">
                <div><dt>Folder</dt><dd>{job.cwd}</dd></div>
                <div><dt>Schedule</dt><dd>{job.schedule} ({job.tz})</dd></div>
                <Show when={job.model}><div><dt>Model</dt><dd>{job.model}</dd></div></Show>
                <div><dt>Time limit</dt><dd>{job.time_limit_s}s</dd></div>
                <Show when={job.cost_cap_usd !== undefined}><div><dt>Cost cap</dt><dd>${job.cost_cap_usd?.toFixed(2)}</dd></div></Show>
                <div data-testid="cron-detail-notify"><dt>Notify</dt><dd>{notifyText(job.notify, pcDefault())}</dd></div>
                <div data-testid="cron-detail-wake"><dt>Wakes</dt><dd>{wakeText(job)}</dd></div>
              </dl>

              <div class="cron-detail-actions">
                <button type="button" disabled={busy()} onClick={() => void run(() => cronRun(props.hostId, job.id))} data-testid="cron-run-now">
                  <Play size={16} /> Run now
                </button>
                <Show when={job.running}>
                  <button type="button" disabled={busy()} onClick={() => void run(() => cronStop(props.hostId, job.id))} data-testid="cron-stop">
                    <Square size={16} /> Stop
                  </button>
                </Show>
                <button type="button" disabled={busy()} onClick={() => props.onEdit(job)} data-testid="cron-edit">
                  Edit
                </button>
                <button
                  type="button"
                  disabled={busy()}
                  onClick={() => void run(() => (job.enabled ? cronDisable(props.hostId, job.id) : cronEnable(props.hostId, job.id)))}
                  data-testid="cron-toggle-enabled"
                >
                  {job.enabled ? 'Disable' : 'Enable'}
                </button>
                <button
                  type="button"
                  disabled={busy()}
                  class="cron-delete-btn"
                  onClick={() => {
                    if (!confirm(`Delete "${job.name}"? Its run history goes too.`)) return;
                    void run(() => cronRemove(props.hostId, job.id)).then(props.onClose);
                  }}
                  data-testid="cron-delete"
                >
                  <Trash2 size={16} /> Delete
                </button>
              </div>

              <h3 class="cron-runs-heading">Run history</h3>
              <Show when={runs.length > 0} fallback={<p class="text-text-muted text-sm">No runs yet.</p>}>
                <ul class="cron-runs-list">
                  <For each={runs}>
                    {(r) => (
                      <li>
                        <button type="button" class="cron-run-row" onClick={() => setOpenRun(r)} data-testid={`cron-run-${r.id}`}>
                          <span class="cron-chip" data-status={r.status}>{STATUS_LABEL[r.status] ?? r.status}</span>
                          <span class="cron-run-when">{new Date(r.fired_at).toLocaleString()}</span>
                          <span class="cron-run-summary truncate">{r.summary ?? r.reason ?? ''}</span>
                        </button>
                      </li>
                    )}
                  </For>
                </ul>
              </Show>

              <Show when={openRun()}>
                {(r) => <RunLogSheet hostId={props.hostId} hostName={props.hostName} job={job} run={r()} onClose={() => setOpenRun(undefined)} onOpened={() => (setOpenRun(undefined), props.onClose())} />}
              </Show>
            </div>
          );
        })()}
      </Show>
    </BoardSheet>
  );
}

/** "Wakes" in plain words: the chat's name, and whether it's open now. */
export function wakeText(job: Pick<CronJob, 'wake'>): string {
  const w = job.wake;
  if (!w) return 'no chat';
  const name = w.session_name || (w.session_file ? folderName(w.session_file.replace(/\.jsonl$/, '')) : w.session_id ?? 'a chat');
  return w.pid ? `${name} (open)` : `${name} — not open now; skipped until it is`;
}

function RunLogSheet(props: { hostId: string; hostName: string; job: CronJob; run: CronRun; onClose: () => void; onOpened: () => void }) {
  const [data, setData] = createSignal<Result<{ job_id: string; run: CronRun; log: string }> | undefined>();
  const [opening, setOpening] = createSignal(false);
  const [openError, setOpenError] = createSignal<string | undefined>();
  async function openSession() {
    setOpening(true);
    setOpenError(undefined);
    const r = await cronOpenSession(props.hostId, props.run.id);
    setOpening(false);
    if (!r.ok) {
      setOpenError(r.error.message);
      return;
    }
    // The host opens the run's saved session in its hub and names it (WP-H8/A23):
    // the conversation route takes it from here (G-12: cron runs open the session).
    props.onOpened();
    openSessionRoute(props.hostId, r.value.session);
  }
  createEffect(() => {
    void cronLog(props.hostId, props.run.id).then(setData);
  });
  return (
    <BoardSheet open={true} onClose={props.onClose} label="Run log" testId="cron-run-log-sheet" title={`Run ${props.run.id}`}>
      <Show when={data()?.ok} fallback={<p class="p-4 text-text-muted text-sm">{(() => { const d = data(); return d && !d.ok ? d.error.message : 'Loading…'; })()}</p>}>
        {(() => {
          const d = data()!;
          if (!d.ok) return null;
          return (
            <div class="cron-run-log">
              <p class="text-xs text-text-dim">
                {STATUS_LABEL[d.value.run.status] ?? d.value.run.status}
                <Show when={d.value.run.session_path}> · session saved</Show>
                <Show when={d.value.run.cost_usd !== undefined}> · ${d.value.run.cost_usd?.toFixed(4)}</Show>
              </p>
              <Show when={d.value.run.session_path && d.value.run.status !== 'running'}>
                <button type="button" class="cron-open-session" disabled={opening()} onClick={() => void openSession()} data-testid="cron-open-session">
                  <MessageSquare size={16} /> {opening() ? 'Starting the agent…' : 'Open session'}
                </button>
                <p class="cron-run-note">Continue this run as a chat — the agent already on it, or a new one in the job's folder.</p>
              </Show>
              <Show when={openError()}>
                <p class="cron-form-error" role="alert" data-testid="cron-open-session-error">{openError()}</p>
              </Show>
              <pre class="cron-log-text">{d.value.log || '(no output)'}</pre>
            </div>
          );
        })()}
      </Show>
    </BoardSheet>
  );
}

function JobFormSheet(props: { hostId: string; job?: CronJob; onClose: () => void; onSaved: () => void }) {
  // One snapshot: saving closes the sheet, and nothing here should read the
  // parent's <Show> accessor after that.
  const job = props.job;
  const hostId = props.hostId;
  const [name, setName] = createSignal(job?.name ?? '');
  const [prompt, setPrompt] = createSignal(job?.prompt ?? '');
  const [cwd, setCwd] = createSignal(job?.cwd ?? '');
  const [schedule, setSchedule] = createSignal(job?.schedule ?? '');
  const [tz, setTz] = createSignal(job?.tz ?? '');
  const [model, setModel] = createSignal(job?.model ?? '');
  const [timeLimit, setTimeLimit] = createSignal(job ? String(job.time_limit_s) : '');
  const [preview, setPreview] = createSignal<Result<{ expression: string; next_fires: string[] }> | undefined>();
  // UNI-319: notify + wake.
  const [notifyMode, setNotifyMode] = createSignal<NotifyMode>(notifyModeOf(job?.notify));
  const [events, setEvents] = createSignal<string[]>(job?.notify?.events ?? ['failed']);
  const [available, setAvailable] = createSignal<string[]>([]);
  const [channels, setChannels] = createSignal<string[]>(job?.notify?.channels ?? []);
  const [pcDefault, setPcDefault] = createSignal<string[]>([]);
  // The PC's defaults pre-fill the choices — never over what the user already ticked.
  let eventsTouched = false;
  let channelsTouched = false;
  void cronSettingsGet(hostId).then((r) => {
    if (!r.ok || !r.value) return;
    setAvailable(r.value.notify_channels ?? []);
    setPcDefault(r.value.notify_default_events ?? []);
    if (!job?.notify?.channels && !channelsTouched) setChannels(r.value.notify_channels ?? []);
    if (!job?.notify?.events && !eventsTouched && r.value.notify_default_events?.length) setEvents(r.value.notify_default_events);
  });
  const toggle = (list: string[], v: string) => (list.includes(v) ? list.filter((x) => x !== v) : [...list, v]);
  // Waking a chat targeted a v1 bridge pid; picking an oino session to wake
  // comes back with the cron job files (WP-A23). Until then: show it, turn it off.
  type WakePick = 'off' | undefined; // undefined = unchanged
  const [wake, setWake] = createSignal<WakePick>(undefined);
  const wakeLabel = () => (wake() === 'off' ? 'Off' : job?.wake ? wakeText(job) : 'Off');
  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal<string | undefined>(undefined);

  let timer: ReturnType<typeof setTimeout> | undefined;
  createEffect(() => {
    const s = schedule();
    const z = tz();
    clearTimeout(timer);
    if (!s.trim()) {
      setPreview(undefined);
      return;
    }
    timer = setTimeout(() => void cronParse(hostId, { schedule: s, tz: z || undefined }).then(setPreview), 300);
  });

  async function save() {
    if (!name().trim() || !prompt().trim() || !cwd().trim() || !schedule().trim()) {
      setError('Name, prompt, folder and schedule are all required.');
      return;
    }
    setSaving(true);
    setError(undefined);
    const timeLimitN = timeLimit().trim() ? Number(timeLimit()) : undefined;
    if (notifyMode() === 'choose' && events().length === 0) {
      setSaving(false);
      setError('Pick at least one outcome to notify on, or choose Never.');
      return;
    }
    const notify = notifyFromForm(notifyMode(), events(), channels(), available());
    const w = wake();
    const r = job
      ? await cronEdit(hostId, job.id, {
          name: name(),
          prompt: prompt(),
          cwd: cwd(),
          schedule: schedule(),
          tz: tz() || undefined,
          model: model() || undefined,
          time_limit_s: timeLimitN,
          notify,
          ...(w === 'off' ? { wake_clear: true } : {}),
        })
      : await cronAdd(hostId, {
          name: name(),
          prompt: prompt(),
          cwd: cwd(),
          schedule: schedule(),
          tz: tz() || undefined,
          model: model() || undefined,
          time_limit_s: timeLimitN,
          ...(Object.keys(notify).length ? { notify } : {}),
        });
    setSaving(false);
    if (!r.ok) {
      setError(r.error.message);
      return;
    }
    props.onSaved();
  }

  return (
    <BoardSheet
      open={true}
      onClose={props.onClose}
      label={job ? 'Edit job' : 'Add a job'}
      testId="cron-form-sheet"
      title={job ? 'Edit job' : 'Add a job'}
      footer={
        <button type="button" class="cron-save-btn" disabled={saving()} onClick={() => void save()} data-testid="cron-save">
          {saving() ? 'Saving…' : 'Save'}
        </button>
      }
    >
      <div class="cron-form">
        <label>
          <span>Name</span>
          <input value={name()} onInput={(e) => setName(e.currentTarget.value)} data-testid="cron-form-name" />
        </label>
        <label>
          <span>Prompt</span>
          <textarea value={prompt()} onInput={(e) => setPrompt(e.currentTarget.value)} rows={4} data-testid="cron-form-prompt" />
        </label>
        <label>
          <span>Folder</span>
          <input value={cwd()} onInput={(e) => setCwd(e.currentTarget.value)} placeholder="/home/me/jobs/digest" data-testid="cron-form-cwd" />
        </label>
        <label>
          <span>Schedule</span>
          <input value={schedule()} onInput={(e) => setSchedule(e.currentTarget.value)} placeholder="every day at 9:00" data-testid="cron-form-schedule" />
        </label>
        <Show when={preview()}>
          {(p) =>
            p().ok ? (
              <p class="cron-preview" data-testid="cron-form-preview">
                {p().ok && (p() as { ok: true; value: { expression: string; next_fires: string[] } }).value.expression} — next: {(p() as { ok: true; value: { expression: string; next_fires: string[] } }).value.next_fires.join(', ')}
              </p>
            ) : (
              <p class="cron-preview-error">{(p() as { ok: false; error: { message: string } }).error.message}</p>
            )
          }
        </Show>
        <label>
          <span>Timezone</span>
          <input value={tz()} onInput={(e) => setTz(e.currentTarget.value)} placeholder="this PC's zone" data-testid="cron-form-tz" />
        </label>
        <label>
          <span>Model (optional)</span>
          <input value={model()} onInput={(e) => setModel(e.currentTarget.value)} data-testid="cron-form-model" />
        </label>
        <label>
          <span>Time limit (seconds)</span>
          <input value={timeLimit()} onInput={(e) => setTimeLimit(e.currentTarget.value)} placeholder="1800" data-testid="cron-form-time-limit" />
        </label>
        <fieldset class="cron-fieldset" data-testid="cron-form-notify">
          <legend><Bell size={14} /> Notify when a run ends</legend>
          <div class="cron-seg" role="radiogroup" aria-label="Notify">
            <For each={[['default', 'PC default'], ['choose', 'Choose'], ['never', 'Never']] as const}>
              {([mode, label]) => (
                <button type="button" role="radio" aria-checked={notifyMode() === mode} data-on={notifyMode() === mode ? '' : undefined} onClick={() => setNotifyMode(mode)} data-testid={`cron-notify-${mode}`}>
                  {label}
                </button>
              )}
            </For>
          </div>
          <Show when={notifyMode() === 'default'}>
            <p class="cron-hint">{notifyText(undefined, pcDefault()).replace('PC default: ', 'On ')} — set in Settings → This computer → Cron.</p>
          </Show>
          <Show when={notifyMode() === 'choose'}>
            <div class="cron-checks">
              <For each={NOTIFY_EVENTS}>
                {(e) => (
                  <label class="cron-check">
                    <input type="checkbox" checked={events().includes(e)} onChange={() => ((eventsTouched = true), setEvents(toggle(events(), e)))} data-testid={`cron-notify-event-${e}`} />
                    <span>{NOTIFY_EVENT_LABEL[e]}</span>
                  </label>
                )}
              </For>
            </div>
          </Show>
          <Show when={notifyMode() !== 'never'}>
            <Show
              when={available().length > 0}
              fallback={<p class="cron-hint" data-testid="cron-notify-no-channels">No notification channel is set up on this PC yet (ntfy, Gotify or Telegram — /unipi:settings → Notify in pi).</p>}
            >
              <Show when={available().length > 1}>
                <div class="cron-checks" aria-label="Channels">
                  <span class="cron-checks-label">Send with</span>
                  <For each={available()}>
                    {(c) => (
                      <label class="cron-check">
                        <input type="checkbox" checked={channels().includes(c)} onChange={() => ((channelsTouched = true), setChannels(toggle(channels(), c)))} data-testid={`cron-notify-channel-${c}`} />
                        <span>{c === 'ntfy' ? 'ntfy' : c[0]!.toUpperCase() + c.slice(1)}</span>
                      </label>
                    )}
                  </For>
                </div>
              </Show>
              <Show when={available().length === 1}>
                <p class="cron-hint">Sent with {available()[0]}.</p>
              </Show>
            </Show>
          </Show>
        </fieldset>

        <fieldset class="cron-fieldset" data-testid="cron-form-wake">
          <legend><MessageSquare size={14} /> Wake a chat when a run ends</legend>
          <div class="cron-wake-row">
            <span class="min-w-0 flex-1 truncate" data-testid="cron-wake-label">{wakeLabel()}</span>
            <Show when={wakeLabel() !== 'Off'}>
              <button type="button" class="cron-small-btn" onClick={() => setWake('off')} data-testid="cron-wake-off">Off</button>
            </Show>
          </div>
          <p class="cron-hint">The chat gets a short message with the result — run now if it's idle, queued if it's busy.</p>
        </fieldset>

        <Show when={error()}>
          <p class="cron-form-error" role="alert">{error()}</p>
        </Show>
      </div>
    </BoardSheet>
  );
}
