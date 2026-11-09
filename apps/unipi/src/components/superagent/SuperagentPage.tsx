import { createEffect, createMemo, createResource, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { useNavigate, useSearchParams } from '@solidjs/router';
import { ArrowLeft, Bot, History, LayoutDashboard, Plus, Users } from 'lucide-solid';
import { toAppError } from '@unipi/protocol';
import { Conversation } from '../conversation/Conversation';
import { IconButton } from '../phone';
import { SheetFrame } from '../phone/Sheet';
import { HostChip, useSelectedHost } from '../shell/HostSwitcher';
import { layout } from '../../lib/layout';
import { sessions } from '../../lib/session/store';
import { buildTurns } from '../../lib/session/turns';
import { boardUi } from '../../lib/board/ui';
import { endQueue, openFromSuperagent, pruneQueue, taskOrigin, type QueueKind, type QueueRef } from '../../lib/board/taskOrigin';
import { isEmptySuperagentSession, openSuperagentSession, superagentApi } from './api';
import { createSuperagentState } from './state';
import { readSavedSuperagentSession, rememberSuperagentSession } from './chatActivity';
import { elapsed } from './format';
import { needsCount, SuperagentPane, type PaneTab } from './SuperagentPane';
import { ReviewTaskLayer } from './ReviewTaskLayer';
import { ReviewWorkspace } from './ReviewWorkspace';
import { restorePaneScroll, savePaneScroll } from './reviewUi';
import './superagent.css';

export default function SuperagentPage() {
  const selected = useSelectedHost();
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const [paneOpen, setPaneOpen] = createSignal(false);
  const [tab, setTab] = createSignal<PaneTab>('needs');
  const [openingError, setOpeningError] = createSignal('');
  const [retry, setRetry] = createSignal(0);
  const [historyOpen, setHistoryOpen] = createSignal(false);
  const [creating, setCreating] = createSignal(false);
  const hostId = () => selected()?.id;
  const sa = createSuperagentState(hostId);
  const needs = () => needsCount(sa);
  const remembered = new Map<string, string>();

  let initialHost = hostId();
  const initialSession = typeof params.session === 'string' ? params.session : undefined;
  let initialSessionHost = typeof params.host === 'string' ? params.host : initialHost;
  const [session] = createResource(() => ({ host: hostId(), retry: retry() }), async ({ host }) => {
    setOpeningError('');
    if (!host) return undefined;
    if (!initialHost) { initialHost = host; initialSessionHost ??= host; }
    const existing = remembered.get(host) ?? (initialSessionHost === host ? initialSession : undefined) ?? readSavedSuperagentSession(host);
    if (existing) { rememberSuperagentSession(host, existing); return { host, id: existing }; }
    try {
      const id = await openSuperagentSession(host);
      remembered.set(host, id);
      rememberSuperagentSession(host, id);
      return { host, id };
    } catch (e) { if (hostId() === host) setOpeningError(toAppError(e).message); return undefined; }
  });
  const current = createMemo(() => session()?.host === hostId() ? session() : undefined);
  const currentRun = () => sa.running().find((row) => row.sessionId === current()?.id);
  const currentRecord = () => sa.history().find((row) => row.sessionId === current()?.id);
  const duration = (start: string | number | null, now: number) => {
    if (start == null) return 'Elapsed unavailable';
    const time = new Date(start).getTime();
    return Number.isFinite(time) ? elapsed(new Date(time).toISOString(), now) ?? 'Elapsed unavailable' : 'Elapsed unavailable';
  };
  function selectSession(host: string, id: string) {
    remembered.set(host, id);
    rememberSuperagentSession(host, id);
    setHistoryOpen(false);
    setRetry((v) => v + 1);
  }
  async function newChat() {
    const host = hostId();
    if (!host || creating()) return;
    const selected = current();
    const handle = selected && sessions().peek(host, selected.id);
    if (handle && handle.state.conn === 'live' && !handle.state.outbox.length && !handle.state.run.running && buildTurns(handle.state).length === 0) return;
    setCreating(true); setOpeningError('');
    try {
      const list = await superagentApi.sessions(host);
      if (list.ok) {
        for (const item of list.value) {
          if (item.sessionId !== selected?.id && await isEmptySuperagentSession(host, item.sessionId)) {
            if (hostId() === host) selectSession(host, item.sessionId);
            return;
          }
        }
      }
      const id = await openSuperagentSession(host);
      if (hostId() === host) selectSession(host, id);
    }
    catch (e) { if (hostId() === host) setOpeningError(toAppError(e).message); }
    finally { setCreating(false); }
  }
  createEffect(() => { const s = current(); if (s) setParams({ session: s.id, host: s.host }, { replace: true }); });
  /** The queue as the owner sees it right now: Review rows, or the tickets waiting in Needs you. */
  function queueOf(queue: QueueKind): QueueRef[] {
    const status = sa.status();
    if (!status) return [];
    return queue === 'review'
      ? status.review.map((r) => ({ project: r.projectId, task: r.taskId }))
      : status.needsYou.filter((n) => n.taskId).map((n) => ({ project: n.projectId, task: n.taskId }));
  }
  /** Opens the ticket over this page (no route change): Back returns here, state untouched. */
  async function openTicket(project: string, task: string, queue: QueueKind) {
    const host = hostId(); if (!host) return;
    savePaneScroll();
    try { await openFromSuperagent(host, queue, queueOf(queue), { project, task }); }
    catch (e) { endQueue(); setOpeningError(toAppError(e).message); }
  }
  // Items the lead or another device moved on their own leave the open queue.
  createEffect(() => {
    const o = taskOrigin();
    if (!o || !sa.status() || params.review) return;
    pruneQueue(queueOf(o.queue));
  });
  // The ticket closed (Back, or the queue ran out): the phone's pane sheet comes back where it was.
  createEffect(on(() => boardUi().openedTask(), (now, prev) => {
    if (now !== undefined || prev === undefined || !taskOrigin() || params.review) return;
    endQueue();
    void sa.refresh(); // the tickets reviewed meanwhile leave the Review list
    // The desktop pane never unmounted; the phone's sheet did, so it reopens on the queue's tab.
    if (!layout().desktop()) { setTab('needs'); setPaneOpen(true); restorePaneScroll(); }
  }));
  onCleanup(() => { if (taskOrigin()) { boardUi().closeTask(); endQueue(); } });
  const pane = () => <Show when={hostId()}>{(host) => <SuperagentPane hostId={host()} sa={sa} tab={tab()} onTab={setTab} onOpen={(id) => { setPaneOpen(false); selectSession(host(), id); }} onTicket={(project, task, queue) => { setPaneOpen(false); void openTicket(project, task, queue); }} />}</Show>;
  return <div class="sa-page" data-testid="superagent-page"><div class="sa-center"><header class="sa-header"><Show when={!layout().desktop()}><button class="shell-icon-btn" aria-label="Back to chats" onClick={() => navigate('/chat')}><ArrowLeft size={18} /></button></Show><Bot size={19} /><h1>Superagent</h1><HostChip /><ReviewWorkspace sa={sa} hostId={hostId} /><button class="shell-icon-btn" aria-label="Kanboard dashboard" title="Kanboard dashboard" onClick={() => { boardUi().setOverview(true); navigate('/board'); }}><LayoutDashboard size={19} /></button><button class="shell-icon-btn sa-count-control" aria-label="New superagent chat" title={`New superagent chat · ${sa.running().length} running`} disabled={creating() || !hostId()} onClick={() => void newChat()}><Plus size={19} /><Show when={sa.running().length}><span>{sa.running().length}</span></Show></button><button class="shell-icon-btn sa-count-control" aria-label="Superagent history" title={`Superagent history · ${sa.running().length} running`} aria-expanded={historyOpen()} onClick={() => { setHistoryOpen(!historyOpen()); if (historyOpen()) void sa.refresh(); }}><History size={19} /><Show when={sa.running().length}><span>{sa.running().length}</span></Show></button><Show when={!layout().desktop()}><IconButton icon={Users} label={needs() ? `Needs you · ${needs()}` : 'Needs you, leads and projects'} badge={needs()} testId="sa-sheet-open" onClick={() => { if (needs()) setTab('needs'); setPaneOpen(true); }} /></Show></header>
    <Show when={current()}><div class="sa-current" data-testid="sa-current"><span classList={{ 'sa-live-dot': !!currentRun(), 'sa-idle-dot': !currentRun() }} /><strong>{currentRun()?.title ?? currentRecord()?.title ?? 'Superagent'}</strong><span>{currentRun()?.project ?? currentRecord()?.project ?? 'No project'}</span><Show when={currentRun()}>{(row) => <><span>{duration(row().startedAt, sa.now())}</span><button type="button" onClick={() => void sa.stopSession(row())}>Stop</button></>}</Show></div></Show>
    <Show when={historyOpen()}><div class="sa-history" aria-label="Superagent history list"><h2>Running now</h2><Show when={sa.running().length} fallback={<p class="sa-empty">No sessions running.</p>}><For each={sa.running()}>{(row) => <button classList={{ 'sa-history-selected': row.open }} aria-current={row.open ? 'true' : undefined} onClick={() => { const host = hostId(); if (host) selectSession(host, row.sessionId); }}><span class="sa-live-dot" /><span>{row.title}<small>{row.project ?? 'No project'} · {duration(row.startedAt, sa.now())}</small></span></button>}</For></Show><h2>Recent</h2><For each={sa.history().filter((item) => !sa.running().some((row) => row.sessionId === item.sessionId))}>{(item) => <button class="sa-history-recent" onClick={() => { const host = hostId(); if (host) selectSession(host, item.sessionId); }}><span>{item.title || 'Superagent'}<small>Ended {new Date(item.updatedAt).toLocaleString()} · {duration(item.createdAt, new Date(item.updatedAt).getTime())}</small></span></button>}</For></div></Show>
    <Show when={openingError()}><div role="alert" class="sa-message">{openingError()} <button onClick={() => setRetry((v) => v + 1)}>Retry</button></div></Show>
    <Show when={hostId()} fallback={<p class="sa-empty">Connect a computer to use Superagent.</p>}><Show when={current()} fallback={<div class="sa-loading" aria-busy="true" aria-label="Opening Superagent"><span /><span /></div>}>{(s) => <Conversation hostId={s().host} sessionId={s().id} superagent />}</Show></Show>
  </div><Show when={layout().desktop()} fallback={<SheetFrame open={paneOpen()} onClose={() => setPaneOpen(false)} label="Superagent overview" title="Superagent" detents={['full']}>{pane()}</SheetFrame>}><aside class="sa-side">{pane()}</aside></Show><Show when={!params.review}><ReviewTaskLayer /></Show></div>;
}
