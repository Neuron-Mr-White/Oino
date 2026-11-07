import { createEffect, createMemo, createResource, createSignal, For, onCleanup, Show } from 'solid-js';
import { useNavigate, useSearchParams } from '@solidjs/router';
import { ArrowLeft, Bot, History, Plus, RefreshCw, Users } from 'lucide-solid';
import { toAppError } from '@unipi/protocol';
import { Conversation } from '../conversation/Conversation';
import { ApprovalCard } from '../composer/ApprovalCard';
import { Button, confirmSheet } from '../phone';
import { SheetFrame } from '../phone/Sheet';
import { SwitchField } from '../Switch';
import { HostChip, useSelectedHost } from '../shell/HostSwitcher';
import { within } from '../../lib/host/within';
import { layout } from '../../lib/layout';
import { api as boardApi } from '../../lib/board/api';
import { boardUi, rememberLast } from '../../lib/board/ui';
import { boardStore } from '../../lib/board/store';
import { sessionHref } from '../shell/Sidebar';
import { ConfigSchema, openSuperagentSession, RelayTextSchema, superagentApi, type SuperagentStatus, type Worker } from './api';
import './superagent.css';

export function autonomyLabel(level: string | undefined): string {
  return `Autonomy: ${level === 'low' ? 'Low' : level === 'high' ? 'High' : 'Medium'}`;
}

