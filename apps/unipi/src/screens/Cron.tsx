/**
 * Cron (WP-A23, G-17/Z-112): the PC's scheduled, unattended runs — job files
 * the `unipi.cron` extension or this screen write, fired by `unipi-host
 * serve`'s embedded scheduler. List → detail (prompt, folder, run history) →
 * add/edit form with a live schedule preview (`cron_parse`). Pause all
 * switch. `/cron` in the composer opens this page (`?form=new` from
 * `/cron add`). Needs the `cron` sharing feature on a phone (FeatureOff,
 * same pattern as Browser/Forward); the PC's own app always has it.
 */
import { createEffect, createMemo, createResource, createSignal, For, on, onCleanup, Show, untrack, type JSX } from 'solid-js';
import { NamespaceForm } from '../ui/settings/form';
import { settingsSchema } from '../ui/settings/data';
import type { Namespace } from '../ui/settings/types';
import { useNavigate, useSearchParams } from '@solidjs/router';
import { Bell, CalendarClock, ChevronDown, ChevronRight, CircleAlert, CircleCheck, CirclePause, ClockAlert, Cpu, Hourglass, LoaderCircle, MessageSquare, Pause, Play, Plus, RefreshCw, Settings2, SkipForward, Square, Trash2 } from 'lucide-solid';
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
  humanCronRunReason,
  NOTIFY_EVENT_LABEL,
  NOTIFY_EVENTS,
  notifyFromForm,
  notifyModeOf,
  notifyText,
  parseCronRunLog,
  type NotifyMode,
} from '../lib/cron';
import { fsRoots } from '../lib/fs';
import { folderChoices, folderName } from '../lib/session/newSession';
import { hubs } from '../lib/session/hub';
import { sessions } from '../lib/session/store';
import { cachedModels, rememberModels } from '../components/draft/hostCache';
import { WorkspacePicker, homeOf } from '../components/draft/WorkspacePicker';
import { modelView } from '../components/model/ModelList';
import { ModelDialog, ModelPickerPanel, PickerSecondary } from '../components/model/ModelPopover';
import { modelKey, modelName, parseModels, type ModelInfo } from '../lib/model/picker';
import { hostReadiness } from '../lib/model/readiness';
import '../components/draft/draft.css';
import '../components/model/model.css';
import { openSessionRoute } from '../lib/notifyInit';
import { useHosts } from '../hosts/context';
import { PageHeader } from '../components/PageHeader';
import { PcChip } from '../components/PcChip';
import { desktopShell } from '../lib/layout';
import { within } from '../lib/host/within';
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

const DAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];
const clock = (hour: string, minute: string) => `${Number(hour)}:${minute.padStart(2, '0')}`;

/** A cron expression in plain words ("0 0 9 * * *" → "Every day at 9:00"); anything it can't say simply stays as written. */
export function humanSchedule(expression: string): string {
  const parts = expression.trim().split(/\s+/);
  if (parts.length !== 5 && parts.length !== 6) return expression;
  const [min, hour, dom, mon, dow] = parts.length === 6 ? parts.slice(1) : parts;
  const sec = parts.length === 6 ? parts[0] : '0';
  if (sec !== '0' || mon !== '*' || !/^\d+$/.test(min!)) return expression;
  if (/^\d+$/.test(hour!) && dom === '*') {
    const at = clock(hour!, min!);
    if (dow === '*') return `Every day at ${at}`;
    if (/^[0-7]$/.test(dow!)) return `Every ${DAYS[Number(dow) % 7]} at ${at}`;
    if (dow === '1-5') return `Weekdays at ${at}`;
  }
  if (hour === '*' && dom === '*' && dow === '*') return min === '0' ? 'Every hour' : `Every hour at :${min!.padStart(2, '0')}`;
  return expression;
}

/** [ui-chat-bits] The schedule in plain words, a step further than `humanSchedule`: one-shot
 * `at:` times, every N minutes / hours, several weekdays, a day of the month. Pure. */
