import { createEffect, createMemo, createSignal, createUniqueId, onCleanup, Show, type JSX } from 'solid-js';
import { ChevronLeft, ChevronRight, Send } from 'lucide-solid';
import { ApprovalCard } from '../composer/ApprovalCard';
import { Markdown } from '../board/Markdown';
import { sessions } from '../../lib/session/store';
import { superagentApi, type ApprovalItem, type NeedsItem } from './api';
import type { SuperagentState } from './state';
import { isLongReport, splitBlocked } from './blocked';

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

function Approvals(props: { hostId: string; sa: SuperagentState; items: ApprovalItem[]; onOpen: (id: string) => void; onTicket: (project: string, task: string) => void; onGone: (id: string, stale?: boolean) => void; onCurrent?: (item: ApprovalItem) => void }) {
  const [index, setIndex] = createSignal(0);
  const at = () => Math.min(index(), props.items.length - 1);
  const item = () => props.items[at()]!;
  createEffect(() => { const it = props.items[at()]; if (it) props.onCurrent?.(it); });
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

/**
 * One blocked ticket as a decision (OIN-62): the question pinned at the top and large, the reply
 * box under it, the lead's report behind "Read full report". `onSent` fires once the lead got the answer.
 */
export function Question(props: { sa: SuperagentState; item: NeedsItem; onTicket?: (project: string, task: string) => void; onSent?: () => void; /** The workbench's "N of M" pager, kept with the pinned part. */ pager?: JSX.Element }) {
  const [text, setText] = createSignal('');
  const [open, setOpen] = createSignal(false);
  const reportId = createUniqueId();
  const key = () => `answer:${props.item.projectId}/${props.item.taskId}`;
  const title = () => props.sa.meta(props.item.projectId, props.item.taskId)?.title;
  const parts = createMemo(() => splitBlocked(props.item.question));
  const long = () => isLongReport(parts().report);
  async function send() {
    if (!text().trim() || props.sa.isBusy(key())) return;
    if (await props.sa.answer(props.item.projectId, props.item.taskId, text())) { setText(''); props.onSent?.(); }
  }
  const heading = () => <><span class="sa-project">{props.sa.projectName(props.item.projectId)} · Blocked</span><strong>{props.item.taskId}{title() ? ` · ${title()}` : ''}</strong></>;
  return <article class="sa-question" data-testid="sa-question">
    <div class="sa-pin">
      <div class="sa-approval-head">
        <Show when={props.onTicket} fallback={<div class="sa-link">{heading()}</div>}>
          <button type="button" class="sa-link" onClick={() => props.onTicket!(props.item.projectId, props.item.taskId)}>{heading()}</button>
        </Show>
        {props.pager}
      </div>
      <Show when={parts().reason}><p class="sa-question-reason" data-testid="sa-question-reason" title={parts().reason}>{parts().reason}</p></Show>
      <section class="sa-ask" aria-label="Need from you" data-testid="sa-ask">
        <h3>Need from you</h3>
        <Markdown text={parts().ask} class="sa-ask-text" />
      </section>
      <form class="sa-reply" onSubmit={(e) => { e.preventDefault(); void send(); }}>
        <textarea rows="1" aria-label={`Reply to ${props.item.taskId}`} placeholder="Reply to the lead" value={text()} disabled={props.sa.isBusy(key())}
          onInput={(e) => setText(e.currentTarget.value)}
          onKeyDown={(e) => { if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { e.preventDefault(); void send(); } }} />
        <button type="submit" class="sa-icon-btn sa-send" aria-label={`Send reply for ${props.item.taskId}`} disabled={!text().trim() || props.sa.isBusy(key())}><Send size={16} /></button>
      </form>
      <Show when={long()}>
        <button type="button" class="sa-report-toggle" aria-expanded={open()} aria-controls={reportId} onClick={() => setOpen(!open())}>
          <ChevronRight size={14} class="sa-chev" aria-hidden="true" />{open() ? 'Hide full report' : 'Read full report'}
        </button>
      </Show>
    </div>
    <Show when={parts().report && (!long() || open())}>
      <div class="sa-report" id={reportId} role="region" aria-label="Full report" data-testid="sa-report"><Markdown text={parts().report} class="sa-report-md" /></div>
    </Show>
  </article>;
}

/** The workbench's Approve tab: approvals one at a time, a decided or stale one drops out at once. */
export function ApprovalQueue(props: { hostId: string; sa: SuperagentState; items: ApprovalItem[]; onOpen: (id: string) => void; onTicket: (project: string, task: string) => void; onCurrent?: (item: ApprovalItem) => void; onAdvance?: () => void }) {
  const [cleared, setCleared] = createSignal<string[]>([]);
  const items = createMemo(() => props.items.filter((a) => !cleared().includes(a.requestId)));
  function gone(id: string, stale = false) {
    setCleared((old) => (old.includes(id) ? old : [...old, id]));
    if (stale) props.sa.flash('This request is no longer pending');
    props.onAdvance?.();
  }
  return <section class="sa-approvals" aria-label="Approvals" data-testid="sa-approvals">
    <Show when={items().length} fallback={<p class="sa-empty">No approvals waiting.</p>}>
      <Approvals hostId={props.hostId} sa={props.sa} items={items()} onOpen={props.onOpen} onTicket={props.onTicket} onGone={gone} onCurrent={props.onCurrent} />
    </Show>
  </section>;
}
