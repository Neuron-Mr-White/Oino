/**
 * Dashboard — every project on a PC at a glance (UNI-117 kanboard migration;
 * the kanboard web UI's dashboard.tsx, UNI-67, in the app's look). Shown by
 * the Board screen in its "Overview" mode: greeting + KPIs, the needs-you
 * inbox grouped by project (Approve / Rework / Answer), agents running, up
 * next, 14-day throughput with health tiles, clustered activity and the
 * project cards. Desktop/web: two columns; phone: one. Moves go through the
 * same rules as the board (a move that needs a note opens the task instead),
 * Approve carries the undo toast. Live: each project's revision stream
 * (≤ 12) reloads it, debounced.
 */
import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { Check, ChevronDown, ChevronRight, EllipsisVertical, Archive, ArchiveRestore, Sparkles, RefreshCw } from 'lucide-solid';
import { api, describe, displayTitle, laneLabel, needsComment, type DashboardData, type ProjectSummary, type RunningItem, type Task } from '../../lib/board/api';
import { boardStore } from '../../lib/board/store';
import { boardUi } from '../../lib/board/ui';
import { hostClient } from '../../lib/host/factory';
import { boardActions } from '../../lib/board/actions';
import { elapsed, hue } from '../../lib/board/view';
import {
  activityLine,
  clusterActivity,
  dashStats,
  dayGroups,
  formatDuration,
  greeting,
  groupInbox,
  inboxCounts,
  relativeAgo,
  sortProjects,
  summaryLine,
  throughput,
  waitAge,
  type InboxFilter,
  type SortMode,
} from '../../lib/board/dashboard';
import { AnchoredMenu, MenuRow } from './AnchoredMenu';
import { PriorityGlyph, ProjectTile, StatusGlyph } from './Glyphs';
import { now } from './TaskCard';
import './dashboard.css';

const SORT_KEY = 'unipi.board.dashSort';
const INBOX_KEY = 'unipi.board.dashInbox';
const SEGMENTS = ['backlog', 'todo', 'in_progress', 'blocked', 'in_review', 'done'];

type InboxPrefs = { collapsed: string[]; expanded: string[]; all: boolean };
function readPrefs(): InboxPrefs {
  try {
    const parsed = JSON.parse(localStorage.getItem(INBOX_KEY) ?? 'null') as Partial<InboxPrefs> | null;
    if (parsed && Array.isArray(parsed.collapsed) && Array.isArray(parsed.expanded)) return { collapsed: parsed.collapsed, expanded: parsed.expanded, all: parsed.all === true };
  } catch {
    /* corrupt → defaults */
  }
  return { collapsed: [], expanded: [], all: false };
}

