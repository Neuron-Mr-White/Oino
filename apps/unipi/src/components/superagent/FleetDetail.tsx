import { paneVisible } from '../../lib/topics/pane';
import { fallbackPoll } from '../../lib/topics/poll-allowlist';
import { topicsFor } from '../../lib/topics/client';
import { createEffect, createMemo, createSignal, onCleanup, Show } from 'solid-js';
import { ExternalLink, Play, RotateCcw, Shuffle, Square, TriangleAlert, X } from 'lucide-solid';
import { sessions } from '../../lib/session/store';
import { sessionRefChat } from '../../lib/timeline/refChat';
import { modelDisplayName } from '../../lib/model/displayName';
import { commandNames, type TimelineCtx } from '../timeline/context';
import { Timeline } from '../timeline/Timeline';
import { SubagentTimeline } from '../panes/tabs/SubagentTimeline';
import { fetchAgentMessages, fetchRecordTranscript } from '../panes/tabs/SubagentTab';
import { HostModelDialog } from '../model/HostModelPicker';
import type { AgentMessage } from '../../lib/session/types';
import type { FleetNode, LeadAction, NeedsItem } from './api';
import type { SuperagentState } from './state';
import { Question } from './NeedsYou';
import { age, elapsed, MODE_LABEL, nodeTitle, openTarget, STATUS_LABEL, type FleetTree } from './fleet';
import { Ring } from './canvas/parts';
import '../timeline/timeline.css';

const POLL_MS = 1500;

export type PeekProps = {
  hostId: string; sa: SuperagentState; node: FleetNode; tree: FleetTree; phone?: boolean;
  /** A lead action is in flight / refused (shown on the health card). */
  busy?: boolean; error?: string;
  onAction: (action: LeadAction, model?: string) => void;
  onOpen: () => void; onClose: () => void;
};

/** Stalled: why, how often, and the three ways out. Only a lead can be acted on. */
function HealthCard(props: PeekProps) {
  const [picking, setPicking] = createSignal(false);
  let pick: HTMLButtonElement | undefined;
  const lead = () => props.node.role === 'lead';
  const h = () => props.node.health;
  return <section class="pk-card pk-health" aria-label="Stalled" data-testid="peek-health">
    <h3><TriangleAlert size={15} aria-hidden="true" />Stalled</h3>
    <p class="pk-err">{h()?.error || 'It stopped with an error.'}</p>
    <p class="pk-sub">{h()?.attempts ? `${h()!.attempts} attempt${h()!.attempts === 1 ? '' : 's'}` : ''}{h()?.attempts && h()?.since ? ' · ' : ''}{h()?.since ? `since ${age(props.sa.now() - h()!.since!)} ago` : ''}</p>
    <Show when={lead()}>
      <div class="pk-actions">
        <button type="button" class="sac-btn" disabled={props.busy} onClick={() => props.onAction('retry')}><RotateCcw size={14} aria-hidden="true" />Retry</button>
        <button type="button" class="sac-btn" ref={pick} disabled={props.busy} aria-haspopup="dialog" onClick={() => setPicking(true)}><Shuffle size={14} aria-hidden="true" />Switch model</button>
        <button type="button" class="sac-btn" data-tone="danger" disabled={props.busy} onClick={() => props.onAction('stop')}><Square size={14} aria-hidden="true" />Stop</button>
      </div>
    </Show>
    <Show when={props.error}><p class="pk-fail" role="alert" data-testid="peek-action-error">{props.error}</p></Show>
    <Show when={picking()}><HostModelDialog hostId={props.hostId} title="Switch model" testId="peek-switch-model" onClose={() => setPicking(false)} trigger={() => pick} value={props.node.model ?? ''} fallback="Lead model" onPick={(key) => { if (key) props.onAction('switch_model', key); }} /></Show>
  </section>;
}

