/**
 * One host's section of the sessions list (Z-8, WP-A4): This computer first,
 * then the paired PCs (both shells), each collapsible (state persisted per
 * host). Inside: the Pinned group (Z-14) and the project groups (D-9). An
 * offline host keeps its section with an inline Offline row + Retry (Z-90)
 * instead of disappearing.
 *
 * [ui-sessions]: the header's sliders button opens the Properties menu (sort,
 * filter, group, property visibility, compact, subagents, Archive all); the
 * host's view applies client-side over its indexed cards (`lib/session/listView`).
 */
import { createEffect, createMemo, createSignal, For, on, Show, type JSX } from 'solid-js';
import { ChevronDown, ChevronRight, Monitor, RefreshCw, SlidersHorizontal } from 'lucide-solid';
import type { HostInfo } from '../../lib/schemas';
import type { SessionCard } from '../../lib/host/client';
import { hubs } from '../../lib/session/hub';
import { isInternalSession } from '../../lib/session/internal';
import { sessions } from '../../lib/session/store';
import { archiveMany } from '../../lib/session/actions';
import { cronRunLabel } from '../../lib/cronRunLabels';
import { activeFilterCount, listView, resetListFilters, viewGroups, wantsArchived, type CardFacts, type ViewGroup } from '../../lib/session/listView';
import { liveRowState, SessionRow, type ModeBadge } from './SessionRow';
import { createPaging, ProjectGroup } from './ProjectGroup';
import { ArchivedList } from './ArchivedList';
import { SearchResults } from './SearchResults';
import { sidebarSearch } from '../../lib/session/sidebarSearch';
import { ArchiveAllMenu, ListProperties } from './ListProperties';
import { knownRuns, SubagentTree, treeHeight, watchRuns } from './SubagentTree';
import type { RowVariant } from './SessionRow';

const COLLAPSE_KEY = 'unipi.sidebar.hosts';

type Collapsed = Record<string, boolean>;

function readCollapsed(): Collapsed {
  try {
    const v = JSON.parse(localStorage.getItem(COLLAPSE_KEY) ?? '{}') as Collapsed;
    return v && typeof v === 'object' ? v : {};
  } catch {
    return {};
  }
}

/** The persisted collapse state (shared by both shells; `undefined` = default open). */
const [collapsed, setCollapsed] = createSignal<Collapsed>(readCollapsed());
function toggleHost(hostId: string): void {
  setCollapsed((cur) => {
    const next = { ...cur, [hostId]: !cur[hostId] };
    try {
      localStorage.setItem(COLLAPSE_KEY, JSON.stringify(next));
    } catch {
      // not remembered
    }
    return next;
  });
}

/** `true` when the host is collapsed (tests can reset the store). */
export function hostCollapsed(hostId: string): boolean {
  return !!collapsed()[hostId];
}
export function resetHostCollapse(): void {
  setCollapsed({});
  try {
    localStorage.removeItem(COLLAPSE_KEY);
  } catch {
    // ignore
  }
}

/** Project groups shown before "Show more projects" (a host with hundreds of folders). */
export const PROJECTS_PAGE = 15;
/** Rows per "Show more" when the list is not grouped (one flat list). */
export const FLAT_PAGE = 25;

/**
 * The project groups to render: the first `limit`, plus the one holding the shown
 * session wherever it sorts (the active row never disappears behind paging).
 */
export function visibleProjects<G extends { cards: readonly { session_id: string }[] }>(groups: readonly G[], limit: number, activeId?: string): G[] {
  if (groups.length <= limit) return [...groups];
  const head = groups.slice(0, limit);
  const active = activeId ? groups.slice(limit).find((g) => g.cards.some((c) => c.session_id === activeId)) : undefined;
  return active ? [...head, active] : head;
}

