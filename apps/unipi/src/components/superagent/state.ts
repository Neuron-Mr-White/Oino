import { topicsFor } from '../../lib/topics/client';
import { fallbackPoll } from '../../lib/topics/poll-allowlist';
import { createEffect, createMemo, createRoot, createSignal, on, onCleanup, type Accessor } from 'solid-js';
import { toAppError } from '@unipi/protocol';
import { within } from '../../lib/host/within';
import { api as boardApi } from '../../lib/board/api';
import { ConfigSchema, isUnsupportedOp, RelayTextSchema, superagentApi, type EnableAction, type Fleet, type Lead, type LeadMode, type PausedGraphPolicy, type Preset, type RelayDelivery, type SuperagentSession, type SuperagentStatus } from './api';
import { superagentSessionId } from './chatActivity';
import { runningSessions, type RunningSession } from './runningSessions';
import { fallbackFleet, makeFleet, mergeStatusLeads } from './fleet';
import { openFailed, sessionMissing, sessions, type SessionHandle } from '../../lib/session/store';

/** A mutation's failure, as far as the pane cares. */
export type RunError = { message: string; code?: unknown; detail?: unknown };
export type EnableInput = { action: EnableAction; projectId?: string; path?: string };
export type EnableResult = { ok: true; projectId: string | null } | { ok: false; message: string };
export type TicketMeta = { title: string; autonomy?: string; status?: string; /** When the ticket last changed (ISO); the Workbench ranks projects by it. */ updated?: string; priority?: string; labels?: string[] };
export const ticketKey = (project: string, task: string) => `${project}/${task}`;
const POLL_MS = 5000;

function waitOutboxSent(handle: SessionHandle, ref: string, timeoutMs = 15_000): Promise<{ ok: true; value: undefined } | { ok: false; error: RunError }> {
  return new Promise((resolve) => {
    createRoot((dispose) => {
      let done = false;
      const finish = (res: { ok: true; value: undefined } | { ok: false; error: RunError }) => {
        if (done) return;
        done = true;
        clearTimeout(timer);
        queueMicrotask(dispose);
        resolve(res);
      };
      const timer = setTimeout(() => {
        handle.dismiss(ref);
        finish({ ok: false, error: { message: 'Timed out resuming the paused graph. Retry.' } });
      }, timeoutMs);
      createEffect(() => {
        const current = handle.state.outbox.find((o) => o.ref === ref);
        if (!current || current.status === 'acked') {
          finish({ ok: true, value: undefined });
          return;
        }
        if (current.status === 'lost') {
          const message = current.error || 'Could not resume the paused graph.';
          handle.dismiss(ref);
          finish({ ok: false, error: { message } });
          return;
        }
        if (openFailed(handle.state) || sessionMissing(handle.state)) {
          const message = handle.state.connError?.message || 'Could not open the previous lead session.';
          handle.dismiss(ref);
          finish({ ok: false, error: { message } });
        }
      });
    });
  });
}

/**
 * One host's Superagent state: polled status, ticket titles, presets and every
 * mutation. Lives above the pane so the phone badge works with the sheet closed.
 */