/** The side pane: header, the owner's question or the health card, then the real transcript. */
export function FleetDetail(props: PeekProps) {
  let element: HTMLElement | undefined;
  const visible = paneVisible(() => element?.closest('.pane-tab-body') as HTMLElement | undefined);
  const target = () => openTarget(props.tree, props.node.id);
  const handle = createMemo(() => sessions().ensure(props.hostId, target()));
  createEffect(() => { const release = handle().acquire(); onCleanup(release); });
  const child = () => props.node.role === 'subagent' || props.node.role === 'worker';

  const [messages, setMessages] = createSignal<AgentMessage[]>([]);
  createEffect(() => {
    if (!child() || !visible()) return;
    const s = handle(); const id = props.node.id; const working = props.node.status === 'working';
    let active = true; let inFlight = false; let sequence = 0; let applied = 0;
    setMessages([]);
    const read = async () => {
      if (inFlight) return;
      inFlight = true;
      const current = ++sequence;
      try {
        const live = await fetchAgentMessages(s, id);
        const result = live.length ? live : await fetchRecordTranscript(s, id);
        if (active && current > applied) { applied = current; setMessages(result); }
      } finally { inFlight = false; }
    };
    void read();
    // A finished or idle child does not change: poll only while it works.
    const stop = working ? topicsFor(props.hostId).watch(`agents:${s.sessionId}`,
      () => void read(), () => fallbackPoll(() => void read(), POLL_MS)) : undefined;
    onCleanup(() => { active = false; stop?.(); });
  });

  const ctx = createMemo<TimelineCtx>(() => {
    const s = handle();
    return { session: s, hostId: s.hostId, sessionId: s.sessionId, cwd: () => props.node.cwd ?? undefined, refs: sessionRefChat(s, () => props.node.cwd ?? undefined), commands: () => commandNames(s.state.commands), columnWidth: () => 360, readOnly: true };
  });
  const question = (): NeedsItem | undefined => (props.node.question && props.node.projectId ? { projectId: props.node.projectId, taskId: props.node.question.taskId, status: 'blocked', question: props.node.question.excerpt } : undefined);
  const meta = () => [props.node.project, props.node.model ? modelDisplayName({ id: props.node.model }) : null, props.node.mode !== 'plain' ? MODE_LABEL[props.node.mode] : null, elapsed(props.node, props.sa.now())].filter(Boolean).join(' · ');
  const label = () => (target() === props.node.id ? 'Open chat' : 'Open parent chat');

  return <aside ref={element} class="pk" aria-label="Agent details" data-testid="peek" data-phone={props.phone ? '' : undefined}>
    <header class="pk-head">
      <Ring node={props.node} size={40} icon={18} anim={false} />
      <div class="pk-titles"><h2 title={nodeTitle(props.node)}>{nodeTitle(props.node)}</h2><p>{meta()}</p></div>
      <Show when={!props.phone}><button type="button" class="sac-icon" aria-label="Close details" data-tip="Close (Esc)" onClick={props.onClose}><X size={18} /></button></Show>
      <div class="pk-head-row">
        <span class="cn-pill" data-status={props.node.status}>{STATUS_LABEL[props.node.status]}</span>
        <button type="button" class="sac-btn" onClick={props.onOpen}><ExternalLink size={14} aria-hidden="true" />{label()}</button>
      </div>
    </header>
    <div class="pk-cards">
      <Show when={props.node.status === 'waiting' ? question() : undefined}>{(item) => <div class="pk-question"><Question sa={props.sa} item={item()} onSent={() => void props.sa.refresh()} /></div>}</Show>
      <Show when={props.node.status === 'stalled'}><HealthCard {...props} /></Show>
      <Show when={props.node.status === 'idle' && props.node.role === 'lead'}>
        <section class="pk-card" aria-label="Idle"><p class="pk-sub">{props.node.idleReason === 'unloaded' ? 'Unloaded to save memory. It wakes when asked.' : 'Waiting for work.'}</p>
          <div class="pk-actions"><button type="button" class="sac-btn" disabled={props.busy} onClick={() => props.onAction('wake')}><Play size={14} aria-hidden="true" />Wake</button></div>
          <Show when={props.error}><p class="pk-fail" role="alert">{props.error}</p></Show></section>
      </Show>
    </div>
    <h3 class="pk-h">Transcript</h3>
    <div class="pk-transcript">
      <Show when={child()} fallback={<div class="tl-frame"><Timeline ctx={ctx()} /></div>}>
        <SubagentTimeline session={handle()} agentId={props.node.id} cwd={props.node.cwd ?? undefined} messages={messages()} running={props.node.status === 'working'} startedAt={props.node.startedAt ?? undefined} />
      </Show>
    </div>
  </aside>;
}
