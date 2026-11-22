import { createSignal, Show } from 'solid-js';
import { RotateCcw, Shuffle, Square } from 'lucide-solid';
import { HostModelDialog } from '../../model/HostModelPicker';
import { age } from '../fleet';
import type { Lead } from '../api';
import type { SuperagentState } from '../state';
import { useLeadAction } from './useLeadAction';

/** "2 attempts · since 14m ago · next retry in 31m": only what the host reported. */
export function healthMeta(health: NonNullable<Lead['health']>, now: number): string {
  const parts: string[] = [];
  if (health.attempts) parts.push(`${health.attempts} attempt${health.attempts === 1 ? '' : 's'}`);
  if (health.since) parts.push(`since ${age(now - health.since)} ago`);
  if (health.nextProbeAt) parts.push(health.nextProbeAt > now ? `next retry in ${age(health.nextProbeAt - now)}` : 'retrying now');
  return parts.join(' · ');
}

/**
 * A stalled lead, once: why it stopped, how often, since when, when the host tries again, and the three ways out
 * (Retry, Switch model through the app's one model dialog, Stop after a confirmation). The host's refusal shows under the buttons.
 */
export function HealthCard(props: { hostId: string; sa: SuperagentState; lead: Lead; name: string }) {
  const [picking, setPicking] = createSignal(false);
  let pick: HTMLButtonElement | undefined;
  const action = useLeadAction({ hostId: () => props.hostId, sa: props.sa, sessionId: () => props.lead.sessionId, name: () => props.name });
  const health = () => props.lead.health;
  return <section class="sa-health" role="group" aria-label={`${props.name} is stalled`} data-testid="sa-health">
    <p class="sa-health-err" data-testid="sa-health-error">{health()?.error || 'It stopped with an error.'}</p>
    <Show when={health()}>{(h) => <p class="sa-health-meta" data-testid="sa-health-meta">{healthMeta(h(), props.sa.now())}</p>}</Show>
    <div class="sa-actions">
      <button type="button" class="sa-btn" disabled={action.busy()} onClick={() => void action.run('retry')}><RotateCcw size={14} aria-hidden="true" />Retry</button>
      <button type="button" class="sa-btn" ref={pick} disabled={action.busy()} aria-haspopup="dialog" onClick={() => setPicking(true)}><Shuffle size={14} aria-hidden="true" />Switch model</button>
      <button type="button" class="sa-btn" data-tone="danger" disabled={action.busy()} onClick={() => void action.run('stop')}><Square size={14} aria-hidden="true" />Stop</button>
    </div>
    <Show when={action.error()}><p class="sa-health-fail" role="alert" data-testid="sa-health-action-error">{action.error()}</p></Show>
    <Show when={picking()}>
      <HostModelDialog hostId={props.hostId} title="Switch model" testId="sa-switch-model" onClose={() => setPicking(false)} trigger={() => pick} value={props.lead.model ?? ''} fallback="Lead model"
        onPick={(key) => { if (key) void action.run('switch_model', key); }} />
    </Show>
  </section>;
}
