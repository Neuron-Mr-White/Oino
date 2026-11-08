import { createEffect, createMemo, createResource, createSignal, For, Show } from 'solid-js';
import { useNavigate, useSearchParams } from '@solidjs/router';
import { ArrowLeft, Bot, History, LayoutDashboard, Plus, Users } from 'lucide-solid';
import { toAppError } from '@unipi/protocol';
import { Conversation } from '../conversation/Conversation';
import { IconButton } from '../phone';
import { SheetFrame } from '../phone/Sheet';
import { HostChip, useSelectedHost } from '../shell/HostSwitcher';
import { layout } from '../../lib/layout';
import { boardUi, rememberLast } from '../../lib/board/ui';
import { boardStore } from '../../lib/board/store';
import { sessionHref } from '../shell/Sidebar';
import { openSuperagentSession, superagentApi } from './api';
import { createSuperagentState } from './state';
import { needsCount, SuperagentPane, type PaneTab } from './SuperagentPane';
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
  const [history, { refetch: refreshHistory }] = createResource(() => historyOpen() ? hostId() : undefined, async (host) => {
    const result = await superagentApi.sessions(host);
    if (!result.ok && String(result.error.code) !== 'E_UNKNOWN_OP') setOpeningError(result.error.message);
    return result.ok ? result.value : [];
  });
  const hostId = () => selected()?.id;
  const sa = createSuperagentState(hostId);
  const needs = () => needsCount(sa);
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
  const pane = () => <Show when={hostId()}>{(host) => <SuperagentPane hostId={host()} sa={sa} tab={tab()} onTab={setTab} onOpen={(id) => { setPaneOpen(false); navigate(sessionHref(host(), id)); }} onTicket={(project, task) => { setPaneOpen(false); void openTicket(project, task); }} />}</Show>;
  return <div class="sa-page" data-testid="superagent-page"><div class="sa-center"><header class="sa-header"><Show when={!layout().desktop()}><button class="shell-icon-btn" aria-label="Back to chats" onClick={() => navigate('/chat')}><ArrowLeft size={18} /></button></Show><Bot size={19} /><h1>Superagent</h1><HostChip /><button class="shell-icon-btn" aria-label="Kanboard dashboard" title="Kanboard dashboard" onClick={() => { boardUi().setOverview(true); navigate('/board'); }}><LayoutDashboard size={19} /></button><button class="shell-icon-btn" aria-label="New superagent chat" title="New superagent chat" disabled={creating() || !hostId()} onClick={() => void newChat()}><Plus size={19} /></button><button class="shell-icon-btn" aria-label="Superagent history" title="Superagent history" aria-expanded={historyOpen()} onClick={() => { setHistoryOpen(!historyOpen()); if (historyOpen()) void refreshHistory(); }}><History size={19} /></button><Show when={!layout().desktop()}><IconButton icon={Users} label={needs() ? `Needs you · ${needs()}` : 'Needs you, leads and projects'} badge={needs()} testId="sa-sheet-open" onClick={() => { if (needs()) setTab('needs'); setPaneOpen(true); }} /></Show></header>
    <Show when={historyOpen()}><div class="sa-history" aria-label="Superagent history list"><Show when={!history.loading} fallback={<div class="sa-loading" aria-label="Loading history" aria-busy="true"><span /></div>}><Show when={history()?.length} fallback={<p class="sa-empty">No past superagent chats.</p>}><For each={history()}>{(item) => <button onClick={() => { const host = hostId(); if (host) selectSession(host, item.sessionId); }}><span>{item.title || 'Superagent'}</span><small>{typeof item.updatedAt === 'number' ? new Date(item.updatedAt).toLocaleDateString() : item.updatedAt}</small></button>}</For></Show></Show></div></Show>
    <Show when={openingError()}><div role="alert" class="sa-message">{openingError()} <button onClick={() => setRetry((v) => v + 1)}>Retry</button></div></Show>
    <Show when={hostId()} fallback={<p class="sa-empty">Connect a computer to use Superagent.</p>}><Show when={current()} fallback={<div class="sa-loading" aria-busy="true" aria-label="Opening Superagent"><span /><span /></div>}>{(s) => <Conversation hostId={s().host} sessionId={s().id} superagent />}</Show></Show>
  </div><Show when={layout().desktop()} fallback={<SheetFrame open={paneOpen()} onClose={() => setPaneOpen(false)} label="Superagent overview" title="Superagent" detents={['full']}>{pane()}</SheetFrame>}><aside class="sa-side">{pane()}</aside></Show></div>;
}
