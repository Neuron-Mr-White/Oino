/**
 * Dream (owner 2026-11-05: "make sure dream settings here has some real purpose"): the
 * native surface over `unipi.dream`'s services instead of three rows that only echoed
 * command text. What a dream is, whether it is on and due, Run now / Stop, every run with
 * its digest count and summary, the report, and the proposals waiting for a decision
 * (Approve lands the skill / check, Reject drops it). Its settings form sits right here.
 *
 * Services (`dream.status`, `dream.run`, `dream.stop`, `dream.detail`, `dream.approve`,
 * `dream.reject`, `dream.dismiss`) run in Settings' own command session.
 *
 * v3 J3: the settings form is always on the page (no hidden toggle): enabled switch,
 * schedule, model picker, thinking, max runtime, stale days (the schema form from the
 * extension). "Recent dreams" shows status, start, duration, memory changes and proposals.
 */
import { createMemo, createResource, createSignal, For, Show } from 'solid-js';
import { Check, ChevronDown, ChevronRight, CircleAlert, LoaderCircle, MoonStar, Play, Square, X } from 'lucide-solid';
import { BoardSheet } from '../board/BoardSheet';
import { Markdown } from '../board/Markdown';
import { settingsCommandChat } from '../../lib/settings/run';
import { toasts } from '../../lib/toasts';
import { NamespaceForm } from '../../ui/settings/form';
import { settingsSchema } from '../../ui/settings/data';

export type DreamProposal = { id: string; name: string; kind: string; decision: string };
export type DreamRun = {
  id: string;
  status: string;
  startedAt?: number;
  endedAt?: number;
  manual?: boolean;
  sessions?: number;
  events?: number;
  pending?: number;
  hasReport?: boolean;
  proposals: DreamProposal[];
  summary?: string[];
  error?: string;
  /** Memory edits the run applied (`changes.json` counts). */
  memoryChanges?: { added: number; updated: number; merged: number; pruned: number };
};
export type DreamScheduler = { registered: boolean; hostRunning: boolean; nextRun?: number; error?: string };
export type DreamStatus = {
  enabled: boolean;
  scheduleMode?: string;
  scheduleTime?: string;
  intervalHours?: number;
  minSessions?: number;
  minGapHours?: number;
  lastRunAt?: number;
  running: boolean;
  runs: DreamRun[];
  due?: { due: boolean; reason?: string };
  scheduler?: DreamScheduler;
};

const obj = (v: unknown): Record<string, unknown> => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : {});
const num = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) ? v : undefined);

/** `dream.status`, leniently. Pure. */
export function parseDreamStatus(raw: unknown): DreamStatus {
  const o = obj(raw);
  const runs = (Array.isArray(o.runs) ? o.runs : []).map(obj).filter((r) => typeof r.id === 'string').map((r) => ({
    id: String(r.id),
    status: typeof r.status === 'string' ? r.status : 'done',
    ...(num(r.startedAt) !== undefined ? { startedAt: num(r.startedAt) } : {}),
    ...(num(r.endedAt) !== undefined ? { endedAt: num(r.endedAt) } : {}),
    manual: r.manual === true,
    sessions: num(r.sessions) ?? 0,
    events: num(r.events) ?? 0,
    pending: num(r.pending) ?? 0,
    hasReport: r.hasReport === true,
    proposals: (Array.isArray(r.proposals) ? r.proposals : []).map(obj).map((p) => ({ id: String(p.id ?? ''), name: String(p.name ?? p.id ?? ''), kind: String(p.kind ?? ''), decision: String(p.decision ?? 'pending') })),
    ...(Array.isArray(r.summary) ? { summary: r.summary.filter((x): x is string => typeof x === 'string') } : {}),
    ...(typeof r.error === 'string' ? { error: r.error } : {}),
    ...(r.memoryChanges && typeof r.memoryChanges === 'object'
      ? { memoryChanges: { added: num(obj(r.memoryChanges).added) ?? 0, updated: num(obj(r.memoryChanges).updated) ?? 0, merged: num(obj(r.memoryChanges).merged) ?? 0, pruned: num(obj(r.memoryChanges).pruned) ?? 0 } }
      : {}),
  }));
  const due = obj(o.due);
  const sched = obj(o.schedule);
  const sc = obj(o.scheduler);
  const next = whenValue(sc.nextRun);
  return {
    enabled: o.enabled === true,
    ...(typeof sched.mode === 'string' ? { scheduleMode: sched.mode } : {}),
    ...(typeof sched.time === 'string' ? { scheduleTime: sched.time } : {}),
    ...(num(sched.intervalHours) !== undefined ? { intervalHours: num(sched.intervalHours) } : {}),
    ...(o.scheduler ? { scheduler: { registered: sc.registered === true, hostRunning: sc.hostRunning === true, ...(next !== undefined ? { nextRun: next } : {}), ...(typeof sc.error === 'string' ? { error: sc.error } : {}) } } : {}),
    ...(num(o.minSessions) !== undefined ? { minSessions: num(o.minSessions) } : {}),
    ...(num(o.minGapHours) !== undefined ? { minGapHours: num(o.minGapHours) } : {}),
    ...(num(o.lastRunAt) !== undefined ? { lastRunAt: num(o.lastRunAt) } : {}),
    running: o.running === true,
    runs,
    ...(o.due ? { due: { due: due.due === true, ...(typeof due.reason === 'string' ? { reason: due.reason } : {}) } } : {}),
  };
}