export function Dashboard(props: { hostId: string; onOpenTask: (slug: string, id: string) => void; onOpenProject: (slug: string) => void }) {
  const store = boardStore();
  const ui = boardUi();
  const [data, setData] = createSignal<DashboardData | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | undefined>();
  const [filter, setFilter] = createSignal<InboxFilter>('all');
  const [sort, setSortSignal] = createSignal<SortMode>(((): SortMode => {
    const stored = localStorage.getItem(SORT_KEY);
    return stored === 'recent' || stored === 'name' ? stored : 'attention';
  })());
  const [prefs, setPrefsSignal] = createSignal<InboxPrefs>(readPrefs());
  const [showAllActivity, setShowAllActivity] = createSignal(false);
  const [showArchived, setShowArchived] = createSignal(false);
  const [busy, setBusy] = createSignal<Set<string>>(new Set());
  const [projectMenu, setProjectMenu] = createSignal<{ project: ProjectSummary; anchor: DOMRect } | null>(null);

  const setSort = (next: SortMode) => {
    setSortSignal(next);
    localStorage.setItem(SORT_KEY, next);
  };
  const setPrefs = (next: InboxPrefs) => {
    setPrefsSignal(next);
    localStorage.setItem(INBOX_KEY, JSON.stringify(next));
  };

  let seq = 0;
  async function load() {
    const mine = ++seq;
    try {
      const dash = await api.dashboard(props.hostId);
      if (mine !== seq) return;
      setData(dash);
      setError(undefined);
    } catch (cause) {
      if (mine === seq) setError(describe(cause));
    } finally {
      if (mine === seq) setLoading(false);
    }
  }
  onMount(() => {
    void load();
    void store.loadProjects();
  });

  // Live: board changes refresh the dashboard (debounced), through the same
  // hub-event stream the board lanes use — the watcher is on the files, so
  // one subscription per host covers every project.
  let debounce: ReturnType<typeof setTimeout> | undefined;
  const scheduleReload = () => {
    clearTimeout(debounce);
    debounce = setTimeout(() => {
      void load();
      void store.loadProjects();
    }, 400);
  };
  createEffect(() => {
    const stop = hostClient(props.hostId).openEvents((event) => {
      if (event.kind === 'board_changed') scheduleReload();
    });
    onCleanup(() => {
      clearTimeout(debounce);
      stop.close();
    });
  });

  const counts = createMemo(() => inboxCounts(data()?.inbox ?? []));
  const groups = createMemo(() => groupInbox(data()?.inbox ?? [], filter(), prefs()));
  const capHidden = createMemo(() => groups().reduce((sum, group) => sum + (group.collapsed ? 0 : group.hidden), 0));
  const running = () => store.state.running;
  const days = createMemo(() => throughput(data()?.doneAt ?? [], now()));
  const maxDay = createMemo(() => Math.max(1, ...days().map((day) => day.count)));
  const stats = createMemo(() => dashStats(data(), now()));
  const activity = createMemo(() => clusterActivity(data()?.activity ?? []));
  const activityShown = createMemo(() => dayGroups(showAllActivity() ? activity() : activity().slice(0, 8), now()));
  const dashBySlug = createMemo(() => new Map((data()?.projects ?? []).map((project) => [project.slug, project])));
  const projects = createMemo(() => sortProjects(store.state.projects, dashBySlug(), sort()));
  const archived = () => store.state.projects.filter((project) => project.archived);
  const oldWait = createMemo(() => (data()?.inbox ?? []).some((item) => waitAge(item.waitingSince, now()).old));

  /** Approve (→ Done) / Rework (→ Todo). A move that needs a note opens the task instead. */
  async function move(slug: string, task: Task, to: string) {
    const key = `${slug}/${task.id}`;
    if (needsComment(store.state.rules, task.status, to)) return props.onOpenTask(slug, task.id);
    setBusy((current) => new Set(current).add(key));
    try {
      await api.move(props.hostId, slug, task.id, to);
      ui.toast(`Moved ${task.id} to ${laneLabel(to)}`, 'success');
    } catch (cause) {
      ui.toast(describe(cause), 'error');
    } finally {
      setBusy((current) => {
        const next = new Set(current);
        next.delete(key);
        return next;
      });
      await load();
    }
  }

  const scrollTo = (selector: string) => document.querySelector(selector)?.scrollIntoView({ behavior: 'smooth', block: 'start' });

  return (
    <div class="dash" data-testid="dashboard">
      <header class="dash-head">
        <div class="dash-head-top">
          <span>{new Date(now()).toLocaleDateString(undefined, { weekday: 'long', month: 'long', day: 'numeric' })}</span>
          <span class="flex-1" />
          <button type="button" class="ghost-icon-button dash-refresh" aria-label="Refresh overview" onClick={() => void load()}>
            <RefreshCw size={16} />
          </button>
        </div>
        <h2 class="dash-greeting">{greeting(new Date(now()).getHours())}</h2>
        <p class="dash-summary" classList={{ calm: counts().all === 0 && running().length === 0 }}>
          {loading() ? 'Loading the boards…' : summaryLine(counts(), running().length)}
        </p>
        <div class="dash-kpis" role="group" aria-label="Key numbers">
          <button type="button" class="dash-kpi" classList={{ amber: oldWait() }} onClick={() => scrollTo('.dash-inbox')} aria-label={`${counts().all} waiting on you — jump to the inbox`}>
            <b class="tabular-nums">{counts().all}</b>
            <span>Waiting on you</span>
          </button>
          <button type="button" class="dash-kpi" onClick={() => scrollTo('.dash-agents')} aria-label={`${running().length} agents running`}>
            <b class="tabular-nums">
              {running().length}
              <Show when={running().length > 0}>
                <span class="dash-live-dot" aria-hidden="true" />
              </Show>
            </b>
            <span>Agents running</span>
          </button>
          <button type="button" class="dash-kpi" onClick={() => scrollTo('.dash-next')} aria-label={`${data()?.readyTotal ?? 0} ready to pick`}>
            <b class="tabular-nums">{data()?.readyTotal ?? 0}</b>
            <span>Ready to pick</span>
          </button>
          <div class="dash-kpi static">
            <b class="tabular-nums">{stats().done7d}</b>
            <span>Done 7d</span>
            <span class="dash-spark" aria-hidden="true">
              <For each={days()}>{(day) => <span classList={{ filled: day.count > 0 }} style={{ height: `${day.count === 0 ? 12 : Math.max(30, (day.count / maxDay()) * 100)}%` }} />}</For>
            </span>
          </div>
        </div>
      </header>

      <Show when={error()}>
        <p class="dash-error" role="alert">
          Couldn't load the overview: {error()}{' '}
          <button type="button" class="lane-empty-clear" onClick={() => void load()}>
            Retry
          </button>
        </p>
      </Show>

      <div class="dash-grid">
        <div class="dash-col">
          <section class="dash-panel dash-inbox" aria-label="Needs you">
            <div class="dash-panel-head">
              <h3>Needs you</h3>
              <span class="flex-1" />
              <Show when={groups().length > 1}>
                <button
                  type="button"
                  class="dash-text-btn"
                  onClick={() => {
                    const all = groups().every((group) => group.collapsed);
                    setPrefs({ ...prefs(), collapsed: all ? [] : groups().map((group) => group.slug) });
                  }}
                >
                  {groups().every((group) => group.collapsed) ? 'Expand all' : 'Collapse all'}
                </button>
              </Show>
              <div class="dash-chips" role="radiogroup" aria-label="Filter the inbox">
                <For each={[['all', 'All'], ['review', 'Review'], ['blocked', 'Blocked']] as const}>
                  {([mode, label]) => (
                    <button type="button" role="radio" class="dash-chip" aria-checked={filter() === mode} onClick={() => setFilter(mode)}>
                      {label} <span class="tabular-nums">{counts()[mode]}</span>
                    </button>
                  )}
                </For>
              </div>
            </div>
            <Show when={!loading()} fallback={<div class="skeleton dash-skel" />}>
              <Show
                when={groups().length > 0}
                fallback={
                  <div class="dash-zero">
                    <Sparkles size={22} />
                    <b>Inbox zero</b>
                    <span>{counts().all > 0 ? 'Nothing here matches this filter.' : 'Everything is moving. Go build something.'}</span>
                  </div>
                }
              >
                <For each={groups()}>
                  {(group) => (
                    <div class="dash-group" data-project={group.slug}>
                      <button
                        type="button"
                        class="dash-group-head"
                        aria-expanded={!group.collapsed}
                        onClick={() => {
                          const p = prefs();
                          setPrefs({ ...p, collapsed: p.collapsed.includes(group.slug) ? p.collapsed.filter((s) => s !== group.slug) : [...p.collapsed, group.slug] });
                        }}
                      >
                        <Show when={group.collapsed} fallback={<ChevronDown size={14} />}>
                          <ChevronRight size={14} />
                        </Show>
                        <ProjectTile name={group.project} hue={hue(group.project)} size={18} />
                        <span class="dash-group-name truncate">{group.project}</span>
                        <span class="flex-1" />
                        <Show when={group.review > 0}>
                          <span class="dash-badge" data-lane="in_review">
                            <StatusGlyph status="in_review" size={11} /> {group.review}
                          </span>
                        </Show>
                        <Show when={group.blocked > 0}>
                          <span class="dash-badge" data-lane="blocked">
                            <StatusGlyph status="blocked" size={11} /> {group.blocked}
                          </span>
                        </Show>
                        <Show when={group.collapsed}>
                          <span class="dash-oldest">oldest {waitAge(new Date(group.oldest).toISOString(), now()).label}</span>
                        </Show>
                      </button>
                      <For each={group.visible}>
                        {(item) => {
                          const age = () => waitAge(item.waitingSince, now());
                          const key = `${item.slug}/${item.task.id}`;
                          return (
                            <div
                              class="dash-row"
                              data-lane={item.task.status}
                              data-id={item.task.id}
                              tabindex="0"
                              role="button"
                              aria-label={`${item.task.id} ${displayTitle(item.task)}`}
                              classList={{ busy: busy().has(key) }}
                              onClick={() => props.onOpenTask(item.slug, item.task.id)}
                              onKeyDown={(event) => event.key === 'Enter' && event.target === event.currentTarget && props.onOpenTask(item.slug, item.task.id)}
                            >
                              <div class="dash-row-line">
                                <StatusGlyph status={item.task.status} size={14} />
                                <span class="bcard-id">{item.task.id}</span>
                                <span class="dash-row-title truncate">{displayTitle(item.task)}</span>
                                <span class="dash-age" classList={{ amber: age().old }} title={`waiting since ${item.waitingSince}`}>
                                  {age().label}
                                </span>
                              </div>
                              <Show when={item.excerpt}>
                                <p class="dash-row-excerpt">{item.excerpt}</p>
                              </Show>
                              <div class="dash-row-actions">
                                <Show when={item.task.status === 'in_review'}>
                                  <button
                                    type="button"
                                    class="dash-act approve"
                                    aria-label={`Approve ${item.task.id}`}
                                    disabled={busy().has(key)}
                                    onClick={(event) => {
                                      event.stopPropagation();
                                      void move(item.slug, item.task, 'done');
                                    }}
                                  >
                                    <Check size={14} /> Approve
                                  </button>
                                  <button
                                    type="button"
                                    class="dash-act"
                                    aria-label={`Send ${item.task.id} back to todo`}
                                    disabled={busy().has(key)}
                                    onClick={(event) => {
                                      event.stopPropagation();
                                      void move(item.slug, item.task, 'todo');
                                    }}
                                  >
                                    Rework
                                  </button>
                                </Show>
                                <Show when={item.task.status === 'blocked'}>
                                  <button
                                    type="button"
                                    class="dash-act"
                                    aria-label={`Answer ${item.task.id}`}
                                    onClick={(event) => {
                                      event.stopPropagation();
                                      props.onOpenTask(item.slug, item.task.id);
                                    }}
                                  >
                                    Answer
                                  </button>
                                </Show>
                              </div>
                            </div>
                          );
                        }}
                      </For>
                      <Show when={!group.collapsed && group.hidden > 0}>
                        <button
                          type="button"
                          class="dash-more"
                          onClick={() => setPrefs({ ...prefs(), expanded: [...prefs().expanded, group.slug] })}
                        >
                          +{group.hidden} more in {group.project}
                        </button>
                      </Show>
                      <Show when={!group.collapsed && group.hidden === 0 && prefs().expanded.includes(group.slug) && group.items.length > 3}>
                        <button type="button" class="dash-more" onClick={() => setPrefs({ ...prefs(), expanded: prefs().expanded.filter((s) => s !== group.slug) })}>
                          Show fewer in {group.project}
                        </button>
                      </Show>
                    </div>
                  )}
                </For>
                <Show when={capHidden() > 0 && !prefs().all}>
                  <button type="button" class="dash-show-all" onClick={() => setPrefs({ ...prefs(), all: true })}>
                    Show all {counts()[filter()]}
                  </button>
                </Show>
                <Show when={prefs().all && counts()[filter()] > 10}>
                  <button type="button" class="dash-show-all" onClick={() => setPrefs({ ...prefs(), all: false, expanded: [] })}>
                    Show less
                  </button>
                </Show>
              </Show>
            </Show>
          </section>

          <section class="dash-panel" aria-label="Activity">
            <div class="dash-panel-head">
              <h3>Activity</h3>
            </div>
            <Show when={!loading()} fallback={<div class="skeleton dash-skel" />}>
              <Show when={activity().length > 0} fallback={<p class="dash-empty">Nothing in the last two days.</p>}>
                <For each={activityShown()}>
                  {(day) => (
                    <div class="dash-day">
                      <div class="dash-day-label">{day.label}</div>
                      <For each={day.items}>
                        {(row) => (
                          <div class="dash-activity-row">
                            <ProjectTile name={row.project} hue={hue(row.project)} size={18} />
                            <div class="min-w-0 flex-1">
                              <button type="button" class="dash-link" onClick={() => props.onOpenTask(row.slug, row.taskId)}>
                                <span class="bcard-id">{row.taskId}</span> <span class="dash-activity-title">{row.title}</span>
                              </button>
                              <div class="dash-activity-text truncate" title={row.lastText}>
                                <span class="dash-actor">{row.actor}</span> {activityLine(row.lastText)}
                                <Show when={row.repeats > 1}>
                                  <span class="text-text-dim"> ×{row.repeats}</span>
                                </Show>
                              </div>
                              <Show when={row.steps.length > 1}>
                                <div class="dash-steps">
                                  <For each={row.steps}>
                                    {(step, index) => (
                                      <>
                                        <Show when={index() > 0}>
                                          <span class="text-text-dim" aria-hidden="true">
                                            →
                                          </span>
                                        </Show>
                                        <span class="dash-step" data-lane={step.status ?? undefined}>
                                          {step.label}
                                        </span>
                                      </>
                                    )}
                                  </For>
                                </div>
                              </Show>
                            </div>
                            <span class="dash-when">{relativeAgo(row.at, now())}</span>
                          </div>
                        )}
                      </For>
                    </div>
                  )}
                </For>
                <Show when={activity().length > 8}>
                  <button type="button" class="dash-show-all" onClick={() => setShowAllActivity(!showAllActivity())}>
                    {showAllActivity() ? 'Show less' : `Show ${activity().length - 8} more`}
                  </button>
                </Show>
              </Show>
            </Show>
          </section>
        </div>

        <div class="dash-col">
          <section class="dash-panel dash-agents" aria-label="Agents running">
            <div class="dash-panel-head">
              <h3>Agents</h3>
            </div>
            <Show when={running().length > 0} fallback={<p class="dash-empty">No agents running.</p>}>
              <For each={running()}>
                {(item: RunningItem) => (
                  <button type="button" class="dash-agent" onClick={() => props.onOpenTask(item.slug, item.task.id)}>
                    <span class="agent-dot">
                      <span class="pulse" />
                    </span>
                    <span class="min-w-0 flex-1 text-left">
                      <span class="block truncate">
                        <span class="bcard-id">{item.task.id}</span> {displayTitle(item.task)}
                      </span>
                      <span class="dash-sub truncate">
                        {item.project} · session {item.task.run?.session ?? '?'}
                        <Show when={item.task.staleness === 'stale'}>
                          <span class="btag stale ml-1">stale</span>
                        </Show>
                      </span>
                    </span>
                    <span class="dash-when text-amber">{elapsed(item.task.run?.started, now())}</span>
                  </button>
                )}
              </For>
            </Show>
          </section>

          <section class="dash-panel dash-next" aria-label="Up next">
            <div class="dash-panel-head">
              <h3>Up next</h3>
            </div>
            <Show when={(data()?.upNext.length ?? 0) > 0} fallback={<p class="dash-empty">{loading() ? '…' : 'No ready tasks.'}</p>}>
              <For each={data()?.upNext ?? []}>
                {(item) => (
                  <button type="button" class="dash-next-row" onClick={() => props.onOpenTask(item.slug, item.task.id)} title={`${item.project} · ${item.task.id}`}>
                    <PriorityGlyph priority={item.task.priority} size={14} />
                    <ProjectTile name={item.project} hue={hue(item.project)} size={16} />
                    <span class="bcard-id">{item.task.id}</span>
                    <span class="truncate">{displayTitle(item.task)}</span>
                  </button>
                )}
              </For>
            </Show>
          </section>

          <section class="dash-panel" aria-label="Last 14 days">
            <div class="dash-panel-head">
              <h3>Last 14 days</h3>
            </div>
            <div class="dash-bars" role="img" aria-label="Done tasks per day, last 14 days">
              <For each={days()}>
                {(day) => (
                  <span class="dash-bar-col" title={`${day.label} · ${day.count} done`}>
                    <span class="dash-bar" classList={{ today: day.today, empty: day.count === 0 }} style={{ height: `${day.count === 0 ? 4 : Math.max(10, (day.count / maxDay()) * 100)}%` }} />
                  </span>
                )}
              </For>
            </div>
            <div class="dash-tiles">
              <div class="dash-tile">
                <b class="tabular-nums">{stats().done7d}</b>
                <span>done this week</span>
              </div>
              <div class="dash-tile">
                <b>{stats().hasWaits ? formatDuration(stats().medianWait) : '—'}</b>
                <span>{stats().hasWaits ? 'median review wait' : 'no completed reviews yet'}</span>
              </div>
              <div class="dash-tile">
                <b class="tabular-nums">{stats().blocked}</b>
                <span>blocked</span>
              </div>
              <div class="dash-tile" classList={{ warn: stats().repairs > 0 }}>
                <b class="tabular-nums">{stats().repairs}</b>
                <span>files need repair</span>
              </div>
            </div>
          </section>

          <section class="dash-panel" aria-label="Projects">
            <div class="dash-panel-head">
              <h3>Projects</h3>
              <span class="flex-1" />
              <div class="dash-chips" role="radiogroup" aria-label="Sort projects">
                <For each={[['attention', 'Attention'], ['recent', 'Recent'], ['name', 'Name']] as const}>
                  {([mode, label]) => (
                    <button type="button" role="radio" class="dash-chip" aria-checked={sort() === mode} onClick={() => setSort(mode)}>
                      {label}
                    </button>
                  )}
                </For>
              </div>
            </div>
            <Show when={store.state.projects.length > 0} fallback={<p class="dash-empty">No projects yet. Use “Add project…” in the Board menu.</p>}>
              <div class="dash-projects">
                <For each={projects()}>
                  {(project) => (
                    <ProjectCard
                      project={project}
                      dash={dashBySlug().get(project.slug)}
                      onOpen={() => props.onOpenProject(project.slug)}
                      onMenu={(anchor) => setProjectMenu({ project, anchor })}
                    />
                  )}
                </For>
              </div>
            </Show>
            <Show when={archived().length > 0}>
              <button type="button" class="board-section-toggle" aria-expanded={showArchived()} onClick={() => setShowArchived(!showArchived())}>
                <Show when={showArchived()} fallback={<ChevronRight size={14} />}>
                  <ChevronDown size={14} />
                </Show>
                Archived projects
                <span class="ml-auto tabular-nums">{archived().length}</span>
              </button>
              <Show when={showArchived()}>
                <div class="dash-projects">
                  <For each={archived()}>
                    {(project) => <ProjectCard project={project} onOpen={() => props.onOpenProject(project.slug)} onMenu={(anchor) => setProjectMenu({ project, anchor })} />}
                  </For>
                </div>
              </Show>
            </Show>
          </section>
        </div>
      </div>

      <Show when={projectMenu()}>
        {(menu) => (
          <AnchoredMenu anchor={menu().anchor} alignEnd width={220} label={`${menu().project.name} actions`} onDismiss={() => setProjectMenu(null)}>
            <MenuRow
              icon={menu().project.archived ? <ArchiveRestore size={18} /> : <Archive size={18} />}
              label={menu().project.archived ? 'Unarchive' : 'Archive'}
              onSelect={() => {
                const project = menu().project;
                setProjectMenu(null);
                boardActions().projectAction(project.archived ? 'unarchive-project' : 'archive-project', project.slug);
              }}
            />
          </AnchoredMenu>
        )}
      </Show>
    </div>
  );
}

