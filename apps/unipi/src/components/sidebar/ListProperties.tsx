/**
 * The sessions list's "Properties" menu ([ui-sessions], after Devin desktop's):
 * the sliders button on a host's list header opens it. One anchored popover
 * with drill-in panels (works the same on the 264 px sidebar and the phone):
 *
 *   Sort ▸ Last updated / Created / Name
 *   Filter ▸ (search, Reset to default) Agent/model ▸, Workspace ▸, Automation ▸,
 *            Status ▸, Created time ▸, Updated time ▸, Archived ▸
 *   Group ▸ None / Project / Agent / Status / Automation
 *   Property visibility ▸ per-chip toggles + Hide all
 *   Compact view · Show subagents
 *   Archive all shown… (confirm with the count; Undo toast)
 *
 * State lives in `lib/session/listView.ts` (persisted per host); this file is
 * only the editor.
 */
import { createMemo, createSignal, For, Show, type JSX } from 'solid-js';
import { Archive, ArrowDownUp, Check, ChevronLeft, ChevronRight, Eye, Filter, LayoutList, ListTree, RotateCcw, Search, Rows2 } from 'lucide-solid';
import type { SessionCard } from '../../lib/host/client';
import { desktopShell } from '../../lib/layout';
import {
  activeFacets,
  ARCHIVED_LABEL,
  AUTOMATION_LABEL,
  dayOf,
  facetOptions,
  GROUP_LABEL,
  isAutomation,
  listView,
  modelKey,
  modelLabel,
  PROP_LABEL,
  PROP_ORDER,
  rangeLabel,
  resetListFilters,
  setListView,
  SORT_LABEL,
  STATUS_LABEL,
  STATUS_ORDER,
  TIME_LABEL,
  toggleIn,
  workspaceKey,
  workspaceLabel,
  type ArchivedKey,
  type AutomationKey,
  type FilterFacet,
  type GroupKey,
  type SortKey,
  type StatusKey,
  type TimePreset,
  type TimeRange,
} from '../../lib/session/listView';
import { shortCwd } from '../../lib/session/selectors';
import { AnchoredMenu } from '../board/AnchoredMenu';

type Panel = 'root' | 'sort' | 'filter' | 'group' | 'props' | 'archive' | `facet:${FilterFacet}` | `custom:${'created' | 'updated'}`;

const FACET_LABEL: Record<FilterFacet, string> = {
  models: 'Agent / model',
  workspaces: 'Workspace',
  automation: 'Automation',
  status: 'Status',
  created: 'Created time',
  updated: 'Updated time',
  archived: 'Archived',
};
const FACET_ORDER: readonly FilterFacet[] = ['models', 'workspaces', 'automation', 'status', 'created', 'updated', 'archived'];
const PRESETS: readonly TimePreset[] = ['today', 'week', '2weeks', 'month'];