/** A timestamp as ms: a number (seconds or ms) or an RFC 3339 string. */
function whenValue(v: unknown): number | undefined {
  if (typeof v === 'number' && Number.isFinite(v) && v > 0) return v < 1e11 ? v * 1000 : v;
  if (typeof v === 'string') {
    const t = Date.parse(v);
    return Number.isNaN(t) ? undefined : t;
  }
  return undefined;
}

/** "3m 12s" / "1h 04m"; running runs have no end. */
export function duration(r: Pick<DreamRun, 'startedAt' | 'endedAt'>): string | undefined {
  if (!r.startedAt || !r.endedAt || r.endedAt < r.startedAt) return undefined;
  const s = Math.round((r.endedAt - r.startedAt) / 1000);
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, '0')}s`;
  return `${Math.floor(s / 3600)}h ${String(Math.floor((s % 3600) / 60)).padStart(2, '0')}m`;
}

/** The memory edits as chips: only the non-zero ones. */
export function changeParts(c?: DreamRun['memoryChanges']): { key: string; n: number; label: string }[] {
  if (!c) return [];
  return ([['added', c.added], ['merged', c.merged], ['updated', c.updated], ['pruned', c.pruned]] as const).filter(([, n]) => n > 0).map(([key, n]) => ({ key, n, label: `${n} ${key}` }));
}

const sameDay = (a: Date, b: Date) => a.toDateString() === b.toDateString();
const clock = (d: Date) => d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });

/** "today 03:00" / "tomorrow 03:00" / "Mon, Nov 9, 03:00". */
export function whenNext(ms: number, now = Date.now()): string {
  const d = new Date(ms);
  const n = new Date(now);
  if (sameDay(d, n)) return `today ${clock(d)}`;
  if (sameDay(d, new Date(now + 86_400_000))) return `tomorrow ${clock(d)}`;
  return `${d.toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric' })}, ${clock(d)}`;
}

/** The one status line: what happens next (or why nothing will). Pure. */
export function statusLine(s: DreamStatus, now = Date.now()): { text: string; tone: 'ok' | 'warn' | 'dim' } {
  if (s.running) return { text: 'Dreaming now…', tone: 'ok' };
  if (!s.enabled) return { text: 'Scheduled dreaming is off. Turn it on below, or press Dream now.', tone: 'dim' };
  if (s.scheduleMode === 'manual') return { text: 'Manual only: dreams run when you press Dream now.', tone: 'dim' };
  const sc = s.scheduler;
  if (sc?.error) return { text: `Couldn’t set up the schedule: ${sc.error}`, tone: 'warn' };
  if (sc && !sc.hostRunning) return { text: 'Scheduled, but the Oino host isn’t running — dreams fire only while it runs.', tone: 'warn' };
  if (sc?.nextRun) return { text: `Next dream: ${whenNext(sc.nextRun, now)}`, tone: 'ok' };
  if (s.due?.due) return { text: 'A dream is due; it runs at the next scheduled check.', tone: 'ok' };
  return { text: s.scheduleMode === 'interval' && s.intervalHours ? `Scheduled every ${s.intervalHours} h.` : `Scheduled daily${s.scheduleTime ? ` at ${s.scheduleTime}` : ''}.`, tone: 'ok' };
}

const when = (ms?: number) => (ms ? new Date(ms).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : 'never');

