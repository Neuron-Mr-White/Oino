import { createEffect, createResource, createSignal, For, Show } from 'solid-js';
import { Play } from 'lucide-solid';
import { SheetFrame } from '../phone/Sheet';
import { api as boardApi, PRIORITY_RANK, type Task } from '../../lib/board/api';
import { displayTitle } from '../../lib/board/api';
import type { ProjectRow } from './api';
import type { SuperagentState } from './state';
import { Limited, SectionHead, Toggle } from './parts';
import { PresetSelect } from './Presets';

const isReady = (t: Task) => t.status === 'todo' && t.ready !== false;
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
  async function start() {
    if (await props.sa.start(props.project.projectId, picked(), preset())) props.onClose();
  }
  return <SheetFrame open onClose={props.onClose} label={`Start ${name()}`} title={`Start ${name()}`} level={2} detents={['content']}
    footer={<button type="button" class="sa-btn" data-tone="primary" data-block disabled={!picked().length || busy()} onClick={() => void start()}>{picked().length === 1 ? 'Start 1 ticket' : `Start ${picked().length} tickets`}</button>}>
    <div class="sa-start" data-testid="sa-start">
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
  </SheetFrame>;
}

export function ProjectsSection(props: { hostId: string; sa: SuperagentState; onManage: () => void }) {
  const [starting, setStarting] = createSignal<ProjectRow>();
  const projects = () => props.sa.status()?.projects;
  const paused = (p: ProjectRow) => p.autoStart && props.sa.status()?.enabled === false;
  return <section class="sa-sec" aria-label="Projects" data-testid="sa-projects">
    <SectionHead title="Projects" count={projects()?.length} />
    <Show when={projects()} fallback={<p class="sa-empty">This host has no per-project controls. Update it to use auto-start.</p>}>
      {(list) => <Show when={list().length} fallback={<p class="sa-empty">No projects on this computer.</p>}>
        <ul class="sa-projects">
          <Limited items={list()} limit={4}>{(p) => <li class="sa-project-row">
            <div class="sa-project-main"><strong title={props.sa.projectName(p.projectId)}>{props.sa.projectName(p.projectId)}</strong>
              <span class="sa-sub">{p.lead ? <><span class="sa-live-dot" aria-hidden="true" /> Lead running</> : <>{paused(p) ? 'Auto-start paused' : 'No lead'}{p.ready > 0 ? ` · ${p.ready} ready` : ''}</>}</span></div>
            <span class="sa-auto"><span aria-hidden="true">Auto</span><Toggle label={`Auto-start ${props.sa.projectName(p.projectId)}`} checked={p.autoStart} disabled={props.sa.isBusy(`auto:${p.projectId}`)} onChange={(v) => void props.sa.setAutoStart(p.projectId, v)} /></span>
            <button type="button" class="sa-btn" aria-label={`Start ${props.sa.projectName(p.projectId)}`} onClick={() => setStarting(p)}><Play size={14} /> Start</button>
          </li>}</Limited>
        </ul>
      </Show>}
    </Show>
    <Show when={starting()} keyed>{(p) => <StartSheet hostId={props.hostId} sa={props.sa} project={p} onClose={() => setStarting(undefined)} onManage={props.onManage} />}</Show>
  </section>;
}