export function ListProperties(props: {
  hostId: string;
  anchor: DOMRect;
  /** The host's live cards (filter option lists + counts). */
  cards: readonly SessionCard[];
  /** The model a card runs (live session, else the card's). */
  modelOf: (card: SessionCard) => string | undefined;
  /** Chats "Archive all shown" would archive (filtered, not pinned, not running). */
  archivable: readonly SessionCard[];
  onArchive: (ids: string[]) => void;
  onDismiss: () => void;
  /** Open straight on a panel (the empty state's "Reset filters" never needs it; tests do). */
  initial?: Panel;
}): JSX.Element {
  const [panel, setPanel] = createSignal<Panel>(props.initial ?? 'root');
  const [query, setQuery] = createSignal('');
  const view = () => listView(props.hostId);
  const facets = createMemo(() => new Set(activeFacets(view().filters)));
  const go = (p: Panel) => {
    setQuery('');
    setPanel(p);
  };
  const back = (): Panel => {
    const p = panel();
    if (p.startsWith('facet:')) return 'filter';
    if (p.startsWith('custom:')) return `facet:${p.slice(7) as 'created' | 'updated'}`;
    return 'root';
  };
  const maxHeight = () => Math.max(200, Math.min(520, window.innerHeight - 76));
  const icon = (el: JSX.Element) => <span class="board-menu-icon">{el}</span>;

  const models = createMemo(() => facetOptions(props.cards, (c) => modelKey(c, props.modelOf(c))));
  const workspaces = createMemo(() => facetOptions(props.cards, workspaceKey));
  const counts = <K extends string>(key: (c: SessionCard) => K) => {
    const map = new Map<K, number>();
    for (const c of props.cards) map.set(key(c), (map.get(key(c)) ?? 0) + 1);
    return map;
  };
  const automationCounts = createMemo(() => counts<AutomationKey>((c) => (isAutomation(c) ? 'cron' : 'manual')));

  const facetHint = (f: FilterFacet): string | undefined => {
    const v = view().filters;
    switch (f) {
      case 'models':
        return v.models.length ? (v.models.length === 1 ? modelLabel(v.models[0]!) : `${v.models.length} selected`) : undefined;
      case 'workspaces':
        return v.workspaces.length ? (v.workspaces.length === 1 ? workspaceLabel(v.workspaces[0]!) : `${v.workspaces.length} selected`) : undefined;
      case 'automation':
        return v.automation.length === 1 ? AUTOMATION_LABEL[v.automation[0]!] : v.automation.length ? 'Both' : undefined;
      case 'status':
        return v.status.length ? (v.status.length === 1 ? STATUS_LABEL[v.status[0]!] : `${v.status.length} selected`) : undefined;
      case 'created':
        return rangeLabel(v.created);
      case 'updated':
        return rangeLabel(v.updated);
      case 'archived':
        return v.archived.length === 1 ? ARCHIVED_LABEL[v.archived[0]!] : 'All';
    }
  };

  /** A navigation row (`▸` + current value). */
  const NavRow = (p: { icon?: JSX.Element; label: string; hint?: string; active?: boolean; to: Panel; testId?: string }) => (
    <button type="button" role="menuitem" class="recent-popover-row board-menu-row lp-row" data-active={p.active ? '' : undefined} data-testid={p.testId} onClick={() => go(p.to)}>
      <Show when={p.icon}>{icon(p.icon)}</Show>
      <span class="lp-label">{p.label}</span>
      <Show when={p.hint}>
        <span class="lp-hint">{p.hint}</span>
      </Show>
      <ChevronRight size={14} class="lp-chev" />
    </button>
  );
  /** A radio / checkbox row. */
  const Choice = (p: { label: string; checked: boolean; radio?: boolean; count?: number; title?: string; testId?: string; onSelect: () => void }) => (
    <button
      type="button"
      role={p.radio ? 'menuitemradio' : 'menuitemcheckbox'}
      aria-checked={p.checked}
      class="recent-popover-row board-menu-row lp-row lp-choice"
      data-active={p.checked ? '' : undefined}
      title={p.title}
      data-testid={p.testId}
      onClick={() => p.onSelect()}
    >
      <span class="lp-check" data-radio={p.radio ? '' : undefined} aria-hidden="true">
        <Show when={p.checked}>
          <Check size={12} />
        </Show>
      </span>
      <span class="lp-label">{p.label}</span>
      <Show when={p.count !== undefined}>
        <span class="lp-count">{p.count}</span>
      </Show>
    </button>
  );
  const Toggle = (p: { icon: JSX.Element; label: string; on: boolean; testId?: string; onToggle: () => void }) => (
    <button type="button" role="menuitemcheckbox" aria-checked={p.on} class="recent-popover-row board-menu-row lp-row" data-testid={p.testId} onClick={() => p.onToggle()}>
      {icon(p.icon)}
      <span class="lp-label">{p.label}</span>
      <span class="lp-switch" data-on={p.on ? '' : undefined} aria-hidden="true" />
    </button>
  );
  const SearchBox = (p: { placeholder: string }) => (
    <label class="lp-search">
      <Search size={13} />
      <input
        type="search"
        placeholder={p.placeholder}
        value={query()}
        onInput={(e) => setQuery(e.currentTarget.value)}
        onKeyDown={(e) => {
          // Arrow keys move through the menu rows, not the caret.
          if (e.key === 'ArrowDown' || e.key === 'ArrowUp') return;
          e.stopPropagation();
        }}
        data-testid="lp-search"
      />
    </label>
  );
  const matches = (text: string) => !query().trim() || text.toLocaleLowerCase().includes(query().trim().toLocaleLowerCase());

  const TimePanel = (p: { facet: 'created' | 'updated' }) => {
    const current = () => view().filters[p.facet];
    const set = (range: TimeRange | undefined) => setListView(props.hostId, { filters: { [p.facet]: range } });
    return (
      <>
        <Choice label="Any time" radio checked={!current()} onSelect={() => set(undefined)} />
        <For each={PRESETS}>{(preset) => <Choice label={TIME_LABEL[preset]} radio checked={current()?.preset === preset} testId={`lp-${p.facet}-${preset}`} onSelect={() => set({ preset })} />}</For>
        <NavRow label={TIME_LABEL.custom} hint={current()?.preset === 'custom' ? rangeLabel(current()) : undefined} active={current()?.preset === 'custom'} to={`custom:${p.facet}`} testId={`lp-${p.facet}-custom`} />
      </>
    );
  };
  const CustomPanel = (p: { facet: 'created' | 'updated' }) => {
    const current = () => view().filters[p.facet];
    const today = dayOf(Date.now());
    const set = (from: string | undefined, to: string | undefined) =>
      setListView(props.hostId, { filters: { [p.facet]: from || to ? { preset: 'custom', ...(from ? { from } : {}), ...(to ? { to } : {}) } : undefined } });
    const from = () => (current()?.preset === 'custom' ? current()!.from : undefined);
    const to = () => (current()?.preset === 'custom' ? current()!.to : undefined);
    return (
      <div class="lp-range" data-testid={`lp-range-${p.facet}`}>
        <label class="lp-date">
          <span>From</span>
          <input type="date" max={to() ?? today} value={from() ?? ''} onInput={(e) => set(e.currentTarget.value || undefined, to())} data-testid={`lp-${p.facet}-from`} />
        </label>
        <label class="lp-date">
          <span>To</span>
          <input type="date" min={from()} max={today} value={to() ?? ''} onInput={(e) => set(from(), e.currentTarget.value || undefined)} data-testid={`lp-${p.facet}-to`} />
        </label>
        <button type="button" role="menuitem" class="lp-link" onClick={() => set(undefined, undefined)}>
          Clear range
        </button>
      </div>
    );
  };

  const title = (): string => {
    const p = panel();
    if (p === 'root') return 'Properties';
    if (p === 'sort') return 'Sort';
    if (p === 'filter') return 'Filter';
    if (p === 'group') return 'Group by';
    if (p === 'props') return 'Property visibility';
    if (p === 'archive') return 'Archive all shown';
    if (p.startsWith('facet:')) return FACET_LABEL[p.slice(6) as FilterFacet];
    return `${FACET_LABEL[p.slice(7) as FilterFacet]} · Custom`;
  };

  return (
    <AnchoredMenu anchor={props.anchor} width={desktopShell() ? 264 : 300} class="srow-menu lp" label="List properties" onDismiss={props.onDismiss}>
      <div class="lp-head">
        <Show when={panel() !== 'root'} fallback={<span class="lp-title">{title()}</span>}>
          <button type="button" role="menuitem" class="lp-back" aria-label="Back" data-testid="lp-back" onClick={() => go(back())}>
            <ChevronLeft size={15} />
            <span class="lp-title">{title()}</span>
          </button>
        </Show>
      </div>
      <div class="lp-body" style={{ 'max-height': `${maxHeight()}px` }} data-panel={panel()} data-testid={`lp-panel-${panel()}`}>
        <Show when={panel() === 'root'}>
          <NavRow icon={<ArrowDownUp size={15} />} label="Sort" hint={SORT_LABEL[view().sort]} to="sort" testId="lp-sort" />
          <NavRow icon={<Filter size={15} />} label="Filter" hint={facets().size ? String(facets().size) : undefined} active={facets().size > 0} to="filter" testId="lp-filter" />
          <NavRow icon={<LayoutList size={15} />} label="Group" hint={GROUP_LABEL[view().group]} to="group" testId="lp-group" />
          <NavRow icon={<Eye size={15} />} label="Property visibility" to="props" testId="lp-props" />
          <div class="board-menu-sep" role="separator" />
          <Toggle icon={<Rows2 size={15} />} label="Compact view" on={view().compact} testId="lp-compact" onToggle={() => setListView(props.hostId, { compact: !view().compact })} />
          <Toggle icon={<ListTree size={15} />} label="Show subagents" on={view().subagents} testId="lp-subagents" onToggle={() => setListView(props.hostId, { subagents: !view().subagents })} />
          <div class="board-menu-sep" role="separator" />
          <button type="button" role="menuitem" class="recent-popover-row board-menu-row lp-row" disabled={!props.archivable.length} data-testid="lp-archive-all" onClick={() => go('archive')}>
            {icon(<Archive size={15} />)}
            <span class="lp-label">{facets().size ? 'Archive all shown…' : 'Archive all…'}</span>
            <span class="lp-count">{props.archivable.length}</span>
          </button>
        </Show>

        <Show when={panel() === 'sort'}>
          <For each={['updated', 'created', 'name'] as SortKey[]}>
            {(k) => <Choice label={SORT_LABEL[k]} radio checked={view().sort === k} testId={`lp-sort-${k}`} onSelect={() => setListView(props.hostId, { sort: k })} />}
          </For>
        </Show>

        <Show when={panel() === 'group'}>
          <For each={['none', 'project', 'agent', 'status', 'automation'] as GroupKey[]}>
            {(k) => <Choice label={GROUP_LABEL[k]} radio checked={view().group === k} testId={`lp-group-${k}`} onSelect={() => setListView(props.hostId, { group: k })} />}
          </For>
        </Show>

        <Show when={panel() === 'props'}>
          <For each={PROP_ORDER}>
            {(k) => <Choice label={PROP_LABEL[k]} checked={view().props[k]} testId={`lp-prop-${k}`} onSelect={() => setListView(props.hostId, { props: { [k]: !view().props[k] } })} />}
          </For>
          <div class="board-menu-sep" role="separator" />
          <button
            type="button"
            role="menuitem"
            class="recent-popover-row board-menu-row lp-row"
            data-testid="lp-props-hide-all"
            onClick={() => {
              const anyOn = PROP_ORDER.some((k) => view().props[k]);
              setListView(props.hostId, { props: Object.fromEntries(PROP_ORDER.map((k) => [k, !anyOn])) });
            }}
          >
            <span class="lp-label">{PROP_ORDER.some((k) => view().props[k]) ? 'Hide all' : 'Show all'}</span>
          </button>
        </Show>

        <Show when={panel() === 'filter'}>
          <SearchBox placeholder="Search filters" />
          <For each={FACET_ORDER.filter((f) => matches(FACET_LABEL[f]))}>
            {(f) => <NavRow label={FACET_LABEL[f]} hint={facetHint(f)} active={facets().has(f)} to={`facet:${f}`} testId={`lp-facet-${f}`} />}
          </For>
          <div class="board-menu-sep" role="separator" />
          <button type="button" role="menuitem" class="recent-popover-row board-menu-row lp-row" disabled={!facets().size} data-testid="lp-reset" onClick={() => resetListFilters(props.hostId)}>
            {icon(<RotateCcw size={14} />)}
            <span class="lp-label">Reset to default</span>
          </button>
        </Show>

        <Show when={panel() === 'facet:models'}>
          <SearchBox placeholder="Search models" />
          <For each={models().filter((o) => matches(modelLabel(o.key)))}>
            {(o) => (
              <Choice
                label={modelLabel(o.key)}
                title={o.key || undefined}
                count={o.count}
                checked={view().filters.models.includes(o.key)}
                onSelect={() => setListView(props.hostId, { filters: { models: toggleIn(view().filters.models, o.key) } })}
              />
            )}
          </For>
        </Show>

        <Show when={panel() === 'facet:workspaces'}>
          <SearchBox placeholder="Search workspaces" />
          <For each={workspaces().filter((o) => matches(o.key ? `${workspaceLabel(o.key)} ${o.key}` : workspaceLabel(o.key)))}>
            {(o) => (
              <Choice
                label={workspaceLabel(o.key)}
                title={o.key ? shortCwd(o.key) : undefined}
                count={o.count}
                testId={`lp-ws-${workspaceLabel(o.key)}`}
                checked={view().filters.workspaces.includes(o.key)}
                onSelect={() => setListView(props.hostId, { filters: { workspaces: toggleIn(view().filters.workspaces, o.key) } })}
              />
            )}
          </For>
        </Show>

        <Show when={panel() === 'facet:automation'}>
          <For each={['cron', 'manual'] as AutomationKey[]}>
            {(k) => (
              <Choice
                label={AUTOMATION_LABEL[k]}
                count={automationCounts().get(k) ?? 0}
                testId={`lp-automation-${k}`}
                checked={view().filters.automation.includes(k)}
                onSelect={() => setListView(props.hostId, { filters: { automation: toggleIn(view().filters.automation, k) } })}
              />
            )}
          </For>
        </Show>

        <Show when={panel() === 'facet:status'}>
          <For each={STATUS_ORDER}>
            {(k: StatusKey) => (
              <Choice
                label={STATUS_LABEL[k]}
                testId={`lp-status-${k}`}
                checked={view().filters.status.includes(k)}
                onSelect={() => setListView(props.hostId, { filters: { status: toggleIn(view().filters.status, k) } })}
              />
            )}
          </For>
        </Show>

        <Show when={panel() === 'facet:created'}>
          <TimePanel facet="created" />
        </Show>
        <Show when={panel() === 'facet:updated'}>
          <TimePanel facet="updated" />
        </Show>
        <Show when={panel() === 'custom:created'}>
          <CustomPanel facet="created" />
        </Show>
        <Show when={panel() === 'custom:updated'}>
          <CustomPanel facet="updated" />
        </Show>

        <Show when={panel() === 'facet:archived'}>
          <For each={['archived', 'unarchived'] as ArchivedKey[]}>
            {(k) => (
              <Choice
                label={ARCHIVED_LABEL[k]}
                testId={`lp-archived-${k}`}
                checked={view().filters.archived.includes(k)}
                onSelect={() => setListView(props.hostId, { filters: { archived: toggleIn(view().filters.archived, k) } })}
              />
            )}
          </For>
        </Show>

        <Show when={panel() === 'archive'}>
          <ArchiveConfirm
            count={props.archivable.length}
            what={facets().size ? 'shown' : 'listed'}
            onCancel={() => go('root')}
            onConfirm={() => {
              props.onArchive(props.archivable.map((c) => c.session_id));
              props.onDismiss();
            }}
          />
        </Show>
      </div>
    </AnchoredMenu>
  );
}

