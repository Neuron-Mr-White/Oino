/**
 * PC + project pickers (UNI-130) as bottom sheets. The PC sheet lists
 * every paired PC with its board availability (offline / not installed
 * greyed with the reason; tap re-checks). The project sheet is the web
 * sidebar on a phone: active projects with open counts, review/blocked
 * hints and the running-agent ring, a collapsible "Archived projects"
 * section, and the PC-wide "Agents running" list.
 */
import { createEffect, createSignal, For, Show } from 'solid-js';
import { Archive, ArchiveRestore, Check, ChevronDown, ChevronRight, Maximize2, Monitor, RefreshCw } from 'lucide-solid';
import { displayTitle, OPEN_LANES, type ProjectSummary, type RunningItem } from '../../lib/board/api';
import { availabilityText, pcAvailability, type Pc } from '../../lib/board/pcs';
import { boardActions } from '../../lib/board/actions';
import { boardStore } from '../../lib/board/store';
import { boardUi } from '../../lib/board/ui';
import { elapsed, hue } from '../../lib/board/view';
import { AnchoredMenu, MenuRow, MenuSeparator } from './AnchoredMenu';
import { BoardSheet } from './BoardSheet';
import { ProjectTile } from './Glyphs';
import { now } from './TaskCard';

export function PcSheet(props: { open: boolean; onClose: () => void; pcs: Pc[]; current?: string; onPick: (id: string) => void }) {
  const avail = pcAvailability();
  const recheckAll = () => props.pcs.forEach((pc) => void avail.check(pc.id));
  // Every PC's state is current while the sheet is open.
  createEffect(() => {
    if (props.open) recheckAll();
  });
  return (
    <BoardSheet
      open={props.open}
      onClose={props.onClose}
      label="Choose a PC"
      testId="pc-sheet"
      headerAction={
        <button type="button" class="ghost-icon-button" aria-label="Check again" onClick={recheckAll}>
          <RefreshCw size={18} />
        </button>
      }
    >
      <div class="settings-group rounded-card bg-surface-2/60">
        <For each={props.pcs}>
          {(pc) => {
            const state = () => avail.availability(pc.id);
            const ok = () => state()?.state === 'ok';
            return (
              <button
                type="button"
                class="picker-row"
                classList={{ dim: !ok() && state()?.state !== 'checking' && state() !== undefined }}
                aria-current={props.current === pc.id ? 'true' : undefined}
                data-state={state()?.state ?? 'checking'}
                onClick={async () => {
                  if (ok()) return props.onPick(pc.id);
                  const result = await avail.check(pc.id);
                  if (result.state === 'ok') props.onPick(pc.id);
                }}
              >
                <span class="settings-icon">
                  <Monitor size={18} />
                </span>
                <span class="min-w-0 flex-1 text-left">
                  <span class="block truncate text-[16px] font-medium">{pc.name}</span>
                  <span class="picker-row-sub" data-state={state()?.state ?? 'checking'}>
                    <span class="pc-dot" data-state={state()?.state ?? 'checking'} />
                    <span class="truncate">{availabilityText(state())}</span>
                  </span>
                </span>
                <Show when={props.current === pc.id}>
                  <Check size={18} class="board-accent shrink-0" />
                </Show>
              </button>
            );
          }}
        </For>
      </div>
    </BoardSheet>
  );
}

export function openCount(project: ProjectSummary): number {
  return OPEN_LANES.reduce((sum, status) => sum + (project.counts?.[status] ?? 0), 0);
}

function projectHint(project: ProjectSummary): string {
  const parts: string[] = [];
  const review = project.counts?.in_review ?? 0;
  const blocked = project.counts?.blocked ?? 0;
  if ((project.running ?? 0) > 0) parts.push(`${project.running} running`);
  if (review > 0) parts.push(`${review} in review`);
  if (blocked > 0) parts.push(`${blocked} blocked`);
  if (parts.length === 0) parts.push(project.root ? project.root.replace(/^\/home\/[^/]+/, '~') : project.slug);
  return parts.join(' · ');
}