export function DreamPanel(props: { hostId: string }) {
  const call = async <T,>(name: string, payload?: Record<string, unknown>): Promise<T> => {
    const handle = await settingsCommandChat(props.hostId);
    if (!handle) throw new Error('Couldn’t reach the agent on this PC — try again in a moment.');
    return handle.cmd<T>({ type: '_oino/service_call', name, ...(payload ? { payload } : {}) });
  };
  const [status, { refetch }] = createResource(async () => parseDreamStatus(await call<unknown>('dream.status')));
  const [ns] = createResource(
    () => props.hostId,
    async (h) => {
      const r = await settingsSchema(h);
      return (r.ok ? r.value.find((n) => n.namespace === 'dream') : undefined) ?? null;
    },
  );
  const [open, setOpen] = createSignal<Record<string, boolean>>({});
  const isOpen = (r: DreamRun) => open()[r.id] ?? r.proposals.some((p) => p.decision === 'pending');
  const [report, setReport] = createSignal<{ id: string; text?: string } | undefined>();
  const [busy, setBusy] = createSignal<string | undefined>();
  const act = async (key: string, name: string, payload?: Record<string, unknown>) => {
    setBusy(key);
    try {
      const r = obj(await call<unknown>(name, payload));
      if (r.message) toasts().push(String(r.message), r.ok === false ? 'error' : 'success');
      await refetch();
    } catch (e) {
      toasts().push(String((e as { message?: unknown })?.message ?? e), 'error');
    } finally {
      setBusy(undefined);
    }
  };
  const openReport = async (id: string) => {
    setReport({ id });
    try {
      const d = obj(await call<unknown>('dream.detail', { id }));
      setReport({ id, text: typeof d.report === 'string' && d.report.trim() ? d.report : 'No report was written for this run.' });
    } catch (e) {
      setReport({ id, text: String((e as { message?: unknown })?.message ?? e) });
    }
  };
  const line = createMemo(() => (status.latest ? statusLine(status.latest) : undefined));
  return (
    <div class="dream" data-testid="dream-panel">
      <div class="dream-intro">
        <MoonStar size={16} class="dream-ic" />
        <p>
          A dream tidies your memories offline: it merges overlapping notes, drops transient details, adds lessons your sessions missed and removes memories nothing used lately. Source sessions and your explicit preferences are kept. Skills and checks it proposes wait for your approval.
        </p>
      </div>
      <Show when={status.latest} fallback={<p class="dream-note">{status.error ? String((status.error as { message?: unknown }).message ?? 'Dream is not available on this PC.') : 'Loading…'}</p>}>
        {(s) => (
          <>
            <div class="dream-status" data-testid="dream-status" data-tone={line()?.tone}>
              <div class="dream-status-main">
                <span class="dream-pill" data-on={s().enabled ? '' : undefined}>{s().enabled ? (s().scheduleMode === 'manual' ? 'Manual only' : 'Scheduled') : 'Off'}</span>
                <span class="dream-status-line" data-testid="dream-status-line">{line()?.text}</span>
                <span class="dream-dim">Last dream: {when(s().lastRunAt)}</span>
              </div>
              <div class="dream-status-actions">
                <Show
                  when={s().running}
                  fallback={
                    <button type="button" class="btn-secondary settings-btn" disabled={!!busy()} onClick={() => void act('run', 'dream.run')} data-testid="dream-run">
                      <Play size={13} /> Dream now
                    </button>
                  }
                >
                  <button type="button" class="btn-secondary settings-btn" disabled={!!busy()} onClick={() => void act('stop', 'dream.stop')} data-testid="dream-stop">
                    <Square size={12} /> Stop
                  </button>
                </Show>
              </div>
            </div>

            <div class="settings-in-card-label dream-sub">Dream settings</div>
            <Show when={ns()} fallback={<p class="dream-note">{ns.loading ? 'Loading…' : 'This PC reports no Dream settings.'}</p>}>
              {(n) => <NamespaceForm namespace={n()} hostId={props.hostId} bare flat onChanged={() => void refetch()} testId="dream-settings-form" />}
            </Show>

            <div class="settings-in-card-label dream-sub">Recent dreams</div>
            <Show when={s().runs.length > 0} fallback={<p class="dream-note">No dreams yet. “Dream now” reviews the sessions since the last one.</p>}>
              <ul class="dream-runs" data-testid="dream-runs">
                <For each={s().runs}>
                  {(r) => {
                    const pending = () => r.proposals.filter((p) => p.decision === 'pending').length;
                    const parts = () => changeParts(r.memoryChanges);
                    return (
                      <li class="dream-run" data-status={r.status} data-testid={`dream-run-${r.id}`}>
                        <button type="button" class="dream-run-head" aria-expanded={isOpen(r)} onClick={() => setOpen({ ...open(), [r.id]: !isOpen(r) })} data-testid={`dream-run-toggle-${r.id}`}>
                          {isOpen(r) ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                          <span class="dream-run-when">{when(r.startedAt)}</span>
                          <span class="dream-run-status" data-status={r.status}>{r.status === 'running' ? <LoaderCircle size={11} class="refresh-spinning" /> : null}{r.status}</span>
                          <Show when={duration(r)}>{(d) => <span class="dream-dim">{d()}</span>}</Show>
                          <span class="dream-dim">{r.manual ? 'manual' : 'scheduled'}</span>
                          <Show when={parts().length}>
                            <span class="dream-chips" data-testid={`dream-changes-${r.id}`}>
                              <For each={parts()}>{(c) => <span class="dream-chip" data-kind={c.key}>{c.label}</span>}</For>
                            </span>
                          </Show>
                          <Show when={pending() > 0}>
                            <span class="dream-chip" data-kind="proposal">{pending()} proposal{pending() === 1 ? '' : 's'}</span>
                          </Show>
                        </button>
                        <Show when={isOpen(r)}>
                          <div class="dream-run-body">
                            <span class="dream-dim">{r.sessions} session{r.sessions === 1 ? '' : 's'} digested · {r.events} event{r.events === 1 ? '' : 's'}</span>
                            <Show when={r.error}>
                              <p class="dream-err"><CircleAlert size={13} /> {r.error}</p>
                            </Show>
                            <Show when={r.summary?.length}>
                              <ul class="dream-summary"><For each={r.summary}>{(l) => <li>{l}</li>}</For></ul>
                            </Show>
                            <Show when={r.proposals.length}>
                              <div class="dream-props">
                                <For each={r.proposals}>
                                  {(p) => (
                                    <div class="dream-prop" data-decision={p.decision}>
                                      <span class="dream-prop-kind">{p.kind || 'proposal'}</span>
                                      <span class="dream-prop-name">{p.name}</span>
                                      <span class="flex-1" />
                                      <Show when={p.decision === 'pending'} fallback={<span class="dream-dim">{p.decision}</span>}>
                                        <button type="button" class="btn-secondary settings-btn" disabled={!!busy()} onClick={() => void act(`ok-${p.id}`, 'dream.approve', { runId: r.id, proposalId: p.id })} data-testid={`dream-approve-${p.id}`}>
                                          <Check size={13} /> Approve
                                        </button>
                                        <button type="button" class="btn-secondary settings-btn" disabled={!!busy()} onClick={() => void act(`no-${p.id}`, 'dream.reject', { runId: r.id, proposalId: p.id })} data-testid={`dream-reject-${p.id}`}>
                                          <X size={13} /> Reject
                                        </button>
                                      </Show>
                                    </div>
                                  )}
                                </For>
                              </div>
                            </Show>
                            <div class="dream-run-actions">
                              <Show when={r.hasReport}>
                                <button type="button" class="btn-secondary settings-btn" onClick={() => void openReport(r.id)} data-testid={`dream-report-${r.id}`}>Report</button>
                              </Show>
                              <Show when={r.status !== 'running'}>
                                <button type="button" class="settings-link" disabled={!!busy()} onClick={() => void act(`dismiss-${r.id}`, 'dream.dismiss', { runId: r.id })} title="Remove this run from the list">Dismiss</button>
                              </Show>
                            </div>
                          </div>
                        </Show>
                      </li>
                    );
                  }}
                </For>
              </ul>
            </Show>
          </>
        )}
      </Show>
      <Show when={report()}>
        {(r) => (
          <BoardSheet open onClose={() => setReport(undefined)} label="Dream report" class="settings-cmd-sheet" testId="dream-report-sheet">
            <Show when={r().text} fallback={<p class="dream-note">Loading…</p>}>
              <div class="settings-cmd-body report-md"><Markdown text={r().text!} /></div>
            </Show>
          </BoardSheet>
        )}
      </Show>
    </div>
  );
}