export function WorkersPane(props: { hostId: string; onOpen: (id: string) => void; onTicket: (project: string, task: string) => void }) {
  const [status, setStatus] = createSignal<SuperagentStatus>();
  const [error, setError] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  const [cap, setCap] = createSignal('1');
  const [capDirty, setCapDirty] = createSignal(false);
  const [relayTo, setRelayTo] = createSignal<Worker>();
  const [text, setText] = createSignal('');
  const [notice, setNotice] = createSignal('');
  const [now, setNow] = createSignal(Date.now());
  const [titles, setTitles] = createSignal<Record<string, { title: string; autonomy?: string }>>({});
  let generation = 0;
  let loading = false;
  async function refresh() {
    if (loading) return;
    loading = true;
    const gen = generation;
    const host = props.hostId;
    try {
      const result = await within(superagentApi.status(props.hostId), () => ({ ok: false as const, error: { code: 'E_TIMEOUT' as const, message: 'The host is not answering. Retry.', retryable: true } }));
      if (gen !== generation) return;
      if (!result.ok) { setError(result.error.message); return; }
      setStatus(result.value);
      if (!busy() && !capDirty()) setCap(String(result.value.concurrencyCap));
      setError('');
      const rows = [...result.value.workers, ...result.value.needsYou];
      const metadata = await Promise.all(rows.map(async (row) => {
        const key = `${row.projectId}/${row.taskId}`;
        try {
          const task = await within(boardApi.task(host, row.projectId, row.taskId), () => undefined);
          if (!task) return [key, { title: '' }] as const;
          return [key, { title: task.displayTitle || task.title, autonomy: task.autonomy ?? 'medium' }] as const;
        } catch { return [key, { title: '' }] as const; }
      }));
      if (gen === generation) setTitles(Object.fromEntries(metadata));
    } catch (e) { if (gen === generation) setError(toAppError(e).message); }
    finally { if (gen === generation) loading = false; }
  }
  createEffect(() => {
    props.hostId;
    generation++;
    loading = false;
    setStatus(undefined); setError(''); setTitles({}); setRelayTo(undefined); setNotice(''); setBusy(false); setCapDirty(false);
    void refresh();
    const timer = setInterval(() => { setNow(Date.now()); void refresh(); }, 5000);
    onCleanup(() => { generation++; clearInterval(timer); });
  });
  const meta = (project: string, task: string) => titles()[`${project}/${task}`];
  async function configure(enabled: boolean) {
    if (busy()) return;
    const parsed = ConfigSchema.safeParse({ enabled, concurrencyCap: Number(cap()) });
    if (!parsed.success) { setError('Concurrency cap must be a whole number of at least 1.'); return; }
    setBusy(true); setError('');
    const gen = generation;
    try {
      const result = await superagentApi.configure(props.hostId, parsed.data);
      if (gen !== generation) return;
      if (!result.ok) setError(result.error.message);
      else { setStatus((old) => old && ({ ...old, ...result.value })); setCap(String(result.value.concurrencyCap)); setCapDirty(false); }
    } catch (e) { if (gen === generation) setError(toAppError(e).message); }
    finally { if (gen === generation) setBusy(false); }
  }
  async function stop(worker: Worker) {
    if (busy()) return;
    const host = props.hostId;
    const gen = generation;
    if (!(await confirmSheet({ title: `Stop ${worker.taskId}?`, body: 'The worker will stop. Open its ticket to review progress before retrying.', confirm: 'Stop worker', tone: 'danger' }))) return;
    if (gen !== generation) return;
    setBusy(true);
    try {
      const result = await superagentApi.stop(host, worker.projectId, worker.taskId);
      if (gen !== generation) return;
      if (!result.ok) setError(result.error.message);
      else { setNotice(result.value.stopped ? `${worker.taskId} stopped.` : `${worker.taskId} is no longer running.`); await refresh(); }
    } catch (e) { if (gen === generation) setError(toAppError(e).message); }
    finally { if (gen === generation) setBusy(false); }
  }
  async function relay() {
    const worker = relayTo();
    const parsed = RelayTextSchema.safeParse(text());
    if (!worker || busy()) return;
    if (!parsed.success) { setError('Enter a message to relay.'); return; }
    const gen = generation;
    setBusy(true); setError('');
    try {
      const result = await superagentApi.relay(props.hostId, worker.projectId, worker.taskId, parsed.data);
      if (gen !== generation) return;
      if (!result.ok) setError(result.error.message);
      else if (!result.value.delivered) setError(`Comment saved, but the worker did not receive it. Open its session before retrying. ${typeof result.value.error === 'string' ? result.value.error : (result.value.error as { message?: string } | undefined)?.message ?? ''}`);
      else { setNotice(`Message delivered to ${worker.taskId} and saved as a ticket comment.`); setText(''); setRelayTo(undefined); }
    } catch (e) { if (gen === generation) setError(toAppError(e).message); }
    finally { if (gen === generation) setBusy(false); }
  }
  const elapsed = (started: string | null) => {
    if (!started || !Number.isFinite(Date.parse(started))) return 'Not reported';
    const minutes = Math.floor(Math.max(0, now() - Date.parse(started)) / 60_000);
    return minutes < 1 ? '<1 min' : `${minutes} min`;
  };
  return <div class="sa-pane" aria-label="Workers and needs you">
    <div class="sa-pane-heading"><h2>Work across projects</h2><button class="shell-icon-btn" aria-label="Refresh workers" onClick={() => void refresh()}><RefreshCw size={16} /></button></div>
    <Show when={error()}><div class="sa-message" role="alert">{error()} <button onClick={() => void refresh()}>Retry</button></div></Show>
    <Show when={notice()}><p class="sa-message" role="status">{notice()}</p></Show>
    <Show when={status()} fallback={<Show when={!error()}><div class="sa-loading" aria-label="Loading workers" aria-busy="true"><span /><span /><span /></div></Show>}>
      {(s) => <>
        <div class="sa-controls"><div class="sa-control-row"><div><strong>Work watcher</strong><p>{s().enabled ? 'Starts ready tickets automatically' : 'Off · manual starts still available'}</p></div><SwitchField label="Work watcher" checked={s().enabled} disabled={busy() || !!error()} onChange={(v) => void configure(v)} /></div>
          <form class="sa-control-row" onSubmit={(e) => { e.preventDefault(); void configure(s().enabled); }}><label for="sa-cap">Concurrency cap</label><div class="sa-cap"><input id="sa-cap" aria-label="Concurrency cap" type="number" min="1" step="1" value={cap()} disabled={busy()} onInput={(e) => { setCap(e.currentTarget.value); setCapDirty(true); }} /><Button disabled={busy() || cap() === String(s().concurrencyCap)} onClick={() => void configure(s().enabled)}>Save</Button></div></form>
        </div>
        <section class="sa-list"><h2>Workers <span>{s().workers.length}</span></h2><Show when={s().workers.length} fallback={<p class="sa-empty">No live workers. Ask Superagent to start a ready ticket.</p>}>
          <For each={s().workers}>{(worker) => <article class="sa-worker"><button class="sa-ticket" onClick={() => props.onTicket(worker.projectId, worker.taskId)}><span class="sa-project">{worker.projectId}</span><strong>{worker.taskId}{meta(worker.projectId, worker.taskId)?.title ? ` · ${meta(worker.projectId, worker.taskId)!.title}` : ''}</strong></button><div class="sa-worker-state"><span class="sa-live-dot" />Running<span class="sa-autonomy">{autonomyLabel(worker.autonomy)}</span></div><dl><div><dt>Step</dt><dd>{worker.step ?? 'Not reported'}</dd></div><div><dt>Tool</dt><dd>{worker.tool ?? 'Not reported'}</dd></div><div><dt>Elapsed</dt><dd>{elapsed(worker.startedAt)}</dd></div><div><dt>Cost</dt><dd>{worker.cost === null ? 'Not reported' : `$${worker.cost.toFixed(2)}`}</dd></div></dl><div class="sa-actions"><Button onClick={() => props.onOpen(worker.sessionId)}>Open</Button><Button disabled={busy()} onClick={() => { setRelayTo(worker); setText(''); }}>Relay</Button><Button disabled={busy()} kind="danger" onClick={() => void stop(worker)}>Stop</Button></div></article>}</For>
        </Show></section>
        <section class="sa-list"><h2>Needs you <span>{s().needsYou.length}</span></h2><Show when={s().needsYou.length} fallback={<p class="sa-empty">Nothing waiting for you.</p>}><For each={s().needsYou}>{(ticket) => <Show when={ticket.requestId && ticket.workerSessionId && ticket.sessionId && ticket.tool && ticket.ceiling && ticket.reason} fallback={<button class="sa-needs" onClick={() => props.onTicket(ticket.projectId, ticket.taskId)}><span class="sa-project">{ticket.projectId} · {ticket.status === 'blocked' ? 'Blocked' : 'In review'}</span><strong>{ticket.taskId}{meta(ticket.projectId, ticket.taskId)?.title ? ` · ${meta(ticket.projectId, ticket.taskId)!.title}` : ''}</strong><Show when={meta(ticket.projectId, ticket.taskId)?.autonomy}>{(level) => <span class="sa-autonomy">{autonomyLabel(level())}</span>}</Show><Show when={ticket.question}><p>{ticket.question}</p></Show></button>}><div><button class="sa-needs" onClick={() => props.onTicket(ticket.projectId, ticket.taskId)}><span class="sa-project">{ticket.projectId}</span><strong>{ticket.taskId}</strong></button><Button onClick={() => props.onOpen(ticket.workerSessionId!)}>Open worker</Button><ApprovalCard request={{ requestId: ticket.requestId!, sessionId: ticket.sessionId!, tool: ticket.tool!, arguments: { command: ticket.command ?? '', summary: ticket.summary ?? undefined }, riskLevel: ticket.risk ?? null, ceiling: ticket.ceiling!, reason: ticket.reason! }} onDecision={async (decision) => { const result = await superagentApi.approvalResponse(props.hostId, ticket.projectId, ticket.taskId, ticket.workerSessionId!, ticket.requestId!, decision); if (!result.ok) throw new Error(result.error.message); await refresh(); }} /></div></Show>}</For></Show></section>
      </>}
    </Show>
    <SheetFrame open={!!relayTo()} onClose={() => { if (!busy()) setRelayTo(undefined); }} label="Relay to worker" level={2} title={`Relay to ${relayTo()?.taskId ?? 'worker'}`} noDrag footer={<Button kind="primary" disabled={busy()} onClick={() => void relay()}>Send relay</Button>}>
      <p class="sa-relay-hint">Send an explicit follow-up and save it as a ticket comment.</p><textarea aria-label="Message to worker" value={text()} onInput={(e) => setText(e.currentTarget.value)} class="sa-relay-input" /><Show when={error()}><p role="alert" class="sa-message">{error()}</p></Show>
    </SheetFrame>
  </div>;
}