export function scheduleText(expression: string, now = Date.now()): string {
  const e = expression.trim();
  if (e.startsWith('at:')) {
    const ms = Date.parse(e.slice(3));
    return Number.isNaN(ms) ? e : `Once, ${runWhenText(ms, now).abs}`;
  }
  const simple = humanSchedule(e);
  if (simple !== e) return simple;
  const parts = e.split(/\s+/);
  if (parts.length !== 5 && parts.length !== 6) return e;
  const [min, hour, dom, mon, dow] = parts.length === 6 ? parts.slice(1) : parts;
  const sec = parts.length === 6 ? parts[0] : '0';
  if (sec !== '0' || mon !== '*') return e;
  const step = (f: string) => /^(?:\*|0)\/(\d+)$/.exec(f)?.[1];
  if (step(min!) && hour === '*' && dom === '*' && dow === '*') return `Every ${Number(step(min!))} min`;
  if (/^\d+$/.test(min!) && step(hour!) && dom === '*' && dow === '*') {
    const n = Number(step(hour!));
    return `Every ${n} h${min === '0' ? '' : ` at :${min!.padStart(2, '0')}`}`;
  }
  if (/^\d+$/.test(min!) && /^\d+$/.test(hour!)) {
    const at = clock(hour!, min!);
    if (dom === '*' && (dow === '0,6' || dow === '6,0')) return `Weekends at ${at}`;
    if (dom === '*' && /^[0-7](,[0-7])+$/.test(dow!)) return `${dow!.split(',').map((d) => DAYS[Number(d) % 7]!.slice(0, 3)).join(', ')} at ${at}`;
    if (/^\d+$/.test(dom!) && dow === '*') return `Monthly on day ${Number(dom)} at ${at}`;
  }
  return e;
}

/** A run's time: "Nov 1, 19:40" (the year when not this one) + relative ("2 h ago"). Pure. */
export function runWhenText(at: number, now = Date.now()): { abs: string; rel: string } {
  const d = new Date(at);
  const time = d.toLocaleTimeString('en-US', { hour: '2-digit', minute: '2-digit', hour12: false });
  const sameYear = d.getFullYear() === new Date(now).getFullYear();
  const date = d.toLocaleDateString('en-US', sameYear ? { month: 'short', day: 'numeric' } : { month: 'short', day: 'numeric', year: 'numeric' });
  const diff = now - at;
  const ahead = diff < 0;
  const a = Math.abs(diff);
  const min = Math.round(a / 60_000);
  const span = min < 1 ? undefined : min < 60 ? `${min} min` : min < 48 * 60 ? `${Math.round(min / 60)} h` : `${Math.round(min / 1440)} d`;
  const rel = !span ? 'just now' : ahead ? `in ${span}` : `${span} ago`;
  return { abs: `${date}, ${time}`, rel };
}