export function ProjectSheet(props: {
  open: boolean;
  onClose: () => void;
  current?: string | null;
  onPick: (slug: string) => void;
  onAgent: (item: RunningItem) => void;
}) {
  const store = boardStore();
  const [showArchived, setShowArchived] = createSignal(false);
  const active = () => store.state.projects.filter((project) => !project.archived);
  const archived = () => store.state.projects.filter((project) => project.archived);
  return (
    <BoardSheet open={props.open} onClose={props.onClose} label="Projects" testId="project-sheet" class="board-sheet-tall">
      <Show when={store.state.projectsError}>
        {(error) => <p class="text-sm text-wm-red mb-3">{error().message}</p>}
      </Show>
      <Show when={store.state.projectsLoaded && store.state.projects.length === 0 && !store.state.projectsError}>
        <p class="text-sm text-text-muted mb-3">No kanboard projects on this PC yet. Open the board from a pi session in a repo to register one.</p>
      </Show>
      <div class="settings-group rounded-card bg-surface-2/60">
        <For each={active()}>{(project) => <ProjectRow project={project} current={props.current} onPick={props.onPick} />}</For>
      </div>
      <Show when={archived().length > 0}>
        <button type="button" class="board-section-toggle" aria-expanded={showArchived()} onClick={() => setShowArchived(!showArchived())}>
          <Show when={showArchived()} fallback={<ChevronRight size={14} />}>
            <ChevronDown size={14} />
          </Show>
          Archived projects
          <span class="ml-auto tabular-nums">{archived().length}</span>
        </button>
        <Show when={showArchived()}>
          <div class="settings-group rounded-card bg-surface-2/60 archived-projects">
            <For each={archived()}>{(project) => <ProjectRow project={project} current={props.current} onPick={props.onPick} />}</For>
          </div>
        </Show>
      </Show>
      <h3 class="board-section-label">
        Agents running
        <Show when={store.state.running.length > 0}>
          <span class="ml-auto tabular-nums">{store.state.running.length}</span>
        </Show>
      </h3>
      <Show when={store.state.running.length > 0} fallback={<p class="text-sm text-text-dim">No agents running.</p>}>
        <div class="settings-group rounded-card bg-surface-2/60">
          <For each={store.state.running}>
            {(item) => (
              <button type="button" class="picker-row" onClick={() => props.onAgent(item)}>
                <span class="agent-dot">
                  <span class="pulse" />
                </span>
                <span class="min-w-0 flex-1 text-left">
                  <span class="block truncate text-[15px]">
                    <span class="bcard-id mr-1.5">{item.task.id}</span>
                    {displayTitle(item.task)}
                  </span>
                  <span class="picker-row-sub">
                    <span class="truncate">
                      {item.project} · session {item.task.run?.session ?? '?'}
                    </span>
                  </span>
                </span>
                <span class="text-[13px] tabular-nums text-amber shrink-0">{elapsed(item.task.run?.started, now())}</span>
              </button>
            )}
          </For>
        </div>
      </Show>
    </BoardSheet>
  );
}

function ProjectRow(props: { project: ProjectSummary; current?: string | null; onPick: (slug: string) => void }) {
  return (
    <button
      type="button"
      class="picker-row"
      aria-current={props.current === props.project.slug ? 'true' : undefined}
      onClick={() => props.onPick(props.project.slug)}
      onContextMenu={(event) => {
        event.preventDefault();
        boardUi().setProjectMenu({ slug: props.project.slug, anchor: (event.currentTarget as HTMLElement).getBoundingClientRect() });
      }}
    >
      <ProjectTile name={props.project.name} hue={hue(props.project.name)} size={30} running={(props.project.running ?? 0) > 0} />
      <span class="min-w-0 flex-1 text-left">
        <span class="block truncate text-[16px] font-medium">{props.project.name}</span>
        <span class="picker-row-sub">
          <span class="truncate">{projectHint(props.project)}</span>
        </span>
      </span>
      <span class="lane-pill-count shrink-0" title="Open tasks">
        {openCount(props.project)}
      </span>
      <Show when={props.current === props.project.slug}>
        <Check size={18} class="board-accent shrink-0" />
      </Show>
    </button>
  );
}

/** Project list long-press (UNI-134): Open / Archive-Unarchive, anchored on the pressed row. */
export function ProjectMenu(props: { onPick: (slug: string) => void }) {
  const ui = boardUi();
  const store = boardStore();
  const entry = () => ui.projectMenu();
  const project = () => store.state.projects.find((candidate) => candidate.slug === entry()?.slug);
  const close = () => ui.setProjectMenu(null);
  return (
    <Show when={entry() ? project() : undefined} keyed>
      {(current) => (
        <AnchoredMenu anchor={entry()!.anchor} width={232} label={`${current.name} actions`} onDismiss={close}>
          <MenuRow
            icon={<Maximize2 size={18} />}
            label="Open"
            onSelect={() => {
              close();
              props.onPick(current.slug);
            }}
          />
          <MenuSeparator />
          <Show
            when={current.archived}
            fallback={
              <MenuRow
                icon={<Archive size={18} />}
                label="Archive"
                onSelect={() => {
                  close();
                  boardActions().projectAction('archive-project', current.slug);
                }}
              />
            }
          >
            <MenuRow
              icon={<ArchiveRestore size={18} />}
              label="Unarchive"
              onSelect={() => {
                close();
                boardActions().projectAction('unarchive-project', current.slug);
              }}
            />
          </Show>
        </AnchoredMenu>
      )}
    </Show>
  );
}
