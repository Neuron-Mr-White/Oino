import { createEffect, createMemo, createSignal, onCleanup, Show } from 'solid-js';
import { ChevronLeft, ChevronRight, Send } from 'lucide-solid';
import { ApprovalCard } from '../composer/ApprovalCard';
import { sessions } from '../../lib/session/store';
import { isApproval, isQuestion, superagentApi, type ApprovalItem, type NeedsItem } from './api';
import type { SuperagentState } from './state';
import { Limited, SectionHead } from './parts';

/** Clears the card as soon as the worker's own stream drops the request. */
function WatchApproval(props: { hostId: string; item: ApprovalItem; onGone: () => void }) {
  const worker = sessions().ensure(props.hostId, props.item.workerSessionId);
  const release = worker.acquire(); onCleanup(release);
  let seen = false;
  createEffect(() => {
    const pending = (worker.state.approvals ?? []).some((r) => r.requestId === props.item.requestId);
    if (pending) seen = true;
    else if (seen && worker.state.conn === 'live') props.onGone();
  });
  return null;
}

function Approvals(props: { hostId: string; sa: SuperagentState; items: ApprovalItem[]; onOpen: (id: string) => void; onTicket: (project: string, task: string) => void; onGone: (id: string, stale?: boolean) => void }) {
  const [index, setIndex] = createSignal(0);
  const at = () => Math.min(index(), props.items.length - 1);
  const item = () => props.items[at()]!;
  const title = () => props.sa.meta(item().projectId, item().taskId)?.title;
  async function decide(it: ApprovalItem, decision: 'allow_once' | 'always' | 'deny') {
    const result = await superagentApi.approvalResponse(props.hostId, it.projectId, it.taskId, it.workerSessionId, it.requestId, decision);
    if (!result.ok) {
      const detail = result.error.detail as { code?: string } | undefined;
      const stale = detail?.code === 'E_WORKER_CLAIM' || (detail?.code === 'E_APPROVAL' && /stale or wrong-session|approval request cancelled/i.test(result.error.message));
      if (!stale) throw new Error(result.error.message);
    }
    props.onGone(it.requestId, !result.ok);
    await props.sa.refresh();
  }
  return <div class="sa-approval" data-testid="sa-approval">
    <div class="sa-approval-head">
      <button type="button" class="sa-link" onClick={() => props.onTicket(item().projectId, item().taskId)}><span class="sa-project">{props.sa.projectName(item().projectId)}</span><strong>{item().taskId}{title() ? ` · ${title()}` : ''}</strong></button>
      <Show when={props.items.length > 1}>
        <span class="sa-pager" role="group" aria-label="Approvals">
          <button type="button" class="sa-icon-btn" aria-label="Previous approval" disabled={at() === 0} onClick={() => setIndex(at() - 1)}><ChevronLeft size={16} /></button>
          <span aria-live="polite">{at() + 1} of {props.items.length}</span>
          <button type="button" class="sa-icon-btn" aria-label="Next approval" disabled={at() >= props.items.length - 1} onClick={() => setIndex(at() + 1)}><ChevronRight size={16} /></button>
        </span>
      </Show>
    </div>
    <Show when={item().requestId} keyed>{(requestId) => {
      const it = props.items.find((i) => i.requestId === requestId)!;
      return <>
        <WatchApproval hostId={props.hostId} item={it} onGone={() => props.onGone(requestId)} />
        <ApprovalCard request={{ requestId, sessionId: it.sessionId, tool: it.tool, arguments: { command: it.command ?? '', summary: it.summary ?? undefined }, riskLevel: it.risk ?? null, ceiling: it.ceiling, reason: it.reason }} onDecision={(d) => decide(it, d)} />
        <button type="button" class="sa-link-quiet" onClick={() => props.onOpen(it.workerSessionId)}>Open lead session</button>
      </>;
    }}</Show>
  </div>;
}

function Question(props: { sa: SuperagentState; item: NeedsItem; onTicket: (project: string, task: string) => void }) {
  const [text, setText] = createSignal('');
  const key = () => `answer:${props.item.projectId}/${props.item.taskId}`;
  const title = () => props.sa.meta(props.item.projectId, props.item.taskId)?.title;
  async function send() {
    if (!text().trim() || props.sa.isBusy(key())) return;
    if (await props.sa.answer(props.item.projectId, props.item.taskId, text())) setText('');
  }
  return <article class="sa-question">
    <button type="button" class="sa-link" onClick={() => props.onTicket(props.item.projectId, props.item.taskId)}><span class="sa-project">{props.sa.projectName(props.item.projectId)} · Blocked</span><strong>{props.item.taskId}{title() ? ` · ${title()}` : ''}</strong></button>
    <p class="sa-question-text">{props.item.question}</p>
    <form class="sa-reply" onSubmit={(e) => { e.preventDefault(); void send(); }}>
      <textarea rows="1" aria-label={`Reply to ${props.item.taskId}`} placeholder="Reply to the lead" value={text()} disabled={props.sa.isBusy(key())}
        onInput={(e) => setText(e.currentTarget.value)}
        onKeyDown={(e) => { if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { e.preventDefault(); void send(); } }} />
      <button type="submit" class="sa-icon-btn sa-send" aria-label={`Send reply for ${props.item.taskId}`} disabled={!text().trim() || props.sa.isBusy(key())}><Send size={16} /></button>
    </form>
  </article>;
}

/** Only what needs the owner: approvals (one at a time) and blocked questions. */
export function NeedsYouSection(props: { hostId: string; sa: SuperagentState; onOpen: (id: string) => void; onTicket: (project: string, task: string) => void }) {
  const [cleared, setCleared] = createSignal<string[]>([]);
  const items = () => props.sa.status()?.needsYou ?? [];
  const approvals = createMemo(() => items().filter(isApproval).filter((a) => !cleared().includes(a.requestId)));
  const questions = createMemo(() => items().filter(isQuestion));
  const total = () => approvals().length + questions().length;
  function gone(id: string, stale = false) {
    setCleared((old) => (old.includes(id) ? old : [...old, id]));
    if (stale) props.sa.flash('This request is no longer pending');
  }
  return <section class="sa-sec" aria-label="Needs you" data-testid="sa-needs">
    <SectionHead title="Needs you" count={total()} />
    <Show when={total()} fallback={<p class="sa-empty">Nothing needs you.</p>}>
      <Show when={approvals().length}><Approvals hostId={props.hostId} sa={props.sa} items={approvals()} onOpen={props.onOpen} onTicket={props.onTicket} onGone={gone} /></Show>
      <Limited items={questions()} limit={2}>{(q) => <Question sa={props.sa} item={q} onTicket={props.onTicket} />}</Limited>
    </Show>
  </section>;
}