/** How long a run took ("42s", "3m 05s", "1h 02m"); `undefined` until it ended. Pure. */
export function runDurationText(run: Pick<CronRun, 'started_at' | 'ended_at'>): string | undefined {
  if (run.started_at === undefined || run.ended_at === undefined) return undefined;
  const s = Math.max(0, Math.round((run.ended_at - run.started_at) / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${String(s % 60).padStart(2, '0')}s`;
  return `${Math.floor(m / 60)}h ${String(m % 60).padStart(2, '0')}m`;
}

/** A run status as an icon + short label (never wraps) + tone. */
const RUN_STATUS: Record<string, { label: string; icon: typeof CircleCheck }> = {
  ok: { label: 'OK', icon: CircleCheck },
  failed: { label: 'Failed', icon: CircleAlert },
  timeout: { label: 'Timed out', icon: ClockAlert },
  skipped: { label: 'Skipped', icon: SkipForward },
  queued: { label: 'Queued', icon: Hourglass },
  running: { label: 'Running', icon: LoaderCircle },
};

export function RunStatusChip(props: { status: string; compact?: boolean; testId?: string }) {
  const meta = () => RUN_STATUS[props.status] ?? { label: props.status, icon: CircleAlert };
  return (
    <span class="cron-status" data-status={props.status} data-compact={props.compact ? '' : undefined} title={meta().label} data-testid={props.testId}>
      {(() => {
        const Icon = meta().icon;
        return <Icon size={14} aria-hidden="true" class={props.status === 'running' ? 'refresh-spinning' : undefined} />;
      })()}
      <span class="cron-status-label">{meta().label}</span>
    </span>
  );
}

/** The job's state for its row icon: running › failed last run › disabled › scheduled. Pure. */
export function jobState(job: Pick<CronJob, 'enabled' | 'running' | 'last_run_status'>): 'running' | 'failed' | 'disabled' | 'scheduled' {
  if (job.running) return 'running';
  if (!job.enabled) return 'disabled';
  if (job.last_run_status === 'failed' || job.last_run_status === 'timeout') return 'failed';
  return 'scheduled';
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
  // The PC is the app-wide selection (sidebar host row / phone chip, [ui-shell] #1).
  const hostId = () => hosts.selectedId();
  const setHostId = (id: string) => hosts.selectHost(id);
  const isLocalHost = (id: string | undefined) => hosts.hosts().find((h) => h.id === id)?.transport === 'local';
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

  // [ui-chat-bits] A cron-run chat's header chip lands here with ?job=<id>[&host=<id>]:
  // pick that PC, open the job's detail once, then clear the params (Back = the list).
  createEffect(() => {
    const job = typeof searchParams.job === 'string' ? searchParams.job : undefined;
    const host = typeof searchParams.host === 'string' ? searchParams.host : undefined;
    if (!job && !host) return;
    untrack(() => {
      if (host && hosts.hosts().some((h) => h.id === host)) setHostId(host);
      if (job) setDetail(job);
    });
    void setSearchParams({ job: undefined, host: undefined }, { replace: true });
  });

  async function refresh() {
    const id = hostId();
    if (!id) return;
    setRefreshing(true);
    const r = await within<Result<{ jobs: CronJob[]; paused: boolean }>>(cronList(id), () => ({
      ok: false,
      error: { code: 'E_TIMEOUT', message: 'This computer did not answer in time.', retryable: true } as AppError,
    }));
    setRefreshing(false);
    if (id === hostId()) setResult(r);
  }
  createEffect(() => {
    hostId();
    setResult(undefined);
    void refresh();
  });

  const view = createMemo<View>(() => viewFrom(result()));

  // E-14: a run in flight is followed live — the row leaves "running" when the run ends.
  createEffect(() => {
    const v = view();
    if (v.kind !== 'ready' || !v.jobs.some((j) => j.running)) return;
    const t = setInterval(() => void refresh(), 2000);
    onCleanup(() => clearInterval(t));
  });

  return (
    <Show when={hosts.hosts().length > 0} fallback={<div class="page-screen"><PageHeader title="Cron" /><NoHostState what="scheduled runs" onAdd={() => setAddHostOpen(true)} /></div>}>
      <div class="page-screen cron-screen" data-testid="cron-screen">
        <PageHeader
          title="Cron"
          lead={desktopShell() ? undefined : <PcChip name={hosts.hosts().find((h) => h.id === hostId())?.name} state={view().kind === "error" ? "offline" : view().kind === "loading" ? "checking" : "ok"} onClick={() => setHostSheetOpen(true)} />}
          tools={
            <>
              <button type="button" class="ghost-icon-button" aria-label="Refresh" onClick={() => void refresh()}>
                <RefreshCw size={18} class={refreshing() ? 'refresh-spinning' : undefined} />
              </button>
              <Show when={view().kind === 'ready'}>
                <button type="button" class="ghost-icon-button" aria-label="Add a job" onClick={() => setFormOpen('new')} data-testid="cron-add">
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
              <OfflineState pc={hostName(hostId())} local={isLocalHost(hostId())} onRetry={() => void refresh()} />
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

                <Show when={v.jobs.length === 0} fallback={<div class="cron-jobs">
                  <For each={v.jobs}>{(job) => <JobRow job={job} hostId={hostId()!} onOpen={() => setDetail(job.id)} onChanged={() => void refresh()} />}</For>
                </div>}>
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
                <SchedulerSettings hostId={hostId()!} />
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

/** The `cron` namespace without what this screen already shows (Pause all). Pure. */
export function schedulerNamespace(ns: Namespace): Namespace {
  return { ...ns, sections: ns.sections.map((section) => ({ ...section, fields: section.fields.filter((f) => f.key !== 'paused') })).filter((section) => section.fields.length > 0) };
}

/**
 * P2-03: the scheduler's settings (concurrency, missed window, retention, time
 * limit, default notify events) — the one place for them, on this screen for
 * the PC it shows (the cron extension's own form; Settings › Extensions links here).
 */
function SchedulerSettings(props: { hostId: string }) {
  const [open, setOpen] = createSignal(false);
  const [ns] = createResource(
    () => (open() ? props.hostId : undefined),
    async (hostId) => {
      const r = await settingsSchema(hostId);
      const found = r.ok ? r.value.find((n) => n.namespace === 'cron') : undefined;
      return found ? schedulerNamespace(found) : null;
    },
  );
  return (
    <div class="cron-scheduler" id="cron-scheduler" data-testid="cron-scheduler">
      <button type="button" class="cron-scheduler-toggle settings-row" aria-expanded={open()} onClick={() => setOpen(!open())} data-testid="cron-scheduler-toggle">
        <span class="settings-icon"><Settings2 size={18} /></span>
        <span class="min-w-0 flex-1 text-left">
          <span class="block text-[15px] text-text">Scheduler settings</span>
          <span class="block text-xs text-text-dim mt-0.5">How many run at once, missed runs, history kept, time limit, default notifications</span>
        </span>
        <ChevronDown size={16} class="cron-scheduler-chevron text-text-dim shrink-0" data-open={open() ? '' : undefined} />
      </button>
      <Show when={open()}>
        <Show when={ns()} fallback={<p class="cron-hint px-4 pb-3">{ns.loading ? 'Loading…' : 'This PC’s cron extension reports no settings.'}</p>}>
          {(n) => <NamespaceForm namespace={n()} hostId={props.hostId} bare flat testId="cron-scheduler-form" />}
        </Show>
      </Show>
    </div>
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

function JobRow(props: { job: CronJob; hostId: string; onOpen: () => void; onChanged: () => void }) {
  const [busy, setBusy] = createSignal(false);
  const [enabled, setEnabled] = createSignal(props.job.enabled);
  createEffect(() => setEnabled(props.job.enabled));
  const state = () => jobState({ ...props.job, enabled: enabled() });
  const last = () => (props.job.running ? 'running' : props.job.last_run_status);
  const toggle = (on: boolean) => {
    setEnabled(on);
    setBusy(true);
    void (on ? cronEnable(props.hostId, props.job.id) : cronDisable(props.hostId, props.job.id)).then((r) => {
      setBusy(false);
      if (!(r as { ok?: boolean }).ok) setEnabled(!on);
      props.onChanged();
    });
  };
  return (
    <div class="cron-row" data-state={state()}>
      <button type="button" class="cron-row-main" data-testid={`cron-job-${props.job.id}`} onClick={props.onOpen}>
        <span class="cron-row-icon" data-state={state()} title={state() === 'failed' ? 'Last run failed' : state() === 'disabled' ? 'Disabled' : state() === 'running' ? 'Running now' : 'Scheduled'}>
          {state() === 'running' ? <LoaderCircle size={18} class="refresh-spinning" /> : state() === 'failed' ? <CircleAlert size={18} /> : state() === 'disabled' ? <CirclePause size={18} /> : <CalendarClock size={18} />}
        </span>
        <span class="cron-row-text">
          <span class="cron-row-name">{props.job.name}</span>
          <span class="cron-row-sub" title={`${props.job.schedule} (${props.job.tz})`}>
            {scheduleText(props.job.schedule)}
            <span class="cron-row-next"> · {enabled() ? nextFireText(props.job) : 'off'}</span>
          </span>
        </span>
        <Show when={last()}>{(st) => <RunStatusChip status={st()} compact testId={`cron-job-last-${props.job.id}`} />}</Show>
      </button>
      <span class="cron-row-toggle">
        <SwitchField label={enabled() ? `Disable ${props.job.name}` : `Enable ${props.job.name}`} checked={enabled()} disabled={busy()} onChange={toggle} testId={`cron-job-toggle-${props.job.id}`} />
      </span>
    </div>
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
  // Poll while the job runs (or just started), so Run now shows its end without leaving the sheet.
  const [justRan, setJustRan] = createSignal(false);
  createEffect(() => {
    const d = data();
    const live = (d?.ok && d.value.job.running) || justRan();
    if (!live) return;
    const t = setInterval(() => {
      void load();
      props.onChanged();
    }, 2000);
    const stop = setTimeout(() => setJustRan(false), 6000);
    onCleanup(() => (clearInterval(t), clearTimeout(stop)));
  });

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
                <div><dt>Schedule</dt><dd title={job.schedule}>{humanSchedule(job.schedule)} ({job.tz})</dd></div>
                <Show when={job.model}><div><dt>Model</dt><dd>{job.model}</dd></div></Show>
                <div><dt>Time limit</dt><dd>{job.time_limit_s}s</dd></div>
                <Show when={job.cost_cap_usd !== undefined}><div><dt>Cost cap</dt><dd>${job.cost_cap_usd?.toFixed(2)}</dd></div></Show>
                <div data-testid="cron-detail-notify"><dt>Notify</dt><dd>{notifyText(job.notify, pcDefault())}</dd></div>
                <div data-testid="cron-detail-wake"><dt>Wakes</dt><dd>{wakeText(job)}</dd></div>
              </dl>

              <div class="cron-detail-actions">
                <button type="button" disabled={busy()} onClick={() => (setJustRan(true), void run(() => cronRun(props.hostId, job.id)))} data-testid="cron-run-now">
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
                          <RunStatusChip status={r.status} />
                          <span class="cron-run-main">
                            <span class="cron-run-when" title={new Date(r.fired_at).toLocaleString()}>
                              {runWhenText(r.fired_at).abs}
                              <span class="cron-run-rel"> · {runWhenText(r.fired_at).rel}</span>
                              <Show when={runDurationText(r)}>{(d) => <span class="cron-run-rel"> · took {d()}</span>}</Show>
                              <Show when={r.missed}><span class="cron-run-tag">catch-up</span></Show>
                            </span>
                            <Show when={r.summary ?? humanCronRunReason(r.reason, undefined, job.model)}>
                              {(text) => <span class="cron-run-summary">{text()}</span>}
                            </Show>
                          </span>
                          <ChevronRight size={16} class="cron-run-chevron" aria-hidden="true" />
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
    <BoardSheet open={true} onClose={props.onClose} label="Run log" testId="cron-run-log-sheet" title={props.job.name}>
      <Show when={data()?.ok} fallback={<p class="p-4 text-text-muted text-sm">{(() => { const d = data(); return d && !d.ok ? d.error.message : 'Loading…'; })()}</p>}>
        {(() => {
          const d = data()!;
          if (!d.ok) return null;
          const run = d.value.run;
          const parsed = parseCronRunLog(d.value.log, run.reason, props.job.model);
          const failed = run.status === 'failed' || run.status === 'timeout';
          return (
            <div class="cron-run-log">
              <p class="text-xs text-text-dim">
                <RunStatusChip status={run.status} />
                {' · '}
                {runWhenText(run.fired_at).abs} ({runWhenText(run.fired_at).rel})
                <Show when={run.session_path}> · session saved</Show>
                <Show when={run.cost_usd !== undefined}> · ${run.cost_usd?.toFixed(4)}</Show>
              </p>
              <Show when={failed && parsed.cause}>
                <p class="cron-run-cause" role="alert" data-testid="cron-run-cause">
                  {run.status === 'timeout' ? 'Timed out' : 'Failed'} — {parsed.cause}
                </p>
              </Show>
              <Show when={run.session_path && run.status !== 'running'}>
                <button type="button" class="cron-open-session" disabled={opening()} onClick={() => void openSession()} data-testid="cron-open-session">
                  <MessageSquare size={16} /> {opening() ? 'Starting the agent…' : 'Open session'}
                </button>
                <p class="cron-run-note">Continue this run as a chat — the agent already on it, or a new one in the job's folder.</p>
              </Show>
              <Show when={openError()}>
                <p class="cron-form-error" role="alert" data-testid="cron-open-session-error">{openError()}</p>
              </Show>
              <pre class="cron-log-text" data-testid="cron-log-text">{parsed.transcript || '(no output)'}</pre>
            </div>
          );
        })()}
      </Show>
    </BoardSheet>
  );
}

/**
 * D-32: the job's model — the ONE model dialog (`ModelDialog`) titled
 * "Job model", with "Use default" (this PC's startup model) in its footer.
 * The button shows the chosen model's display name + provider.
 */
function CronModelPicker(props: { hostId: string; value: string; onChange: (modelKey: string) => void }) {
  const [open, setOpen] = createSignal(false);
  const [query, setQuery] = createSignal('');
  let btn: HTMLButtonElement | undefined;
  const [data] = createResource(
    () => (open() ? props.hostId : undefined),
    async (id): Promise<{ models: ModelInfo[]; readiness?: Awaited<ReturnType<typeof hostReadiness>> }> => {
      const reg = sessions();
      const key = reg.keys().find((k) => k.startsWith(`${id}/`));
      const peer = key ? reg.peek(id, key.slice(id.length + 1)) : undefined;
      const [reply, readiness] = await Promise.all([
        peer && peer.state.conn === 'live' ? peer.cmd<unknown>({ type: 'get_available_models' }).catch(() => undefined) : Promise.resolve(undefined),
        hostReadiness(id).catch(() => undefined),
      ]);
      const live = reply ? parseModels(reply) : [];
      if (live.length) rememberModels(id, live);
      const models = live.length ? live : (cachedModels(id) ?? []);
      return { models, ...(readiness ? { readiness } : {}) };
    },
  );
  const view = createMemo(() =>
    modelView(
      data.latest?.models ?? [],
      data.latest?.readiness,
      undefined,
      props.value ? { kind: 'single', key: props.value } : undefined,
      query(),
    ),
  );
  const buttonLabel = () => {
    const v = props.value.trim();
    if (!v) return 'Choose model…';
    const known = (data.latest?.models ?? cachedModels(props.hostId) ?? []).find((m) => modelKey(m) === v);
    return known ? modelName(known) : v.slice(v.indexOf('/') + 1);
  };
  const provider = () => {
    const v = props.value.trim();
    return v.includes('/') ? v.slice(0, v.indexOf('/')) : '';
  };
  const close = () => setOpen(false);
  return (
    <>
      <button
        type="button"
        class="ws-chip"
        aria-haspopup="dialog"
        aria-expanded={open()}
        ref={btn}
        onClick={() => (open() ? close() : (setQuery(''), setOpen(true)))}
        data-testid="cron-model-picker-btn"
      >
        <Cpu size={14} class="ws-chip-ic" />
        <span class="ws-chip-label">{buttonLabel()}</span>
        <Show when={provider()}>
          <span class="mp-field-prov" data-testid="cron-model-provider">{provider()}</span>
        </Show>
        <ChevronDown size={13} class="ws-chip-chev" />
      </button>
      <Show when={open()}>
        <ModelDialog title="Job model" testId="cron-model-menu" onClose={close} trigger={() => btn}>
          <ModelPickerPanel
            view={view()}
            query={query()}
            onQuery={setQuery}
            now={props.value.trim() ? `${buttonLabel()}${provider() ? ` · ${provider()}` : ''}` : 'Default — this PC’s startup model'}
            loading={!!data.loading && !data.latest}
            showEmpty={(data.latest?.models.length ?? 0) === 0}
            empty={<p class="mp-hint mp-empty">Open a chat once on this PC to browse every model.</p>}
            currentKey={props.value.trim() || undefined}
            onSelect={(m) => {
              props.onChange(modelKey(m));
              close();
            }}
            scope="“Use default” runs the job on this PC’s startup model."
            actions={
              <PickerSecondary
                label="Use default"
                pressed={!props.value.trim()}
                title="Run the job on this PC’s startup model"
                onClick={() => {
                  props.onChange('');
                  close();
                }}
                testId="cron-model-default"
              />
            }
          />
        </ModelDialog>
      </Show>
    </>
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

  // D-32: folder roots + recent session folders for the Folder picker.
  const hub = createMemo(on(() => hostId, (id) => untrack(() => hubs().peek(id))));
  const [roots] = createResource(() => hostId, async (id) => {
    const res = await fsRoots(id).catch(() => undefined);
    return res?.ok ? res.value.roots : [];
  });
  const home = createMemo(() => homeOf(roots.latest ?? []));
  const recentFolders = createMemo(() => {
    const cards = hub()?.state.loaded ? Object.values(hub()!.state.cards) : [];
    const sorted = [...cards].sort((a, b) => (b.updated_at ?? 0) - (a.updated_at ?? 0));
    return folderChoices(sorted, [], undefined);
  });

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
        <div class="cron-form-footer">
          <Show when={error()}>
            <p class="cron-form-error" role="alert" data-testid="cron-form-error">{error()}</p>
          </Show>
          <button type="button" class="cron-save-btn" disabled={saving()} onClick={() => void save()} data-testid="cron-save">
            {saving() ? 'Saving…' : 'Save'}
          </button>
        </div>
      }
    >
      <div class="cron-form">
        <label for="cron-field-name">
          <span>Name</span>
          <input id="cron-field-name" aria-label="Name" value={name()} onInput={(e) => setName(e.currentTarget.value)} placeholder="e.g. Daily inbox digest" data-testid="cron-form-name" />
        </label>
        <label for="cron-field-prompt">
          <span>Prompt</span>
          <textarea id="cron-field-prompt" aria-label="Prompt" value={prompt()} onInput={(e) => setPrompt(e.currentTarget.value)} rows={4} placeholder="What the unattended run should do…" data-testid="cron-form-prompt" />
        </label>
        <div class="cron-field-group">
          <div class="cron-field-head">
            <label for="cron-field-cwd">Folder</label>
            <WorkspacePicker
              hostId={hostId}
              value={cwd().trim() || undefined}
              onChange={(p) => p && setCwd(p)}
              recent={recentFolders()}
              roots={roots.latest ?? []}
              home={home()}
              allowNone={false}
              placeholder="Browse folders…"
            />
          </div>
          <input id="cron-field-cwd" aria-label="Folder" value={cwd()} onInput={(e) => setCwd(e.currentTarget.value)} placeholder="e.g. /home/me/projects/app" data-testid="cron-form-cwd" />
        </div>
        <label for="cron-field-schedule">
          <span>Schedule</span>
          <input id="cron-field-schedule" aria-label="Schedule" value={schedule()} onInput={(e) => setSchedule(e.currentTarget.value)} placeholder="e.g. every day at 9:00" data-testid="cron-form-schedule" />
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
        <label for="cron-field-tz">
          <span>Timezone</span>
          <input id="cron-field-tz" aria-label="Timezone" value={tz()} onInput={(e) => setTz(e.currentTarget.value)} placeholder="This PC’s time zone (default)" data-testid="cron-form-tz" />
        </label>
        <div class="cron-field-group">
          <div class="cron-field-head">
            <label for="cron-field-model">Model (optional)</label>
            <CronModelPicker hostId={hostId} value={model()} onChange={setModel} />
          </div>
          <input id="cron-field-model" aria-label="Model (optional)" value={model()} onInput={(e) => setModel(e.currentTarget.value)} placeholder="Default model" data-testid="cron-form-model" />
        </div>
        <label for="cron-field-time-limit">
          <span>Time limit (seconds)</span>
          <input id="cron-field-time-limit" aria-label="Time limit (seconds)" value={timeLimit()} onInput={(e) => setTimeLimit(e.currentTarget.value)} placeholder="e.g. 1800 (30 minutes)" data-testid="cron-form-time-limit" />
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
            <p class="cron-hint">{notifyText(undefined, pcDefault()).replace('PC default: ', 'On ')} — set in Scheduler settings on the Cron screen.</p>
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
              fallback={<p class="cron-hint" data-testid="cron-notify-no-channels">No notification channel is set up on this PC yet (ntfy, Gotify or Telegram — Settings › Notifications).</p>}
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
      </div>
    </BoardSheet>
  );
}
