import { createEffect, createResource, createSignal, For, Show } from 'solid-js';
import { Play } from 'lucide-solid';
import { SheetFrame } from '../phone/Sheet';
import { api as boardApi, PRIORITY_RANK, type Task } from '../../lib/board/api';
import { displayTitle } from '../../lib/board/api';
import { isHardBlocker, startRefusal, type Blocker, type ProjectRow } from './api';
import { prettyModelId } from '../../lib/model/displayName';
import type { SuperagentState } from './state';
import { Limited, SectionHead, Toggle } from './parts';
import { PresetSelect } from './Presets';
import { DispatchBlockedModal, type BlockedRequest } from './DispatchBlockedModal';

const isReady = (t: Task) => t.status === 'todo' && t.ready !== false;
/** The reason a project cannot take work; an unregistered row always has one, even from a host that sent none. */
const firstBlocker = (p: ProjectRow): Blocker | undefined => p.blockers?.[0] ?? (p.registered !== false ? undefined : { code: 'not_registered', message: 'This workspace has no Kanboard board yet.', action: { kind: 'register', label: 'Set up Kanboard' } });
/** The short row hint for a blocker; the full message sits in the title. */
function hintText(p: ProjectRow, b: Blocker): string {
  const wait = p.ready > 0 ? ` · ${p.ready} waiting` : '';
  switch (b.code) {
    case 'not_registered': return 'No Kanboard board';
    case 'archived': return 'Archived';
    case 'project_auto_off': return `Auto off${wait}`;
    case 'global_auto_off': return `Auto-start paused${wait}`;
  }
}
/** Refusals the host raised without a blocker on the row (older status, race): same modal, built from the error. */
function refusalBlocker(kind: 'not_registered' | 'archived', message: string): Blocker {
  return kind === 'archived'
    ? { code: 'archived', message, action: { kind: 'unarchive', label: 'Unarchive' } }
    : { code: 'not_registered', message, action: { kind: 'register', label: 'Set up Kanboard' } };
}
const byUrgency = (a: Task, b: Task) => (PRIORITY_RANK[a.priority] ?? 9) - (PRIORITY_RANK[b.priority] ?? 9) || a.order - b.order;

/** Pick ready tickets and a preset, then start (or feed) the project's lead. */
function StartSheet(props: { hostId: string; sa: SuperagentState; project: ProjectRow; onClose: () => void; onManage: () => void }) {
  const name = () => props.sa.projectName(props.project.projectId);
  const [tickets] = createResource(() => props.project.projectId, async (id) => {
    const result = await boardApi.tasks(props.hostId, id);
    return result.tasks.filter(isReady).sort(byUrgency);
  });
  const [picked, setPicked] = createSignal<string[]>([]);
  const [preset, setPreset] = createSignal<string | null>(null);
  createEffect(() => { const t = tickets(); if (t) setPicked(t.map((x) => x.id)); });
  createEffect(() => { if (preset() === null) setPreset(props.sa.presets().defaultId); });
  const toggle = (id: string) => setPicked((old) => (old.includes(id) ? old.filter((x) => x !== id) : [...old, id]));
  const busy = () => props.sa.isBusy(`start:${props.project.projectId}`);
  const [asking, setAsking] = createSignal<BlockedRequest>();
  const ask = (blocker: Blocker) => setAsking({
    projectId: props.project.projectId, name: name(), path: props.project.path, blocker, startCount: picked().length,
    // Same tickets and preset; `register` hands back the new project id.
    retry: async (id) => {
      let message = '';
      const ok = await props.sa.start(id, picked(), preset(), (e) => { message = e.message; return true; });
      if (ok) props.onClose();
      return ok ? null : message || 'Could not start. Retry.';
    },
  });
  const hard = () => { const b = firstBlocker(props.sa.status()?.projects?.find((x) => x.projectId === props.project.projectId) ?? props.project); return isHardBlocker(b) ? b : undefined; };
  async function start() {
    const blocked = hard();
    if (blocked) return ask(blocked);
    const handled = (e: { message: string; code?: unknown; detail?: unknown }) => {
      const kind = startRefusal(e);
      if (kind) ask(refusalBlocker(kind, e.message));
      return !!kind;
    };
    if (await props.sa.start(props.project.projectId, picked(), preset(), handled)) props.onClose();
  }
  return <><SheetFrame open onClose={props.onClose} label={`Start ${name()}`} title={`Start ${name()}`} level={2} detents={['content']}
    footer={<button type="button" class="sa-btn" data-tone="primary" data-block disabled={!picked().length || busy()} onClick={() => void start()}>{picked().length === 1 ? 'Start 1 ticket' : `Start ${picked().length} tickets`}</button>}>
    <div class="sa-start" data-testid="sa-start">
      <Show when={hard()}>{(b) => <p class="sa-message sa-start-blocked" role="status">{b().message}</p>}</Show>
      <div class="sa-start-preset"><span>Preset</span><PresetSelect sa={props.sa} value={preset()} onChange={setPreset} /><button type="button" class="sa-link-quiet" onClick={props.onManage}>Manage</button></div>
      <Show when={!tickets.loading} fallback={<div class="sa-loading" aria-busy="true" aria-label="Loading tickets"><span /><span /></div>}>
        <Show when={tickets.error}><p class="sa-message" role="alert">Could not load tickets. {String((tickets.error as Error)?.message ?? '')}</p></Show>
        <Show when={tickets()?.length} fallback={<Show when={!tickets.error}><p class="sa-empty">No ready tickets. Move tickets to To do on the board, or ask Superagent.</p></Show>}>
          <ul class="sa-ready">
            <For each={tickets()}>{(t) => <li><label class="sa-check"><input type="checkbox" checked={picked().includes(t.id)} onChange={() => toggle(t.id)} /><span class="sa-check-id">{t.id}</span><span class="sa-check-title">{displayTitle(t)}</span></label></li>}</For>
          </ul>
        </Show>
      </Show>
    </div>
  </SheetFrame>
  <Show when={asking()} keyed>{(request) => <DispatchBlockedModal sa={props.sa} request={request} onClose={() => setAsking(undefined)} />}</Show></>;
}

