import { For, Show } from 'solid-js';
import { Dynamic } from 'solid-js/web';
import { Play } from 'lucide-solid';
import { confirmSheet } from '../phone';
import { leadState, MODES, type Lead, type LeadMode, type LeadState } from './api';
import { autonomyLabel, elapsed, modeLabel, money, progressText, shortModel, MODE_LABEL } from './format';
import type { SuperagentState } from './state';
import { ChipSelect, SectionHead } from './parts';
import { STATE_ICON } from './canvas/parts';
import { HealthCard } from './leads/HealthCard';
import { LEAD_STATE_LABEL, LEAD_STATE_RANK, minutesText } from './leads/lead';
import { useLeadAction } from './leads/useLeadAction';

function Progress(props: { lead: Lead }) {
  const p = () => props.lead.progress;
  const total = () => (p() ? p()!.running + p()!.queued + p()!.done : 0);
  const pct = (n: number) => (total() ? `${(n / total()) * 100}%` : '0%');
  return <Show when={p()}>
    <div class="sa-progress" role="img" aria-label={progressText(props.lead) ?? ''}>
      <span data-k="done" style={{ width: pct(p()!.done) }} /><span data-k="running" style={{ width: pct(p()!.running) }} /><span data-k="queued" style={{ width: pct(p()!.queued) }} />
    </div>
  </Show>;
}

/** The state as the canvas draws it: green dot, amber Hand, red TriangleAlert, Moon for idle and unloaded. */
function StatePill(props: { state: LeadState }) {
  return <span class="sa-state" data-state={props.state} data-testid="sa-lead-state">
    {props.state === 'working' ? <span class="sa-live-dot" aria-hidden="true" /> : <Dynamic component={STATE_ICON[props.state === 'unloaded' ? 'idle' : props.state]} size={12} aria-hidden="true" />}
    {LEAD_STATE_LABEL[props.state]}
  </span>;
}