function ProjectCard(props: { project: ProjectSummary; dash?: DashboardData['projects'][number]; onOpen: () => void; onMenu: (anchor: DOMRect) => void }) {
  const project = () => props.project;
  const total = () => SEGMENTS.reduce((sum, status) => sum + (project().counts?.[status] ?? 0), 0);
  return (
    <div class="dash-project" classList={{ archived: !!project().archived }} style={{ '--project-hue': hue(project().name) }}>
      <button
        type="button"
        class="dash-project-open"
        onClick={() => props.onOpen()}
        onContextMenu={(event) => {
          event.preventDefault();
          props.onMenu(new DOMRect(event.clientX, event.clientY, 0, 0));
        }}
      >
        <div class="dash-project-head">
          <ProjectTile name={project().name} hue={hue(project().name)} size={28} running={(project().running ?? 0) > 0} />
          <span class="min-w-0 flex-1 text-left">
            <span class="block truncate font-medium">{project().name}</span>
            <span class="dash-sub block truncate">{(project().root ?? project().slug).replace(/^\/home\/[^/]+/, '~')}</span>
          </span>
          <span class="dash-when">{relativeAgo(project().updatedAt, now())}</span>
        </div>
        <Show when={props.dash && (props.dash.review > 0 || props.dash.blocked > 0 || props.dash.ready > 0)}>
          <div class="dash-project-badges">
            <Show when={props.dash!.review > 0}>
              <span class="dash-badge" data-lane="in_review">
                <StatusGlyph status="in_review" size={11} /> {props.dash!.review} review
              </span>
            </Show>
            <Show when={props.dash!.blocked > 0}>
              <span class="dash-badge" data-lane="blocked">
                <StatusGlyph status="blocked" size={11} /> {props.dash!.blocked} blocked
              </span>
            </Show>
            <Show when={props.dash!.ready > 0}>
              <span class="dash-badge" data-lane="todo">
                <StatusGlyph status="todo" size={11} /> {props.dash!.ready} ready
              </span>
            </Show>
          </div>
        </Show>
        <div class="dash-stack" aria-hidden="true">
          <For each={SEGMENTS}>
            {(status) => (
              <Show when={(project().counts?.[status] ?? 0) > 0}>
                <span data-lane={status} style={{ flex: String(project().counts?.[status] ?? 0) }} />
              </Show>
            )}
          </For>
        </div>
        <div class="dash-sub">
          <Show when={total() > 0} fallback={<>No tasks yet</>}>
            {SEGMENTS.filter((status) => (project().counts?.[status] ?? 0) > 0)
              .map((status) => `${project().counts?.[status]} ${laneLabel(status)}`)
              .join(' · ')}
          </Show>
          <Show when={(project().problems ?? []).length > 0}>
            <span class="btag stale ml-1">{(project().problems ?? []).length} need repair</span>
          </Show>
        </div>
      </button>
      <button type="button" class="ghost-icon-button dash-project-menu" aria-label={`${project().name} options`} onClick={(event) => props.onMenu(event.currentTarget.getBoundingClientRect())}>
        <EllipsisVertical size={16} />
      </button>
    </div>
  );
}