export function HostGroup(props: {
  host: HostInfo;
  variant: RowVariant;
  activeSession?: { hostId: string; sessionId: string };
  modeOf: (card: { hostId: string; session_id: string }) => ModeBadge | undefined;
}): JSX.Element {
  const hostId = () => props.host.id;
  /** [ux-sessions] a settled sidebar query: the body shows the host's search pages. */
  const searching = () => !!sidebarSearch.query();
  const hub = createMemo(() => hubs().ensure(props.host.id));
  const view = () => listView(props.host.id);
  const showArchived = () => wantsArchived(view().filters);
  /** The host's chats (never the app's internal ones), unfiltered. */
  const liveCards = createMemo(() => Object.values(hub().state.cards).filter((c) => !isInternalSession(c.session_id)));
  const archivedIds = createMemo(() => (showArchived() ? new Set(hub().state.archived.map((c) => c.session_id)) : new Set<string>()));
  const allCards = createMemo(() => (showArchived() ? [...liveCards(), ...hub().state.archived.filter((c) => !hub().state.cards[c.session_id])] : liveCards()));
  // The Archived filter lists the host's trash inline (loaded once, like the Archived footer).
  createEffect(() => {
    if (showArchived()) void hub().loadArchived();
  });
  const liveModel = (card: SessionCard): string | undefined => {
    const id = sessions().peek(props.host.id, card.session_id)?.state.model?.id;
    return id ? String(id) : undefined;
  };
  const factsOf = (card: SessionCard): CardFacts => ({
    state: liveRowState(card, sessions().peek(props.host.id, card.session_id)),
    ...(card.source === 'cron' ? { cronStatus: cronRunLabel(props.host.id, card.session_id)?.status } : {}),
    model: liveModel(card),
    archived: archivedIds().has(card.session_id),
  });
  const groups = createMemo(() => viewGroups(allCards(), hub().state.meta, view(), factsOf));
  const filterCount = () => activeFilterCount(view().filters);
  const total = () => liveCards().length;
  const isCollapsed = () => hostCollapsed(props.host.id);
  const local = () => props.host.transport === 'local';
  const offline = () => hub().state.error !== undefined && !hub().state.loaded;
  const paging = createPaging();
  const [projectLimit, setProjectLimit] = createSignal(PROJECTS_PAGE);
  const activeId = () => (props.activeSession?.hostId === props.host.id ? props.activeSession.sessionId : undefined);
  const shownGroups = createMemo(() => visibleProjects(groups().groups, projectLimit(), activeId()));
  /** [ux-sessions] Group by Project: folders under "Projects", the No-workspace chats under "Tasks". */
  const projectGroups = createMemo(() => groups().groups.filter((g) => g.cwd !== ''));
  const tasksGroup = createMemo(() => groups().groups.find((g) => g.cwd === ''));
  const shownProjects = createMemo(() => visibleProjects(projectGroups(), projectLimit(), activeId()));
  const hiddenProjects = () => Math.max(0, (view().group === 'project' ? projectGroups() : groups().groups).length - projectLimit());
  // First load (or a reload after going offline): skeleton rows, not an empty list.
  const loading = () => !hub().state.loaded && !offline();

  // Collapsing the host (or regrouping) resets the groups' paging (ZCode WSTP-05).
  createEffect(
    on([isCollapsed, () => view().group], ([now, group], prev) => {
      if (!prev || now !== prev[0] || group !== prev[1]) {
        paging.reset();
        setProjectLimit(PROJECTS_PAGE);
      }
    }),
  );


  /** One group's section (a project folder, a status / agent group, or the headless Tasks rows). */
  const renderGroup = (group: ViewGroup, tasks = false) => (
    <ProjectGroup
      hostId={props.host.id}
      cwd={tasks ? undefined : group.cwd}
      groupKey={group.key}
      label={tasks ? 'Tasks' : group.label}
      section={tasks}
      cards={group.cards}
      meta={hub().state.meta}
      variant={props.variant}
      paging={paging}
      modeOf={(card) => props.modeOf({ hostId: props.host.id, session_id: card.session_id })}
      onPin={pin}
      activeOf={activeOf}
      show={view().props}
      compact={view().compact}
      archivedOf={archivedOf}
      headless={view().group === 'none'}
      page={view().group === 'none' ? FLAT_PAGE : undefined}
      onArchiveAll={
        archivable(group.cards).length
          ? (anchor) => setArchiveAsk({ anchor, label: tasks ? 'Tasks' : group.label, cards: archivable(group.cards) })
          : undefined
      }
      treeOf={treeOf}
    />
  );
  const pin = (card: { session_id: string }, pinned: boolean) => void hub().setMeta(card.session_id, { pinned });
  const activeOf = (card: SessionCard) => props.activeSession?.hostId === props.host.id && props.activeSession?.sessionId === card.session_id;
  const archivedOf = (card: SessionCard) => archivedIds().has(card.session_id);

  /** What "Archive all" takes: listed, live, not pinned, not running / waiting. */
  const archivable = (cards: readonly SessionCard[]) =>
    cards.filter((c) => !archivedIds().has(c.session_id) && !hub().state.meta[c.session_id]?.pinned && !['working', 'needs-you'].includes(factsOf(c).state));
  const allArchivable = createMemo(() => archivable(groups().groups.flatMap((g) => g.cards)));

  // ── Properties menu + group "Archive all" confirm ──
  const [propsAt, setPropsAt] = createSignal<DOMRect | undefined>();
  const [archiveAsk, setArchiveAsk] = createSignal<{ anchor: DOMRect; label: string; cards: SessionCard[] } | undefined>();

  // ── subagents tree (opt-in) ──
  createEffect(() => {
    if (!view().subagents) return;
    for (const c of liveCards()) watchRuns(sessions().peek(props.host.id, c.session_id));
  });
  const treeOf = (card: SessionCard) => {
    const v = view();
    const explicit = v.subagentRows.includes(card.session_id);
    if (!explicit && !v.subagents) return undefined;
    if (archivedIds().has(card.session_id)) return undefined;
    const runs = knownRuns(props.host.id, card.session_id);
    if (!explicit && !runs?.length) return undefined;
    return {
      height: treeHeight(props.variant, props.host.id, card.session_id, runs),
      render: () => <SubagentTree hostId={props.host.id} sessionId={card.session_id} cwd={card.cwd} variant={props.variant} explicit={explicit} />,
    };
  };

  return (
    <section class="sb-host" data-testid={`sb-host-${props.host.id}`} data-collapsed={isCollapsed() ? '' : undefined}>
      <div class="sb-host-bar">
        <button type="button" class="sb-host-head" data-testid={`sb-host-head-${props.host.id}`} onClick={() => toggleHost(props.host.id)} aria-expanded={!isCollapsed()}>
          <Show when={isCollapsed()} fallback={<ChevronDown size={14} class="sb-host-chevron" />}>
            <ChevronRight size={14} class="sb-host-chevron" />
          </Show>
          <Show when={local()} fallback={<Monitor size={14} class="sb-host-pc" />}>
            <Monitor size={14} class="sb-host-pc sb-host-pc-local" />
          </Show>
          <span class="sb-host-name">{local() ? 'This computer' : props.host.name}</span>
          <Show when={total() > 0}>
            <span class="sb-host-count" title={filterCount() ? `${groups().total} of ${total()} sessions shown` : `${total()} sessions`}>
              {filterCount() ? `${groups().total}/${total()}` : total()}
            </span>
          </Show>
          <Show when={offline()}>
            <span class="sb-host-offline" title={hub().state.error?.message ?? 'Offline'}>Offline</span>
            <span
              role="button"
              tabindex={0}
              class="sb-host-retry"
              title="Retry connecting"
              aria-label="Retry connecting"
              data-testid={`sb-host-retry-${props.host.id}`}
              onClick={(e) => (e.stopPropagation(), void hub().reload())}
              onKeyDown={(e) => e.key === 'Enter' && (e.stopPropagation(), void hub().reload())}
            >
              <RefreshCw size={13} />
            </span>
          </Show>
        </button>
        <button
          type="button"
          class="sb-host-props"
          data-active={filterCount() ? '' : undefined}
          aria-label={filterCount() ? `List properties (${filterCount()} filters on)` : 'List properties: sort, filter, group'}
          title={filterCount() ? `Properties · ${filterCount()} ${filterCount() === 1 ? 'filter' : 'filters'} on` : 'Properties: sort, filter, group, view'}
          aria-haspopup="menu"
          aria-expanded={!!propsAt()}
          data-testid={`sb-props-${props.host.id}`}
          onClick={(e) => setPropsAt((e.currentTarget as HTMLElement).getBoundingClientRect())}
        >
          <SlidersHorizontal size={14} />
          <Show when={filterCount()}>
            <span class="sb-host-props-n">{filterCount()}</span>
          </Show>
        </button>
      </div>
      <Show when={propsAt()}>
        {(at) => (
          <ListProperties
            hostId={hostId()}
            anchor={at()}
            cards={liveCards()}
            modelOf={liveModel}
            archivable={allArchivable()}
            onArchive={(ids) => void archiveMany(props.host.id, ids)}
            onDismiss={() => setPropsAt(undefined)}
          />
        )}
      </Show>
      <Show when={archiveAsk()}>
        {(ask) => (
          <ArchiveAllMenu
            anchor={ask().anchor}
            count={ask().cards.length}
            scope={ask().label}
            onConfirm={() => void archiveMany(props.host.id, ask().cards.map((c) => c.session_id))}
            onDismiss={() => setArchiveAsk(undefined)}
          />
        )}
      </Show>
      <Show when={!isCollapsed()}>
        <div class="sb-host-body">
          {/* [ux-sessions] a sidebar search replaces the groups with the host's query pages. */}
          <Show when={!searching()} fallback={<SearchResults hostId={props.host.id} variant={props.variant} activeOf={activeOf} onPin={pin} />}>
            <Show when={loading()}>
              <div class="sb-skeleton" role="status" aria-label="Loading sessions" data-testid={`sb-skeleton-${props.host.id}`}>
                <For each={[78, 64, 86, 58, 72]}>{(w) => <span class="sb-skel-row" style={{ width: `${w}%` }} />}</For>
              </div>
            </Show>
            {/* [ux-sessions] a batch archive / restore: the host's progress, never an empty state. */}
            <Show when={hub().state.bulk}>
              {(b) => (
                <div class="sb-bulk" role="status" aria-live="polite" data-testid={`sb-bulk-${props.host.id}`}>
                  <span class="sb-bulk-label">
                    {b().kind === 'archive' ? 'Archiving' : 'Restoring'} {b().done} of {b().total}…
                  </span>
                  <span class="sb-bulk-bar" aria-hidden="true">
                    <span class="sb-bulk-fill" style={{ width: `${Math.round((100 * b().done) / Math.max(1, b().total))}%` }} />
                  </span>
                </div>
              )}
            </Show>
            {/* Only a settled, current list may say it is empty (never while loading or mid-batch). */}
            <Show when={hub().state.loaded && !hub().state.loading && !hub().state.bulk && total() === 0 && !offline() && !filterCount()}>
              <p class="sb-empty" data-testid={`sb-empty-${props.host.id}`}>
                {hub().state.archivedLoaded && hub().state.archived.length > 0 ? 'Every session is archived' : 'No sessions yet'}
              </p>
            </Show>
            <Show when={hub().state.loaded && !offline() && filterCount() > 0 && groups().total === 0}>
              <p class="sb-empty sb-empty-filtered" data-testid={`sb-no-match-${props.host.id}`}>
                No sessions found
                <button type="button" class="sb-reset-filters" data-testid={`sb-reset-filters-${props.host.id}`} onClick={() => resetListFilters(props.host.id)}>
                  Reset filters
                </button>
              </p>
            </Show>
            <Show when={offline()}>
              <p class="sb-empty sb-empty-error" role="alert">{hub().state.error?.message ?? 'This PC is offline'}</p>
            </Show>
            <Show when={groups().pinned.length > 0}>
              <div class="sb-project sb-pinned" data-testid="sb-pinned">
                <div class="sb-project-head" title="Pinned sessions">
                  <span class="sb-project-label">Pinned</span>
                  <span class="sb-project-count">{groups().pinned.length}</span>
                </div>
                <div class="sb-rows">
                  <For each={groups().pinned}>
                    {(card) => (
                      <div class="sb-row-slot">
                        <SessionRow
                          hostId={props.host.id}
                          card={card}
                          meta={hub().state.meta[card.session_id]}
                          active={activeOf(card)}
                          variant={props.variant}
                          mode={props.modeOf({ hostId: props.host.id, session_id: card.session_id })}
                          showFolder={props.variant === 'page' && view().group === 'project'}
                          show={view().props}
                          compact={view().compact}
                          onPin={pin}
                        />
                        <Show when={treeOf(card)}>{(t) => t().render()}</Show>
                      </div>
                    )}
                  </For>
                </div>
              </div>
            </Show>
            {/* [ux-sessions] Task list: "Projects" (folder rows) then "Tasks" (chats without a workspace). */}
            <Show when={view().group === 'project' && shownProjects().length > 0}>
              <div class="sb-section" data-testid={`sb-section-projects-${props.host.id}`}>
                Projects
              </div>
            </Show>
            <For each={view().group === 'project' ? shownProjects() : shownGroups()}>{(group) => renderGroup(group)}</For>
            <Show when={hiddenProjects() > 0}>
              <button
                type="button"
                class="sb-more sb-more-projects"
                data-testid={`sb-more-projects-${props.host.id}`}
                title={`${hiddenProjects()} more groups`}
                onClick={() => setProjectLimit((n) => n + PROJECTS_PAGE)}
              >
                {view().group === 'project' ? 'Show more projects' : 'Show more groups'}
                <span class="sb-more-count">{hiddenProjects()}</span>
              </button>
            </Show>
            <Show when={view().group === 'project' ? tasksGroup() : undefined} keyed>
              {(g) => renderGroup(g, true)}
            </Show>
            {/* Z-13 (WP-A5): the host's trash — open read-only, restore or delete archived sessions. */}
            <ArchivedList
              hostId={props.host.id}
              variant={props.variant}
              activeSessionId={props.activeSession?.hostId === props.host.id ? props.activeSession.sessionId : undefined}
            />
          </Show>
        </div>
      </Show>
    </section>
  );
}