export function createSuperagentState(hostOrNone: Accessor<string | undefined>) {
  const hostId = () => hostOrNone() ?? '';
  const [status, setStatus] = createSignal<SuperagentStatus>();
  const [history, setHistory] = createSignal<SuperagentSession[]>([]);
  const running = createMemo(() => runningSessions(status(), history(), superagentSessionId(hostId())));
  /** The host's fleet; `fallback` means it has none (older host) and the canvas derives it from status + history. */
  const [hostFleet, setHostFleet] = createSignal<Fleet>();
  const [fleetFallback, setFleetFallback] = createSignal(false);
  /** A host that sends a `rev` knows its own leads; only an older fleet is overlaid with the status leads. */
  const fleet = createMemo<Fleet>(() => {
    const current = hostFleet();
    return current ? (current.rev ? current : mergeStatusLeads(current, status())) : fleetFallback() ? fallbackFleet(status(), history()) : makeFleet();
  });
  /** When the last poll fully succeeded, how many polls in a row failed, and the last error: the canvas's "stale" and first-load error states. */
  const [updatedAt, setUpdatedAt] = createSignal<number | null>(null);
  const [failedPolls, setFailedPolls] = createSignal(0);
  const [fleetError, setFleetError] = createSignal<{ code: string; message: string } | undefined>();
  const [error, setError] = createSignal('');
  const [notice, setNotice] = createSignal('');
  const [busy, setBusy] = createSignal<readonly string[]>([]);
  const [titles, setTitles] = createSignal<Record<string, TicketMeta>>({});
  const [presets, setPresets] = createSignal<{ list: Preset[]; defaultId: string | null; supported: boolean }>({ list: [], defaultId: null, supported: true });
  const [now, setNow] = createSignal(Date.now());
  let generation = 0;
  let loading = false;
  let pendingRefresh: Promise<void> | undefined;
  let fleetSupported = true;
  let noticeTimer: ReturnType<typeof setTimeout> | undefined;
  const requested = new Set<string>();
  let currentTickets = new Set<string>();

  const isBusy = (key: string) => busy().includes(key);
  const flash = (text: string) => {
    setNotice(text);
    clearTimeout(noticeTimer);
    noticeTimer = setTimeout(() => setNotice(''), 6000);
  };
  /** Runs one mutation: busy key, host-generation guard, one error surface. */
  async function run<T>(key: string, job: () => Promise<{ ok: true; value: T } | { ok: false; error: RunError }>, done: (value: T) => void | Promise<void>, handled?: (error: RunError) => boolean): Promise<boolean> {
    if (isBusy(key)) return false;
    const gen = generation;
    setBusy((old) => [...old, key]); setError('');
    try {
      const result = await job();
      if (gen !== generation) return false;
      if (!result.ok) { if (!handled?.(result.error)) setError(result.error.message); return false; }
      await done(result.value);
      return true;
    } catch (e) { if (gen === generation) setError(toAppError(e).message); return false; }
    finally { if (gen === generation) setBusy((old) => old.filter((k) => k !== key)); }
  }

  /** Titles asked for before the first poll has settled its ticket set are kept (the poll prunes them afterwards). */
  async function ensureTitles(rows: ReadonlyArray<{ projectId: string; taskId: string }>) {
    const host = hostId();
    const gen = generation;
    const missing = rows.filter((row) => !requested.has(ticketKey(row.projectId, row.taskId)));
    missing.forEach((row) => requested.add(ticketKey(row.projectId, row.taskId)));
    const found = await Promise.all(missing.map(async (row) => {
      const key = ticketKey(row.projectId, row.taskId);
      try {
        const task = await within(boardApi.task(host, row.projectId, row.taskId), () => undefined);
        if (!task) { requested.delete(key); return [key, { title: '' }] as const; }
        return [key, { title: task.displayTitle || task.title, autonomy: task.autonomy ?? 'medium', status: task.status, updated: task.updated, priority: task.priority, labels: task.labels }] as const;
      } catch { requested.delete(key); return [key, { title: '' }] as const; }
    }));
    if (gen === generation && found.length) setTitles((old) => ({ ...old, ...Object.fromEntries(found.filter(([key]) => currentTickets.size === 0 || currentTickets.has(key))) }));
  }

  async function refresh() {
    if (loading) { await pendingRefresh; if (loading) return; }
    const job = poll();
    pendingRefresh = job;
    try { await job; } finally { if (pendingRefresh === job) pendingRefresh = undefined; }
  }

  async function poll() {
    loading = true;
    const gen = generation;
    try {
      const result = await within(superagentApi.status(hostId()), () => ({ ok: false as const, error: { code: 'E_TIMEOUT' as const, message: 'The host is not answering. Retry.', retryable: true } }));
      if (gen !== generation) return;
      if (!result.ok) { setError(result.error.message); setFailedPolls((n) => n + 1); setFleetError({ code: String(result.error.code), message: result.error.message }); return; }
      setStatus(result.value);
      let fleetFailure: { code: string; message: string } | undefined;
      const historyRequest = within(superagentApi.sessions(hostId()), () => undefined).then((sessions) => {
        if (gen === generation && sessions?.ok) setHistory(sessions.value);
      }).catch(() => { /* History is optional on older hosts. */ });
      const fleetRequest = fleetSupported ? within(superagentApi.fleet(hostId()), () => undefined).then((fleet) => {
        if (gen !== generation) return;
        if (!fleet) { fleetFailure = { code: 'E_TIMEOUT', message: 'The host is not answering. Retry.' }; return; }
        if (fleet.ok) { setHostFleet(fleet.value); setFleetFallback(false); return; }
        // Only an older host (no `superagent.fleet`) falls back to status + history; any other failure keeps the last fleet and counts as a failed poll.
        if (isUnsupportedOp(fleet.error.code)) { fleetSupported = false; setFleetFallback(true); return; }
        fleetFailure = { code: String(fleet.error.code), message: fleet.error.message };
      }).catch((e) => { if (gen === generation) fleetFailure = { code: 'E_FAILED', message: toAppError(e).message }; }) : Promise.resolve();
      await Promise.all([historyRequest, fleetRequest]);
      if (gen !== generation) return;
      if (fleetFailure) { setFailedPolls((n) => n + 1); setFleetError(fleetFailure); } else { setFailedPolls(0); setFleetError(undefined); setUpdatedAt(Date.now()); }
      const rows = [...result.value.needsYou, ...result.value.review,
        ...result.value.leads.flatMap((lead) => lead.ticketIds.map((taskId) => ({ projectId: lead.projectId, taskId })))];
      currentTickets = new Set(rows.map((row) => ticketKey(row.projectId, row.taskId)));
      for (const key of requested) if (!currentTickets.has(key)) requested.delete(key);
      setTitles((old) => Object.fromEntries(Object.entries(old).filter(([key]) => currentTickets.has(key))));
      setError('');
      void ensureTitles(result.value.needsYou);
    } catch (e) { if (gen === generation) setError(toAppError(e).message); }
    finally { if (gen === generation) loading = false; }
  }

  async function loadPresets() {
    const gen = generation;
    try {
      const result = await superagentApi.presets.list(hostId());
      if (gen !== generation) return;
      if (result.ok) setPresets({ list: result.value.presets, defaultId: result.value.defaultPresetId ?? null, supported: true });
      else if (String(result.error.code) === 'E_UNKNOWN_OP') setPresets({ list: [], defaultId: null, supported: false });
    } catch { /* Presets are optional: Start works without one. */ }
  }

  // Keyed on the id: the selected host's record is re-issued on every connection event, and each of those used to wipe the fleet.
  const hostKey = createMemo(hostId);
  createEffect(on(hostKey, () => {
    generation++;
    loading = false;
    fleetSupported = true;
    requested.clear();
    currentTickets.clear();
    setHostFleet(undefined); setFleetFallback(false); setUpdatedAt(null); setFailedPolls(0); setFleetError(undefined);
    setStatus(undefined); setHistory([]); setError(''); setNotice(''); setTitles({}); setBusy([]);
    setPresets({ list: [], defaultId: null, supported: true });
    if (!hostId()) return;
    void refresh();
    void loadPresets();
    const tick = () => { setNow(Date.now()); if (!loading) void refresh(); };
    const stop = topicsFor(hostId()!).watch('superagent', tick, () => fallbackPoll(tick, POLL_MS));
    onCleanup(() => { generation++; stop(); clearTimeout(noticeTimer); });
  }));

  const patchStatus = (patch: (s: SuperagentStatus) => SuperagentStatus) => setStatus((old) => old && patch(old));

  return {
    status, history, running, fleet, fleetLoaded: () => !!hostFleet() || fleetFallback(), updatedAt, failedPolls, fleetError, error, notice, titles, presets, now, isBusy, refresh, ensureTitles, loadPresets,
    clearError: () => setError(''),
    flash,
    projectName: (id: string) => status()?.projects?.find((p) => p.projectId === id)?.name || id,
    presetLabel: (id: string | null | undefined) => (id ? presets().list.find((p) => p.id === id)?.label ?? id : null),
    meta: (project: string, task: string) => titles()[ticketKey(project, task)],
    configure(enabled: boolean) {
      const parsed = ConfigSchema.parse({ enabled });
      return run('configure', () => superagentApi.configure(hostId(), parsed), (v) => {
        patchStatus((s) => ({ ...s, enabled: v.enabled, ...(v.pausedGraphPolicy ? { pausedGraphPolicy: v.pausedGraphPolicy } : {}) }));
      });
    },
    setPausedGraphPolicy(pausedGraphPolicy: PausedGraphPolicy) {
      // TODO(OIN-56 phase 2): host-side "reuse" currently behaves like "report" until automatic wake/reuse is wired.
      const parsed = ConfigSchema.parse({ enabled: status()?.enabled ?? false, pausedGraphPolicy });
      return run('configure', () => superagentApi.configure(hostId(), parsed), (v) => {
        patchStatus((s) => ({ ...s, enabled: v.enabled, pausedGraphPolicy: v.pausedGraphPolicy ?? pausedGraphPolicy }));
      });
    },
    resumePausedGraph(sessionId: string) {
      return run(`resume:${sessionId}`, async () => {
        const handle = sessions().ensure(hostId(), sessionId);
        const release = handle.acquire();
        if (openFailed(handle.state)) handle.retry();
        try {
          const item = handle.send('/graph resume', { mode: 'prompt' });
          return await waitOutboxSent(handle, item.ref);
        } finally {
          release();
        }
      }, async () => {
        flash('Resuming paused graph.');
        await refresh();
      });
    },
    async setAutoStart(projectId: string, autoStart: boolean) {
      const before = status()?.projects?.find((p) => p.projectId === projectId)?.autoStart;
      const apply = (value: boolean) => patchStatus((s) => ({ ...s, projects: s.projects?.map((p) => p.projectId === projectId ? { ...p, autoStart: value } : p) ?? null }));
      apply(autoStart);
      const ok = await run(`auto:${projectId}`, () => superagentApi.configureProject(hostId(), projectId, autoStart), (v) => { apply(v.autoStart); });
      if (!ok && before !== undefined) apply(before);
      // The hint under the name comes from the host's blockers: re-read them.
      if (ok && status()?.projects?.find((p) => p.projectId === projectId)?.blockers?.length) void refresh();
      if (ok && autoStart) {
        const current = status();
        const project = current?.projects?.find((p) => p.projectId === projectId);
        if (current?.enabled === false) flash('Auto-start is off globally — turn it on in Superagent settings to start leads');
        else if (current?.enabled && project && project.ready > 0) flash(`Starting a lead for ${project.ready} ready tickets`);
      }
      return ok;
    },
    setMode: (projectId: string, mode: LeadMode) => run(`mode:${projectId}`, () => superagentApi.setMode(hostId(), projectId, mode), async () => { flash(`Mode will switch at the next step.`); await refresh(); }),
    stopLead: (lead: Lead) => run(`stop:${lead.projectId}`, () => superagentApi.stop(hostId(), lead.projectId, lead.ticketIds[0] ?? ''), async (v) => { setHistory((rows) => rows.map((r) => r.sessionId === lead.sessionId ? { ...r, state: { kind: 'idle' }, pendingWork: null } : r)); patchStatus((s) => ({ ...s, leads: s.leads.filter((l) => l.sessionId !== lead.sessionId) })); flash(v.stopped ? 'Lead stopped.' : 'That lead is no longer running.'); await refresh(); }),
    async stopSession(row: RunningSession) {
      if (row.lead) {
        return run(`stop:${row.lead.projectId}`, () => superagentApi.stop(hostId(), row.lead!.projectId, row.lead!.ticketIds[0] ?? ''), async () => {
          setHistory((rows) => rows.map((r) => r.sessionId === row.sessionId ? { ...r, state: { kind: 'idle' }, pendingWork: null } : r));
          patchStatus((s) => ({ ...s, leads: s.leads.filter((l) => l.sessionId !== row.sessionId) }));
          await refresh();
        });
      }
      return run(`stop-session:${row.sessionId}`, async () => {
        const handle = sessions().ensure(hostId(), row.sessionId);
        const release = handle.acquire();
        try { await handle.abort(); return { ok: true as const, value: undefined }; }
        finally { release(); }
      }, () => { setHistory((rows) => rows.map((r) => r.sessionId === row.sessionId ? { ...r, state: { kind: 'idle' }, pendingWork: null } : r)); });
    },
    /** `handled` sees a refusal first: return true to keep it out of the pane's error line (the caller shows it). */
    start: (projectId: string, taskIds: string[], presetId: string | null, handled?: (error: RunError) => boolean) => run(`start:${projectId}`, () => superagentApi.start(hostId(), projectId, taskIds, presetId), async () => { flash(taskIds.length === 1 ? 'Started 1 ticket.' : `Started ${taskIds.length} tickets.`); await refresh(); }, handled),
    /** Clears one blocker (register, unarchive, Auto on) and refreshes. The caller shows a failure inline; nothing goes to the pane's error line. */
    async enable(input: EnableInput): Promise<EnableResult> {
      const key = `enable:${input.projectId ?? input.path ?? input.action}`;
      if (isBusy(key)) return { ok: false, message: 'Already working on it.' };
      const gen = generation;
      setBusy((old) => [...old, key]);
      try {
        const result = await superagentApi.enable(hostId(), input);
        if (gen !== generation) return { ok: false, message: 'The host changed.' };
        if (!result.ok) return { ok: false, message: result.error.message };
        // Drop the cleared blocker now; the refresh below confirms it (a poll already in flight may be stale).
        if (input.action !== 'register') patchStatus((st) => ({
          ...st, enabled: input.action === 'enable_auto_start' ? true : st.enabled,
          projects: st.projects?.map((p) => (input.action === 'enable_auto_start' || p.projectId === input.projectId ? { ...p, autoStart: input.action === 'enable_project_auto' ? true : p.autoStart, blockers: (p.blockers ?? []).filter((b) => b.action.kind !== input.action) } : p)) ?? null,
        }));
        if (input.action === 'register' || input.action === 'unarchive') sessions().refreshBoards(hostId());
        await refresh();
        return { ok: true, projectId: result.value.projectId ?? input.projectId ?? null };
      } catch (e) { return { ok: false, message: toAppError(e).message }; }
      finally { if (gen === generation) setBusy((old) => old.filter((k) => k !== key)); }
    },
    /** Answer a blocked ticket, restarting its lead when necessary. */
    answer(projectId: string, taskId: string, text: string, delivery: RelayDelivery = 'steer') {
      const parsed = RelayTextSchema.safeParse(text);
      if (!parsed.success) { setError(parsed.error.issues[0]?.message ?? 'Enter a message.'); return Promise.resolve(false); }
      return run(`answer:${projectId}/${taskId}`, () => superagentApi.relay(hostId(), { projectId, taskIds: [taskId], text: `Re ${taskId}: ${parsed.data}`, delivery }), (v) => {
        if (!v.delivered && !v.saved) throw new Error(`Saved, but the lead did not receive it. ${typeof v.error === 'string' ? v.error : (v.error as { message?: string } | undefined)?.message ?? 'Open the lead before retrying.'}`);
        flash(v.saved ? ((v.error as { message?: string } | undefined)?.message ?? 'Answer saved, but could not start a lead.') : v.started ? `Answer saved — started a lead for ${taskId}` : `Sent to the ${projectId} lead.`);
        patchStatus((st) => ({ ...st, needsYou: st.needsYou.filter((item) => !(item.projectId === projectId && item.taskId === taskId && 'question' in item)) }));
      });
    },
    savePreset: (preset: Preset) => run('preset', () => superagentApi.presets.save(hostId(), preset), () => loadPresets()),
    deletePreset: (id: string) => run('preset', () => superagentApi.presets.remove(hostId(), id), () => loadPresets()),
    setDefaultPreset: (id: string) => run('preset', () => superagentApi.presets.setDefault(hostId(), id), () => loadPresets()),
  };
}
export type SuperagentState = ReturnType<typeof createSuperagentState>;
