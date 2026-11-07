import { createEffect, createSignal, onCleanup, type Accessor } from 'solid-js';
import { toAppError } from '@unipi/protocol';
import { within } from '../../lib/host/within';
import { api as boardApi } from '../../lib/board/api';
import { ConfigSchema, RelayTextSchema, superagentApi, type Lead, type LeadMode, type Preset, type RelayDelivery, type SuperagentStatus } from './api';

export type TicketMeta = { title: string; autonomy?: string };
export const ticketKey = (project: string, task: string) => `${project}/${task}`;
const POLL_MS = 5000;

/**
 * One host's Superagent state: polled status, ticket titles, presets and every
 * mutation. Lives above the pane so the phone badge works with the sheet closed.
 */
export function createSuperagentState(hostOrNone: Accessor<string | undefined>) {
  const hostId = () => hostOrNone() ?? '';
  const [status, setStatus] = createSignal<SuperagentStatus>();
  const [error, setError] = createSignal('');
  const [notice, setNotice] = createSignal('');
  const [busy, setBusy] = createSignal<readonly string[]>([]);
  const [titles, setTitles] = createSignal<Record<string, TicketMeta>>({});
  const [presets, setPresets] = createSignal<{ list: Preset[]; defaultId: string | null; supported: boolean }>({ list: [], defaultId: null, supported: true });
  const [now, setNow] = createSignal(Date.now());
  let generation = 0;
  let loading = false;
  let noticeTimer: ReturnType<typeof setTimeout> | undefined;
  const requested = new Set<string>();

  const isBusy = (key: string) => busy().includes(key);
  const flash = (text: string) => {
    setNotice(text);
    clearTimeout(noticeTimer);
    noticeTimer = setTimeout(() => setNotice(''), 6000);
  };
  /** Runs one mutation: busy key, host-generation guard, one error surface. */
  async function run<T>(key: string, job: () => Promise<{ ok: true; value: T } | { ok: false; error: { message: string } }>, done: (value: T) => void | Promise<void>): Promise<boolean> {
    if (isBusy(key)) return false;
    const gen = generation;
    setBusy((old) => [...old, key]); setError('');
    try {
      const result = await job();
      if (gen !== generation) return false;
      if (!result.ok) { setError(result.error.message); return false; }
      await done(result.value);
      return true;
    } catch (e) { if (gen === generation) setError(toAppError(e).message); return false; }
    finally { if (gen === generation) setBusy((old) => old.filter((k) => k !== key)); }
  }

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
        return [key, { title: task.displayTitle || task.title, autonomy: task.autonomy ?? 'medium' }] as const;
      } catch { requested.delete(key); return [key, { title: '' }] as const; }
    }));
    if (gen === generation && found.length) setTitles((old) => ({ ...old, ...Object.fromEntries(found) }));
  }

  async function refresh() {
    if (loading) return;
    loading = true;
    const gen = generation;
    try {
      const result = await within(superagentApi.status(hostId()), () => ({ ok: false as const, error: { code: 'E_TIMEOUT' as const, message: 'The host is not answering. Retry.', retryable: true } }));
      if (gen !== generation) return;
      if (!result.ok) { setError(result.error.message); return; }
      setStatus(result.value);
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

  createEffect(() => {
    hostId();
    generation++;
    loading = false;
    requested.clear();
    setStatus(undefined); setError(''); setNotice(''); setTitles({}); setBusy([]);
    setPresets({ list: [], defaultId: null, supported: true });
    if (!hostId()) return;
    void refresh();
    void loadPresets();
    const timer = setInterval(() => { setNow(Date.now()); void refresh(); }, POLL_MS);
    onCleanup(() => { generation++; clearInterval(timer); clearTimeout(noticeTimer); });
  });

  const patchStatus = (patch: (s: SuperagentStatus) => SuperagentStatus) => setStatus((old) => old && patch(old));

  return {
    status, error, notice, titles, presets, now, isBusy, refresh, ensureTitles, loadPresets,
    clearError: () => setError(''),
    flash,
    projectName: (id: string) => status()?.projects?.find((p) => p.projectId === id)?.name || id,
    presetLabel: (id: string | null | undefined) => (id ? presets().list.find((p) => p.id === id)?.label ?? id : null),
    meta: (project: string, task: string) => titles()[ticketKey(project, task)],
    configure(enabled: boolean, concurrencyCap: number) {
      const parsed = ConfigSchema.safeParse({ enabled, concurrencyCap });
      if (!parsed.success) { setError('Max leads must be 1 to 3.'); return Promise.resolve(false); }
      return run('configure', () => superagentApi.configure(hostId(), parsed.data), (v) => { patchStatus((s) => ({ ...s, enabled: v.enabled, concurrencyCap: v.concurrencyCap })); });
    },
    async setAutoStart(projectId: string, autoStart: boolean) {
      const before = status()?.projects?.find((p) => p.projectId === projectId)?.autoStart;
      const apply = (value: boolean) => patchStatus((s) => ({ ...s, projects: s.projects?.map((p) => p.projectId === projectId ? { ...p, autoStart: value } : p) ?? null }));
      apply(autoStart);
      const ok = await run(`auto:${projectId}`, () => superagentApi.configureProject(hostId(), projectId, autoStart), (v) => { apply(v.autoStart); });
      if (!ok && before !== undefined) apply(before);
      return ok;
    },
    setMode: (projectId: string, mode: LeadMode) => run(`mode:${projectId}`, () => superagentApi.setMode(hostId(), projectId, mode), async () => { flash(`Mode will switch at the next step.`); await refresh(); }),
    stopLead: (lead: Lead) => run(`stop:${lead.projectId}`, () => superagentApi.stop(hostId(), lead.projectId, lead.ticketIds[0] ?? ''), async (v) => { flash(v.stopped ? 'Lead stopped.' : 'That lead is no longer running.'); await refresh(); }),
    start: (projectId: string, taskIds: string[], presetId: string | null) => run(`start:${projectId}`, () => superagentApi.start(hostId(), projectId, taskIds, presetId), async () => { flash(taskIds.length === 1 ? 'Started 1 ticket.' : `Started ${taskIds.length} tickets.`); await refresh(); }),
    /** Answers go to the project's lead; the ticket id is in the text because a blocked ticket is not claimed. */
    answer(projectId: string, taskId: string, text: string, delivery: RelayDelivery = 'steer') {
      const parsed = RelayTextSchema.safeParse(text);
      if (!parsed.success) { setError(parsed.error.issues[0]?.message ?? 'Enter a message.'); return Promise.resolve(false); }
      return run(`answer:${projectId}/${taskId}`, () => superagentApi.relay(hostId(), { projectId, text: `Re ${taskId}: ${parsed.data}`, delivery }), (v) => {
        if (!v.delivered) throw new Error(`Saved, but the lead did not receive it. ${typeof v.error === 'string' ? v.error : (v.error as { message?: string } | undefined)?.message ?? 'Open the lead before retrying.'}`);
        flash(`Sent to the ${projectId} lead.`);
      });
    },
    savePreset: (preset: Preset) => run('preset', () => superagentApi.presets.save(hostId(), preset), () => loadPresets()),
    deletePreset: (id: string) => run('preset', () => superagentApi.presets.remove(hostId(), id), () => loadPresets()),
    setDefaultPreset: (id: string) => run('preset', () => superagentApi.presets.setDefault(hostId(), id), () => loadPresets()),
  };
}
export type SuperagentState = ReturnType<typeof createSuperagentState>;