function LeadCard(props: { hostId: string; sa: SuperagentState; lead: Lead; onOpen: (id: string) => void }) {
  const lead = () => props.lead;
  const name = () => props.sa.projectName(lead().projectId);
  const state = () => leadState(lead());
  const quiet = () => state() === 'idle' || state() === 'unloaded';
  const wake = useLeadAction({ hostId: () => props.hostId, sa: props.sa, sessionId: () => lead().sessionId, name });
  const lastActive = () => lead().lastActiveAt ? `Last active ${minutesText(props.sa.now() - lead().lastActiveAt!)} ago` : null;
  const modeKey = () => `mode:${lead().projectId}`;
  const options = () => {
    const base = MODES.map((m) => ({ value: m as string, label: MODE_LABEL[m] }));
    const mode = lead().mode;
    return mode && !(MODES as readonly string[]).includes(mode) ? [...base, { value: mode, label: mode }] : base;
  };
  async function pickMode(value: string) {
    if (value === lead().mode) return;
    // Switching is a soft gate: the lead keeps working, so ask first.
    const ok = await confirmSheet({ title: `Switch ${name()} to ${modeLabel(value)}?`, body: 'The running lead changes mode at its next step boundary. Work already started continues.', confirm: `Switch to ${modeLabel(value)}` });
    if (ok) await props.sa.setMode(lead().projectId, value as LeadMode);
  }
  async function stop() {
    if (!(await confirmSheet({ title: `Stop the ${name()} lead?`, body: 'The lead stops and its claimed tickets are released. Open a ticket to review progress before retrying.', confirm: 'Stop lead', tone: 'danger' }))) return;
    await props.sa.stopLead(lead());
  }
  const meta = () => [props.sa.presetLabel(lead().presetId), shortModel(lead().model), lead().thinking ? `${lead().thinking} thinking` : null].filter(Boolean).join(' · ');
  const stats = () => [elapsed(lead().startedAt, props.sa.now()), money(lead().cost)].filter(Boolean).join(' · ');
  const tickets = () => lead().ticketIds.length;
  return <article class="sa-lead" data-testid="sa-lead" data-state={state()}>
    <div class="sa-lead-top">
      <strong class="sa-lead-name" title={name()}>{name()}</strong>
      <Show when={lead().pausedGraph}>{(pg) =>
        <span class="sa-badge sa-paused-badge" title={pg().title ?? undefined}>Graph paused</span>
      }</Show>
      <Show when={lead().mode && !lead().legacy && state() !== 'stalled' && !quiet()}>
        <ChipSelect class="sa-mode" label={`Mode for ${name()}`} value={lead().mode!} options={options()} disabled={props.sa.isBusy(modeKey())}
          onChange={(v) => void pickMode(v)} />
      </Show>
    </div>
    <p class="sa-lead-state"><StatePill state={state()} /><Show when={quiet() && lastActive()}><span class="sa-lead-since">{lastActive()}</span></Show></p>
    <Show when={lead().pausedGraph?.title}>{(title) =>
      <p class="sa-lead-meta sa-paused-title">Paused graph: {title()}</p>
    }</Show>
    <p class="sa-lead-meta"><Show when={meta()}>{meta()} · </Show><span class="sa-autonomy">{autonomyLabel(lead().autonomy)}</span></p>
    <Show when={state() === 'waiting' && lead().question?.excerpt}>{(excerpt) => <p class="sa-lead-ask" title={excerpt()} data-testid="sa-lead-ask">{excerpt()}</p>}</Show>
    <Show when={state() === 'working' || state() === 'waiting'}>
      <div class="sa-lead-step">
        <span class="sa-step"><span class="sa-step-text">{lead().step ?? 'Step not reported'}</span><Show when={lead().tool}><code>{lead().tool}</code></Show></span>
        <Show when={stats()}><span class="sa-stats">{stats()}</span></Show>
      </div>
      <Progress lead={lead()} />
    </Show>
    <Show when={state() === 'stalled'}><HealthCard hostId={props.hostId} sa={props.sa} lead={lead()} name={name()} /></Show>
    <Show when={tickets() > 0 || progressText(lead())}><p class="sa-lead-count"><Show when={progressText(lead())}>{progressText(lead())} · </Show>{tickets()} {tickets() === 1 ? 'ticket' : 'tickets'}</p></Show>
    <Show when={wake.error() && quiet()}><p class="sa-health-fail" role="alert" data-testid="sa-wake-error">{wake.error()}</p></Show>
    <div class="sa-actions">
      <Show when={lead().pausedGraph}>{(pg) =>
        <button type="button" class="sa-btn" data-tone="primary" disabled={props.sa.isBusy(`resume:${pg().sessionId}`)} onClick={() => void props.sa.resumePausedGraph(pg().sessionId)}>Resume</button>
      }</Show>
      <Show when={quiet()}><button type="button" class="sa-btn" disabled={wake.busy()} onClick={() => void wake.run('wake')}><Play size={14} aria-hidden="true" />Wake</button></Show>
      <button type="button" class="sa-btn" onClick={() => props.onOpen(lead().sessionId)}>Open</button>
      <Show when={state() === 'working' || state() === 'waiting'}><button type="button" class="sa-btn" data-tone="danger" disabled={props.sa.isBusy(`stop:${lead().projectId}`)} onClick={() => void stop()}>Stop</button></Show>
    </div>
  </article>;
}

export function LeadsSection(props: { hostId: string; sa: SuperagentState; onOpen: (id: string) => void }) {
  const rows = () => props.sa.running();
  const rank = (lead: Lead) => LEAD_STATE_RANK[leadState(lead)];
  const leads = () => rows().flatMap((row) => row.lead ? [row.lead] : []).sort((a, b) => rank(a) - rank(b));
  return <section class="sa-sec" aria-label="Leads" data-testid="sa-leads">
    <SectionHead title="Leads" count={rows().length} />
    <Show when={rows().length} fallback={<p class="sa-empty">No lead is running. Start one from a project, or ask Superagent.</p>}>
      <For each={leads()}>{(lead) => <LeadCard hostId={props.hostId} sa={props.sa} lead={lead} onOpen={props.onOpen} />}</For>
      <For each={rows().filter((row) => !row.lead)}>{(row) => <article class="sa-lead" data-testid="sa-manual-run">
        <div class="sa-lead-top"><span class="sa-live-dot" /><strong class="sa-lead-name">{row.title}</strong></div>
        <p class="sa-lead-meta">{row.project ?? 'No project'} · {elapsed(row.startedAt == null ? null : new Date(row.startedAt).toISOString(), props.sa.now()) ?? 'Elapsed unavailable'}</p>
        <div class="sa-actions"><button type="button" class="sa-btn" onClick={() => props.onOpen(row.sessionId)}>Open</button><button type="button" class="sa-btn" data-tone="danger" onClick={() => void props.sa.stopSession(row)}>Stop</button></div>
      </article>}</For>
    </Show>
  </section>;
}