export default function SuperagentPage() {
  const selected = useSelectedHost();
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const [paneOpen, setPaneOpen] = createSignal(false);
  const [openingError, setOpeningError] = createSignal('');
  const [retry, setRetry] = createSignal(0);
  const [historyOpen, setHistoryOpen] = createSignal(false);
  const [creating, setCreating] = createSignal(false);
  const [history, { refetch: refreshHistory }] = createResource(() => historyOpen() ? hostId() : undefined, async (host) => {
    const result = await superagentApi.sessions(host);
    if (!result.ok && String(result.error.code) !== 'E_UNKNOWN_OP') setOpeningError(result.error.message);
    return result.ok ? result.value : [];
  });
  const hostId = () => selected()?.id;
  const remembered = new Map<string, string>();
  const savedKey = (host: string) => `unipi.superagent.session.${host}`;
  const readSaved = (host: string) => { try { return localStorage.getItem(savedKey(host)) || undefined; } catch { return undefined; } };
  let initialHost = hostId();
  const initialSession = typeof params.session === 'string' ? params.session : undefined;
  let initialSessionHost = typeof params.host === 'string' ? params.host : initialHost;
  const [session] = createResource(() => ({ host: hostId(), retry: retry() }), async ({ host }) => {
    setOpeningError('');
    if (!host) return undefined;
    if (!initialHost) { initialHost = host; initialSessionHost ??= host; }
    const existing = remembered.get(host) ?? (initialSessionHost === host ? initialSession : undefined) ?? readSaved(host);
    if (existing) return { host, id: existing };
    try {
      const id = await openSuperagentSession(host);
      remembered.set(host, id);
      try { localStorage.setItem(savedKey(host), id); } catch { /* Private storage: route still keeps the session. */ }
      return { host, id };
    } catch (e) { if (hostId() === host) setOpeningError(toAppError(e).message); return undefined; }
  });
  const current = createMemo(() => session()?.host === hostId() ? session() : undefined);
  function selectSession(host: string, id: string) {
    remembered.set(host, id);
    try { localStorage.setItem(savedKey(host), id); } catch { /* Route remains usable without storage. */ }
    setHistoryOpen(false);
    setRetry((v) => v + 1);
  }
  async function newChat() {
    const host = hostId();
    if (!host || creating()) return;
    setCreating(true); setOpeningError('');
    try { const id = await openSuperagentSession(host); if (hostId() === host) selectSession(host, id); }
    catch (e) { if (hostId() === host) setOpeningError(toAppError(e).message); }
    finally { setCreating(false); }
  }
  createEffect(() => { const s = current(); if (s) setParams({ session: s.id, host: s.host }, { replace: true }); });
  async function openTicket(project: string, task: string) {
    const host = hostId(); if (!host) return;
    try {
      rememberLast(host, project);
      await boardStore().open(host, project);
      boardUi().openTask(task);
      navigate('/board');
    } catch (e) { setOpeningError(toAppError(e).message); }
  }
  const pane = () => <Show when={hostId()}>{(host) => <WorkersPane hostId={host()} onOpen={(id) => navigate(sessionHref(host(), id))} onTicket={(project, task) => void openTicket(project, task)} />}</Show>;
  return <div class="sa-page" data-testid="superagent-page"><div class="sa-center"><header class="sa-header"><Show when={!layout().desktop()}><button class="shell-icon-btn" aria-label="Back to chats" onClick={() => navigate('/chat')}><ArrowLeft size={18} /></button></Show><Bot size={19} /><h1>Superagent</h1><HostChip /><button class="shell-icon-btn" aria-label="New superagent chat" title="New superagent chat" disabled={creating() || !hostId()} onClick={() => void newChat()}><Plus size={19} /></button><button class="shell-icon-btn" aria-label="Superagent history" title="Superagent history" aria-expanded={historyOpen()} onClick={() => { setHistoryOpen(!historyOpen()); if (historyOpen()) void refreshHistory(); }}><History size={19} /></button><Show when={!layout().desktop()}><button class="shell-icon-btn" aria-label="Workers and needs you" onClick={() => setPaneOpen(true)}><Users size={19} /></button></Show></header>
    <Show when={historyOpen()}><div class="sa-history" aria-label="Superagent history list"><Show when={!history.loading} fallback={<div class="sa-loading" aria-label="Loading history" aria-busy="true"><span /></div>}><Show when={history()?.length} fallback={<p class="sa-empty">No past superagent chats.</p>}><For each={history()}>{(item) => <button onClick={() => { const host = hostId(); if (host) selectSession(host, item.sessionId); }}>{item.title || 'Superagent'}<small>{item.updatedAt}</small></button>}</For></Show></Show></div></Show>
    <Show when={openingError()}><div role="alert" class="sa-message">{openingError()} <button onClick={() => setRetry((v) => v + 1)}>Retry</button></div></Show>
    <Show when={hostId()} fallback={<p class="sa-empty">Connect a computer to use Superagent.</p>}><Show when={current()} fallback={<div class="sa-loading" aria-busy="true" aria-label="Opening Superagent"><span /><span /></div>}>{(s) => <Conversation hostId={s().host} sessionId={s().id} superagent />}</Show></Show>
  </div><Show when={layout().desktop()} fallback={<SheetFrame open={paneOpen()} onClose={() => setPaneOpen(false)} label="Workers and needs you" title="Workers and needs you" detents={['full']}>{pane()}</SheetFrame>}><aside class="sa-side">{pane()}</aside></Show></div>;
}
