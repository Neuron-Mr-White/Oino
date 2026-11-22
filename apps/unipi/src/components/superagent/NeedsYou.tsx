import { createEffect, createMemo, createSignal, createUniqueId, For, onCleanup, Show, type JSX } from 'solid-js';
import { ChevronLeft, ChevronRight, Send } from 'lucide-solid';
import { NoteField } from '../board/NoteField';
import { api as boardApi, filePath } from '../../lib/board/api';
import { uploadAll } from '../../lib/board/attach';
import { boardUi } from '../../lib/board/ui';
import { ApprovalCard } from '../composer/ApprovalCard';
import { Markdown } from '../board/Markdown';
import { sessions } from '../../lib/session/store';
import { useSelectedHost } from '../shell/HostSwitcher';
import { isClaimedBlockedQuestion, isLeadQuestion, superagentApi, type ApprovalItem, type NeedsItem } from './api';
import type { SuperagentState } from './state';
import { isLongReport, splitBlocked } from './blocked';
import { answerLeadQuestion, questionTiming } from './leads/question';
import { readDraft, writeDraft } from '../../lib/superagent/workbenchState';

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

function Approvals(props: { hostId: string; sa: SuperagentState; items: ApprovalItem[]; onOpen: (id: string) => void; onTicket: (project: string, task: string) => void; onGone: (id: string, stale?: boolean) => void; onCurrent?: (item: ApprovalItem) => void; /** Where the pager starts, and who hears when it moves (the workbench keeps it across a visit elsewhere). */ initialAt?: number; onAt?: (at: number) => void }) {
  const [index, setIndex] = createSignal(props.initialAt ?? 0);
  const at = () => Math.max(0, Math.min(index(), props.items.length - 1));
  const item = () => props.items[at()]!;
  createEffect(() => { const it = props.items[at()]; if (it) props.onCurrent?.(it); });
  createEffect(() => props.onAt?.(at()));
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
export function Question(props: { sa: SuperagentState; item: NeedsItem; /** The host the lead runs on; the selected one when absent (the workbench does not pass it). */ hostId?: string; onTicket?: (project: string, task: string) => void; onSent?: () => void; /** The workbench's "N of M" pager, kept with the pinned part. */ pager?: JSX.Element }) {
  const selected = useSelectedHost();
  /** Media of a reply go to the ticket it answers, whichever project the Board has open. */
  const replyHost = () => props.hostId ?? selected()?.id ?? '';
  const key = () => `answer:${props.item.projectId}/${props.item.taskId}`;
  /** A half-typed reply outlives the card (a visit to another page, the pager, a reload); sending clears it. */
  const [typed, setTyped] = createSignal(replyHost() ? readDraft(replyHost(), key()) : '');
  const text = typed;
  const setText = (value: string) => { setTyped(value); if (replyHost()) writeDraft(replyHost(), key(), value); };
  const [uploading, setUploading] = createSignal(0);
  const [failure, setFailure] = createSignal('');
  const [sending, setSending] = createSignal(false);
  /** Answered elsewhere: the card stays readable until the next status poll drops it, with nothing left to send. */
  const [stale, setStale] = createSignal(false);
  /** A lead's own question (OIN-81): still in progress, answered through the lead, and it becomes a blocked ticket when the wait runs out. */
  const lead = () => isLeadQuestion(props.item);
  /** A blocked ticket that still holds the lead's own claim: answered via `superagent.answer` to wake the same lead. */
  const claimedBlocked = () => isClaimedBlockedQuestion(props.item);
  const directLead = () => lead() || claimedBlocked();
  const choices = createMemo(() => (props.item.options?.[0] ?? []).map((o) => o.trim()).filter(Boolean));
  const timing = () => (lead() ? questionTiming(props.item, props.sa.now()) : null);
  const [open, setOpen] = createSignal(false);
  const reportId = createUniqueId();
  const title = () => props.sa.meta(props.item.projectId, props.item.taskId)?.title;
  const parts = createMemo(() => splitBlocked(props.item.question));
  const long = () => isLongReport(parts().report);
  const target = {
    upload: (files: File[]) => uploadAll(replyHost(), props.item.projectId, props.item.taskId, files, (message) => boardUi().toast(message, 'error')),
    thumb: (item: { name: string }) => boardApi.fileUrl(replyHost(), filePath(props.item.projectId, props.item.taskId, item.name)),
  };
  async function sendLead(raw = text()) {
    if (!raw.trim() || sending() || uploading() > 0) return;
    setSending(true); setFailure('');
    try {
      const result = await answerLeadQuestion(props.hostId ?? selected()?.id ?? '', props.item, raw);
      if (result.kind === 'sent') { setText(''); props.sa.flash(`Sent to the ${props.sa.projectName(props.item.projectId)} lead.`); props.onSent?.(); void props.sa.refresh(); }
      else { setFailure(result.message); if (result.kind === 'answered') setStale(true); }
    } finally { setSending(false); }
  }
  async function send(raw = text()) {
    if (directLead()) return sendLead(raw);
    if (!raw.trim() || uploading() > 0 || props.sa.isBusy(key())) return;
    if (await props.sa.answer(props.item.projectId, props.item.taskId, raw)) { setText(''); props.onSent?.(); }
  }
  const working = () => (directLead() ? sending() || stale() : props.sa.isBusy(key()));
  const heading = () => <><span class="sa-project">{props.sa.projectName(props.item.projectId)} · {lead() ? 'Lead is asking' : 'Blocked'}</span><strong>{props.item.taskId}{title() ? ` · ${title()}` : ''}</strong></>;
  return <article class="sa-question" data-testid={lead() ? 'sa-lead-question' : 'sa-question'}>
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
      <Show when={timing()}>{(t) => <p class="sa-ask-timing" data-testid="sa-ask-timing">{t()}</p>}</Show>
      <Show when={choices().length}>
        <div class="sa-ask-options" role="group" aria-label="Quick choices" data-testid="sa-ask-options">
          <For each={choices()}>{(choice) => (
            <button type="button" class="sa-btn" disabled={working() || uploading() > 0} onClick={() => void send(choice)}>{choice}</button>
          )}</For>
        </div>
      </Show>
      <div class="sa-reply">
        <NoteField
          {...target}
          class="sa-reply-note"
          taskId={props.item.taskId}
          label={`Reply to ${props.item.taskId}`}
          placeholder="Reply to the lead. Ctrl+Enter sends"
          value={text()}
          onInput={setText}
          uploading={uploading()}
          setUploading={setUploading}
          disabled={working()}
          onKeyDown={(e) => { if (e.key === 'Enter' && (e.ctrlKey || e.metaKey) && !e.isComposing) { e.preventDefault(); void send(); } }}
          trailing={<button type="button" class="sa-icon-btn sa-send" aria-label={`Send reply for ${props.item.taskId}`} disabled={!text().trim() || working() || uploading() > 0} onClick={() => void send()}><Send size={16} /></button>}
        />
      </div>
      <Show when={failure()}><p class="sa-ask-fail" role="alert" data-testid="sa-ask-error">{failure()}</p></Show>
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
export function ApprovalQueue(props: { hostId: string; sa: SuperagentState; items: ApprovalItem[]; onOpen: (id: string) => void; onTicket: (project: string, task: string) => void; onCurrent?: (item: ApprovalItem) => void; onAdvance?: () => void; initialAt?: number; onAt?: (at: number) => void }) {
  const [cleared, setCleared] = createSignal<string[]>([]);
  const items = createMemo(() => props.items.filter((a) => !cleared().includes(a.requestId)));
  function gone(id: string, stale = false) {
    setCleared((old) => (old.includes(id) ? old : [...old, id]));
    if (stale) props.sa.flash('This request is no longer pending');
    props.onAdvance?.();
  }
  return <section class="sa-approvals" aria-label="Approvals" data-testid="sa-approvals">
    <Show when={items().length} fallback={<p class="sa-empty">No approvals waiting.</p>}>
      <Approvals hostId={props.hostId} sa={props.sa} items={items()} onOpen={props.onOpen} onTicket={props.onTicket} onGone={gone} onCurrent={props.onCurrent} initialAt={props.initialAt} onAt={props.onAt} />
    </Show>
  </section>;
}