export function ProjectsSection(props: { hostId: string; sa: SuperagentState; onManage: () => void }) {
  const [starting, setStarting] = createSignal<ProjectRow>();
  const [asking, setAsking] = createSignal<BlockedRequest>();
  /** Set up / Enable: change the setting, then refresh. No start (an unregistered workspace has no tickets; Auto starts ready ones itself). */
  const askFor = (p: ProjectRow, blocker: Blocker) => setAsking({ projectId: p.projectId, name: props.sa.projectName(p.projectId), path: p.path, blocker, ready: p.ready });
  const projects = () => props.sa.status()?.projects;
  const paused = (p: ProjectRow) => p.autoStart && props.sa.status()?.enabled === false;
  return <section class="sa-sec" aria-label="Projects" data-testid="sa-projects">
    <SectionHead title="Projects" count={projects()?.length} />
    <Show when={projects()} fallback={<p class="sa-empty">This host has no per-project controls. Update it to use auto-start.</p>}>
      {(list) => <Show when={list().length} fallback={<p class="sa-empty">No projects on this computer.</p>}>
        <ul class="sa-projects">
          <Limited items={list()} limit={4}>{(p) => <li class="sa-project-row">
            <div class="sa-project-main"><strong title={p.registered === false ? (p.path ?? props.sa.projectName(p.projectId)) : props.sa.projectName(p.projectId)}>{props.sa.projectName(p.projectId)}</strong>
              <span class="sa-sub">{p.lead ? <><span class="sa-live-dot" aria-hidden="true" /> Lead running</> : <Show when={firstBlocker(p)} fallback={<>{paused(p) ? 'Auto-start paused' : 'No lead'}{p.ready > 0 ? ` · ${p.ready} ready` : ''}</>}>{(b) => <span class="sa-hint" title={b().message}>
                <span class="sa-hint-text" aria-hidden="true">{hintText(p, b())}</span><span class="sr-only">{b().message}</span>
                <Show when={p.ready > 0 && !isHardBlocker(b())}><button type="button" class="sa-hint-btn" aria-label={`${b().action.label} for ${props.sa.projectName(p.projectId)}`} onClick={() => askFor(p, b())}>Enable</button></Show>
              </span>}</Show>}{p.leadModel ? <span title={`Lead model ${p.leadModel}${p.leadThinking ? ` · ${p.leadThinking}` : ''}`}> · {prettyModelId(p.leadModel)}</span> : null}</span></div>
            <Show when={p.registered !== false} fallback={<button type="button" class="sa-btn" aria-label={`Set up ${props.sa.projectName(p.projectId)}`} onClick={() => askFor(p, firstBlocker(p)!)}>Set up</button>}>
              <span class="sa-auto"><span aria-hidden="true">Auto</span><Toggle label={`Auto-start ${props.sa.projectName(p.projectId)}`} checked={p.autoStart} disabled={props.sa.isBusy(`auto:${p.projectId}`)} onChange={(v) => void props.sa.setAutoStart(p.projectId, v)} /></span>
              <button type="button" class="sa-btn" aria-label={`Start ${props.sa.projectName(p.projectId)}`} onClick={() => setStarting(p)}><Play size={14} /> Start</button>
            </Show>
          </li>}</Limited>
        </ul>
      </Show>}
    </Show>
    <Show when={starting()} keyed>{(p) => <StartSheet hostId={props.hostId} sa={props.sa} project={p} onClose={() => setStarting(undefined)} onManage={props.onManage} />}</Show>
    <Show when={asking()} keyed>{(request) => <DispatchBlockedModal sa={props.sa} request={request} onClose={() => setAsking(undefined)} />}</Show>
  </section>;
}
