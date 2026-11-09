import { For, Show } from 'solid-js';
import { confirmSheet } from '../phone';
import { MODES, type Lead, type LeadMode } from './api';
import { autonomyLabel, elapsed, modeLabel, money, progressText, shortModel, MODE_LABEL } from './format';
import type { SuperagentState } from './state';
import { ChipSelect, SectionHead } from './parts';

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

function LeadCard(props: { sa: SuperagentState; lead: Lead; onOpen: (id: string) => void }) {
  const lead = () => props.lead;
  const name = () => props.sa.projectName(lead().projectId);
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
  return <article class="sa-lead" data-testid="sa-lead">
    <div class="sa-lead-top">
      <span class="sa-live-dot" aria-hidden="true" />
      <strong class="sa-lead-name" title={name()}>{name()}</strong>
      <Show when={lead().mode && !lead().legacy}>
        <ChipSelect class="sa-mode" label={`Mode for ${name()}`} value={lead().mode!} options={options()} disabled={props.sa.isBusy(modeKey())}
          onChange={(v) => void pickMode(v)} />
      </Show>
    </div>
    <p class="sa-lead-meta"><Show when={meta()}>{meta()} · </Show><span class="sa-autonomy">{autonomyLabel(lead().autonomy)}</span></p>
    <div class="sa-lead-step">
      <span class="sa-step"><span class="sa-step-text">{lead().step ?? 'Step not reported'}</span><Show when={lead().tool}><code>{lead().tool}</code></Show></span>
      <Show when={stats()}><span class="sa-stats">{stats()}</span></Show>
    </div>
    <Progress lead={lead()} />
    <p class="sa-lead-count"><Show when={progressText(lead())}>{progressText(lead())} · </Show>{tickets()} {tickets() === 1 ? 'ticket' : 'tickets'}</p>
    <div class="sa-actions">
      <button type="button" class="sa-btn" onClick={() => props.onOpen(lead().sessionId)}>Open</button>
      <button type="button" class="sa-btn" data-tone="danger" disabled={props.sa.isBusy(`stop:${lead().projectId}`)} onClick={() => void stop()}>Stop</button>
    </div>
  </article>;
}

export function LeadsSection(props: { sa: SuperagentState; onOpen: (id: string) => void }) {
  const rows = () => props.sa.running();
  const leads = () => rows().flatMap((row) => row.lead ? [row.lead] : []);
  return <section class="sa-sec" aria-label="Leads" data-testid="sa-leads">
    <SectionHead title="Leads" count={rows().length} />
    <Show when={rows().length} fallback={<p class="sa-empty">No lead is running. Start one from a project, or ask Superagent.</p>}>
      <For each={leads()}>{(lead) => <LeadCard sa={props.sa} lead={lead} onOpen={props.onOpen} />}</For>
      <For each={rows().filter((row) => !row.lead)}>{(row) => <article class="sa-lead" data-testid="sa-manual-run">
        <div class="sa-lead-top"><span class="sa-live-dot" /><strong class="sa-lead-name">{row.title}</strong></div>
        <p class="sa-lead-meta">{row.project ?? 'No project'} · {elapsed(row.startedAt == null ? null : new Date(row.startedAt).toISOString(), props.sa.now()) ?? 'Elapsed unavailable'}</p>
        <div class="sa-actions"><button type="button" class="sa-btn" onClick={() => props.onOpen(row.sessionId)}>Open</button><button type="button" class="sa-btn" data-tone="danger" onClick={() => void props.sa.stopSession(row)}>Stop</button></div>
      </article>}</For>
    </Show>
  </section>;
}
