import { createSignal, Show } from 'solid-js';
import { Minus, Plus, RefreshCw, SlidersHorizontal } from 'lucide-solid';
import { AnchoredMenu } from '../board/AnchoredMenu';
import { Segmented } from '../phone';
import { layout } from '../../lib/layout';
import { CAP_MAX, CAP_MIN } from './api';
import { countNeeds } from './needs';
import type { SuperagentState } from './state';
import { Toggle } from './parts';
import { NeedsYouSection } from './NeedsYou';
import { LeadsSection } from './Leads';
import { ProjectsSection } from './Projects';
import { PresetManager } from './Presets';
import type { QueueKind } from '../../lib/board/taskOrigin';

export type PaneTab = 'needs' | 'leads' | 'projects';

/** Number of things waiting on the owner: drives the phone badge. */
export function needsCount(sa: SuperagentState): number {
  return countNeeds(sa.status()?.needsYou);
}

function Settings(props: { sa: SuperagentState; onManage: () => void }) {
  const s = () => props.sa.status()!;
  const cap = () => s().concurrencyCap;
  const step = (d: number) => void props.sa.configure(s().enabled, Math.min(CAP_MAX, Math.max(CAP_MIN, cap() + d)));
  return <div class="sa-settings" data-testid="sa-settings">
    <div class="sa-set-row"><div><strong>Auto-start</strong><p>Master switch. Each project also needs its own.</p></div>
      <Toggle label="Auto-start" checked={s().enabled} disabled={props.sa.isBusy('configure')} onChange={(v) => void props.sa.configure(v, cap())} /></div>
    <div class="sa-set-row"><strong>Max leads</strong>
      <span class="sa-stepper" role="group" aria-label="Max leads">
        <button type="button" class="sa-icon-btn" aria-label="Decrease max leads" disabled={cap() <= CAP_MIN || props.sa.isBusy('configure')} onClick={() => step(-1)}><Minus size={16} /></button>
        <output aria-live="polite">{cap()}</output>
        <button type="button" class="sa-icon-btn" aria-label="Increase max leads" disabled={cap() >= CAP_MAX || props.sa.isBusy('configure')} onClick={() => step(1)}><Plus size={16} /></button>
      </span></div>
    <Show when={props.sa.presets().supported}><button type="button" class="sa-btn" data-block onClick={props.onManage}>Manage presets</button></Show>
  </div>;
}

export function SuperagentPane(props: { hostId: string; sa: SuperagentState; tab: PaneTab; onTab: (tab: PaneTab) => void; onOpen: (id: string) => void; onTicket: (project: string, task: string, queue: QueueKind) => void }) {
  const sa = props.sa;
  const [menu, setMenu] = createSignal<DOMRect>();
  const [managing, setManaging] = createSignal(false);
  const phone = () => !layout().desktop();
  const show = (tab: PaneTab) => !phone() || props.tab === tab;
  const summary = () => { const s = sa.status(); return s ? (s.enabled ? `Auto-start on · up to ${s.concurrencyCap} ${s.concurrencyCap === 1 ? 'lead' : 'leads'}` : 'Auto-start off') : 'Superagent'; };
  const tabs = () => [
    { value: 'needs' as const, label: `Needs you · ${needsCount(sa)}` },
    { value: 'leads' as const, label: `Leads · ${sa.running().length}` },
    { value: 'projects' as const, label: 'Projects' },
  ];
  return <div class="sa-pane" aria-label="Needs you, leads and projects">
    <div class="sa-pane-top">
    <div class="sa-pane-heading">
      <button type="button" class="sa-summary" aria-label="Auto-start settings" aria-haspopup="menu" disabled={!sa.status()} onClick={(e) => setMenu(e.currentTarget.getBoundingClientRect())}><SlidersHorizontal size={15} /><span>{summary()}</span></button>
      <button type="button" class="sa-icon-btn" aria-label="Refresh" title="Refresh" onClick={() => void sa.refresh()}><RefreshCw size={16} /></button>
    </div>
    <Show when={phone()}><div class="sa-tabs"><Segmented label="Superagent sections" value={props.tab} options={tabs()} onChange={props.onTab} /></div></Show>
    </div>
    <Show when={sa.error()}><div class="sa-message" role="alert">{sa.error()} <button type="button" onClick={() => { sa.clearError(); void sa.refresh(); }}>Retry</button></div></Show>
    <Show when={sa.notice()}><p class="sa-message" role="status">{sa.notice()}</p></Show>
    <Show when={sa.status()} fallback={<Show when={!sa.error()}><div class="sa-loading" aria-label="Loading Superagent" aria-busy="true"><span /><span /><span /></div></Show>}>
      <Show when={show('needs')}><NeedsYouSection hostId={props.hostId} sa={sa} onOpen={props.onOpen} onTicket={(project, task) => props.onTicket(project, task, 'needs')} /></Show>
      <Show when={show('leads')}><LeadsSection sa={sa} onOpen={props.onOpen} /></Show>
      <Show when={show('projects')}><ProjectsSection hostId={props.hostId} sa={sa} onManage={() => setManaging(true)} /></Show>
    </Show>
    <Show when={menu()}>{(rect) => <AnchoredMenu anchor={rect()} onDismiss={() => setMenu(undefined)} label="Auto-start settings" title="Settings" width={300}><Settings sa={sa} onManage={() => { setMenu(undefined); setManaging(true); }} /></AnchoredMenu>}</Show>
    <PresetManager sa={sa} open={managing()} onClose={() => setManaging(false)} />
  </div>;
}