/** The "Archive N sessions?" confirm (list header + group header). */
export function ArchiveConfirm(props: { count: number; what: string; scope?: string; onCancel: () => void; onConfirm: () => void }): JSX.Element {
  return (
    <div class="srow-confirm" role="group" aria-label="Confirm archiving" data-testid="archive-all-confirm">
      <p class="srow-confirm-prompt">
        Archive {props.count} {props.count === 1 ? 'session' : 'sessions'}
        {props.scope ? ` in ${props.scope}` : ` ${props.what}`}? Pinned and running chats are kept. You can undo it, or restore them later from Archived.
      </p>
      <div class="srow-confirm-actions">
        <button type="button" role="menuitem" class="srow-confirm-cancel" onClick={() => props.onCancel()}>
          Cancel
        </button>
        <button type="button" role="menuitem" class="srow-confirm-yes" data-testid="archive-all-yes" disabled={!props.count} onClick={() => props.onConfirm()}>
          <Archive size={14} />
          Archive {props.count}
        </button>
      </div>
    </div>
  );
}

/** The group header's confirm popover ("Archive all in <project>"). */
export function ArchiveAllMenu(props: { anchor: DOMRect; count: number; scope: string; onConfirm: () => void; onDismiss: () => void }): JSX.Element {
  return (
    <AnchoredMenu anchor={props.anchor} width={desktopShell() ? 264 : 300} class="srow-menu" alignEnd label={`Archive all in ${props.scope}`} onDismiss={props.onDismiss}>
      <ArchiveConfirm
        count={props.count}
        what=""
        scope={props.scope}
        onCancel={props.onDismiss}
        onConfirm={() => {
          props.onConfirm();
          props.onDismiss();
        }}
      />
    </AnchoredMenu>
  );
}
