/**
 * Dream (owner 2026-11-05: "make sure dream settings here has some real purpose"): the
 * native surface over `unipi.dream`'s services instead of three rows that only echoed
 * command text. What a dream is, whether it is on and due, Run now / Stop, every run with
 * its digest count and summary, the report, and the proposals waiting for a decision
 * (Approve lands the skill / check, Reject drops it). Its settings form sits right here.
 *
 * Services (`dream.status`, `dream.run`, `dream.stop`, `dream.detail`, `dream.approve`,
 * `dream.reject`, `dream.dismiss`) run in Settings' own command session.
 */
import { createResource, createSignal, For, Show } from 'solid-js';
import { Check, CircleAlert, LoaderCircle, MoonStar, Play, Square, X } from 'lucide-solid';
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
};
export type DreamStatus = { enabled: boolean; minSessions?: number; minGapHours?: number; lastRunAt?: number; running: boolean; runs: DreamRun[]; due?: { due: boolean; reason?: string } };

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
  }));
  const due = obj(o.due);
  return {
    enabled: o.enabled === true,
    ...(num(o.minSessions) !== undefined ? { minSessions: num(o.minSessions) } : {}),
    ...(num(o.minGapHours) !== undefined ? { minGapHours: num(o.minGapHours) } : {}),
    ...(num(o.lastRunAt) !== undefined ? { lastRunAt: num(o.lastRunAt) } : {}),
    running: o.running === true,
    runs,
    ...(o.due ? { due: { due: due.due === true, ...(typeof due.reason === 'string' ? { reason: due.reason } : {}) } } : {}),
  };
}

const when = (ms?: number) => (ms ? new Date(ms).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : 'never');

export function DreamPanel(props: { hostId: string }) {
  const call = async <T,>(name: string, payload?: Record<string, unknown>): Promise<T> => {
    const handle = await settingsCommandChat(props.hostId);
    if (!handle) throw new Error('Couldn’t reach the agent on this PC — try again in a moment.');
    return handle.cmd<T>({ type: '_oino/service_call', name, ...(payload ? { payload } : {}) });
  };
  const [status, { refetch }] = createResource(async () => parseDreamStatus(await call<unknown>('dream.status')));
  const [showSettings, setShowSettings] = createSignal(false);
  const [ns] = createResource(
    () => (showSettings() ? props.hostId : undefined),
    async (h) => {
      const r = await settingsSchema(h);
      return (r.ok ? r.value.find((n) => n.namespace === 'dream') : undefined) ?? null;
    },
  );
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
  return (
    <div class="dream" data-testid="dream-panel">
      <div class="dream-intro">
        <MoonStar size={16} class="dream-ic" />
        <p>
          A dream reviews your recent sessions offline: the failed tool calls you recovered from and your corrections. It turns them into memory lessons, and proposes skills or checks that you approve below — so the next session takes the shortest path.
        </p>
      </div>
      <Show when={status.latest} fallback={<p class="dream-note">{status.error ? String((status.error as { message?: unknown }).message ?? 'Dream is not available on this PC.') : 'Loading…'}</p>}>
        {(s) => (
          <>
            <div class="dream-status" data-testid="dream-status">
              <span class="dream-pill" data-on={s().enabled ? '' : undefined}>{s().enabled ? 'Automatic' : 'Manual only'}</span>
              <span class="dream-dim">Last dream: {when(s().lastRunAt)}</span>
              <Show when={s().due}>
                {(d) => <span class="dream-dim">{d().due ? 'Due now' : d().reason ?? 'Not due yet'}</span>}
              </Show>
              <Show when={!s().enabled}>
                <span class="dream-dim">Runs on its own once turned on (after {s().minSessions ?? 'a few'} new sessions, {s().minGapHours ?? 24} h apart).</span>
              </Show>
              <span class="flex-1" />
              <Show
                when={s().running}
                fallback={
                  <button type="button" class="btn-secondary settings-btn" disabled={!!busy()} onClick={() => void act('run', 'dream.run')} data-testid="dream-run">
                    <Play size={13} /> Dream now
                  </button>
                }
              >
                <span class="dream-dim"><LoaderCircle size={13} class="refresh-spinning" /> Dreaming…</span>
                <button type="button" class="btn-secondary settings-btn" disabled={!!busy()} onClick={() => void act('stop', 'dream.stop')} data-testid="dream-stop">
                  <Square size={12} /> Stop
                </button>
              </Show>
              <button type="button" class="settings-link" onClick={() => setShowSettings(!showSettings())} aria-expanded={showSettings()} data-testid="dream-settings-toggle">
                {showSettings() ? 'Hide settings' : 'Settings'}
              </button>
            </div>
            <Show when={showSettings()}>
              <Show when={ns()} fallback={<p class="dream-note">{ns.loading ? 'Loading…' : 'This PC reports no Dream settings.'}</p>}>
                {(n) => <NamespaceForm namespace={n()} hostId={props.hostId} bare flat testId="dream-settings-form" />}
              </Show>
            </Show>
            <Show when={s().runs.length > 0} fallback={<p class="dream-note">No dreams yet. “Dream now” reviews the sessions since the last one.</p>}>
              <ul class="dream-runs" data-testid="dream-runs">
                <For each={s().runs}>
                  {(r) => (
                    <li class="dream-run" data-status={r.status}>
                      <div class="dream-run-head">
                        <span class="dream-run-when">{when(r.startedAt)}</span>
                        <span class="dream-dim">{r.status}{r.manual ? ' · manual' : ''} · {r.sessions} session{r.sessions === 1 ? '' : 's'} digested</span>
                        <span class="flex-1" />
                        <Show when={r.hasReport}>
                          <button type="button" class="settings-link" onClick={() => void openReport(r.id)} data-testid={`dream-report-${r.id}`}>Report</button>
                        </Show>
                        <Show when={r.status !== 'running'}>
                          <button type="button" class="settings-link" disabled={!!busy()} onClick={() => void act(`dismiss-${r.id}`, 'dream.dismiss', { runId: r.id })} title="Remove this run">Dismiss</button>
                        </Show>
                      </div>
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
                    </li>
                  )}
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
