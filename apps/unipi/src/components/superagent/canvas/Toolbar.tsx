import { createSignal, For, Show, type Accessor } from 'solid-js';
import { Check, ChevronDown, Circle, ListFilter, Maximize2, MessageSquare, Minus, Network, Plus, RectangleHorizontal, Search, X } from 'lucide-solid';
import { AnchoredMenu, MenuCheckRow, MenuLabel, MenuSeparator } from '../../board/AnchoredMenu';
import { filterActive, STATUSES, STATUS_LABEL, type CanvasFilter, type Density, type FleetGroup, type FleetStatus } from '../fleet';

export type FilterProps = { filter: Accessor<CanvasFilter>; set: (f: CanvasFilter) => void; counts: Accessor<Record<FleetStatus | 'all', number>>; groups: Accessor<readonly FleetGroup[]> };

/** Multi-select status chips with counts. None selected means all. */
export function StatusChips(props: FilterProps) {
  const toggle = (s: FleetStatus) => { const f = props.filter(); props.set({ ...f, statuses: f.statuses.includes(s) ? f.statuses.filter((x) => x !== s) : [...f.statuses, s] }); };
  return <div class="sac-chips" role="group" aria-label="Status">
    <For each={STATUSES}>{(s) => <button type="button" class="sac-chip" data-status={s} aria-pressed={props.filter().statuses.includes(s)} onClick={() => toggle(s)}>
      <i class="sac-chip-dot" aria-hidden="true" />{STATUS_LABEL[s]}<span class="sac-chip-count" aria-label={`${props.counts()[s]} agents`}>{props.counts()[s]}</span></button>}</For>
  </div>;
}

export function ChatsChip(props: FilterProps) {
  return <button type="button" class="sac-chip" data-kind="chats" aria-pressed={props.filter().chats} onClick={() => props.set({ ...props.filter(), chats: !props.filter().chats })}><MessageSquare size={14} aria-hidden="true" />Chats</button>;
}

export function ProjectMenu(props: FilterProps) {
  const [anchor, setAnchor] = createSignal<DOMRect>();
  const hidden = () => props.filter().hidden;
  const label = () => (hidden().length ? `Projects · ${Math.max(0, props.groups().length - hidden().length)}/${props.groups().length}` : 'All projects');
  const toggle = (key: string) => { const f = props.filter(); props.set({ ...f, hidden: f.hidden.includes(key) ? f.hidden.filter((k) => k !== key) : [...f.hidden, key] }); };
  let btn!: HTMLButtonElement;
  return <>
    <button type="button" class="sac-chip" data-kind="projects" ref={btn} aria-haspopup="menu" aria-expanded={!!anchor()} onClick={() => setAnchor(anchor() ? undefined : btn.getBoundingClientRect())}>{label()}<ChevronDown size={14} aria-hidden="true" /></button>
    <Show when={anchor()}>{(a) => <AnchoredMenu anchor={a()} label="Projects" title="Projects" width={260} onDismiss={() => setAnchor(undefined)}>
      <MenuCheckRow label="All projects" checked={hidden().length === 0} onSelect={() => props.set({ ...props.filter(), hidden: [] })} />
      <MenuSeparator />
      <For each={props.groups()}>{(g) => <MenuCheckRow label={g.name} checked={!hidden().includes(g.key)} onSelect={() => toggle(g.key)} />}</For>
    </AnchoredMenu>}</Show>
  </>;
}

export function DensityToggle(props: { density: Accessor<Density>; set: (d: Density) => void }) {
  return <div class="sac-seg" role="radiogroup" aria-label="Density">
    <button type="button" role="radio" aria-checked={props.density() === 'compact'} aria-label="Compact" data-tip="Compact (D)" onClick={() => props.set('compact')}><Circle size={15} aria-hidden="true" /></button>
    <button type="button" role="radio" aria-checked={props.density() === 'detailed'} aria-label="Detailed" data-tip="Detailed (D)" onClick={() => props.set('detailed')}><RectangleHorizontal size={15} aria-hidden="true" /></button>
  </div>;
}

export type ToolbarProps = FilterProps & {
  phone: boolean; density: Accessor<Density>; setDensity: (d: Density) => void; query: Accessor<string>; setQuery: (q: string) => void; onEnter: () => void;
  fit: () => void; onClose: () => void; openFilters: () => void; searchRef: (el: HTMLInputElement) => void;
};

export function Toolbar(props: ToolbarProps) {
  const search = <label class="sac-search"><Search size={15} aria-hidden="true" />
    <input ref={props.searchRef} type="text" role="searchbox" aria-label="Search agents" placeholder="Search title, project or ticket" value={props.query()} autocomplete="off" spellcheck={false}
      onInput={(e) => props.setQuery(e.currentTarget.value)} onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); props.onEnter(); } else if (e.key === 'Escape' && props.query()) { e.preventDefault(); e.stopPropagation(); props.setQuery(''); } }} />
    <Show when={props.query()}><button type="button" class="sac-search-x" aria-label="Clear search" onClick={() => props.setQuery('')}><X size={14} /></button></Show>
    <Show when={!props.query()}><kbd aria-hidden="true">/</kbd></Show></label>;
  const fit = <button type="button" class="sac-btn" aria-label="Fit to screen" data-tip="Fit (F)" onClick={props.fit}><Maximize2 size={15} aria-hidden="true" /><span class="sac-lbl">Fit</span></button>;
  return <header class="sac-bar" data-phone={props.phone ? '' : undefined}>
    <div class="sac-title"><Network size={18} aria-hidden="true" /><h2>Agent canvas</h2></div>
    <Show when={!props.phone}>
      {search}
      <StatusChips {...props} /><ChatsChip {...props} /><ProjectMenu {...props} />
      <DensityToggle density={props.density} set={props.setDensity} />
      {fit}
    </Show>
    <Show when={props.phone}>
      {search}
      <button type="button" class="sac-btn sac-filter-btn" aria-label="Filters" data-active={filterActive(props.filter()) ? '' : undefined} onClick={props.openFilters}><ListFilter size={16} aria-hidden="true" /></button>
      {fit}
    </Show>
    <button type="button" class="sac-icon sac-close" aria-label="Close canvas" data-tip="Close (Esc)" onClick={props.onClose}><X size={18} /></button>
  </header>;
}

/** Zoom out / level / zoom in, docked in the stage's bottom-right corner so the toolbar keeps its width for the filters. */
export function ZoomDock(props: { scale: Accessor<number>; zoom: (factor: number) => void }) {
  return <div class="sac-zoom" role="group" aria-label="Zoom">
    <button type="button" class="sac-icon" aria-label="Zoom out" data-tip="Zoom out (−)" onClick={() => props.zoom(1 / 1.2)}><Minus size={16} /></button>
    <output class="sac-scale" aria-label="Zoom level">{Math.round(props.scale() * 100)}%</output>
    <button type="button" class="sac-icon" aria-label="Zoom in" data-tip="Zoom in (+)" onClick={() => props.zoom(1.2)}><Plus size={16} /></button>
  </div>;
}

/** Phone: every filter in one sheet body. */
export function FilterPanel(props: FilterProps & { onClear: () => void }) {
  return <div class="sac-filters">
    <MenuLabel>Status</MenuLabel><StatusChips {...props} />
    <MenuLabel>Show</MenuLabel><div class="sac-chips"><ChatsChip {...props} /></div>
    <MenuLabel>Projects</MenuLabel>
    <div class="sac-proj-list" role="group" aria-label="Projects">
      <For each={props.groups()}>{(g) => <button type="button" class="sac-proj" role="checkbox" aria-checked={!props.filter().hidden.includes(g.key)} onClick={() => { const f = props.filter(); props.set({ ...f, hidden: f.hidden.includes(g.key) ? f.hidden.filter((k) => k !== g.key) : [...f.hidden, g.key] }); }}>
        <span class="sac-proj-box" aria-hidden="true"><Check size={14} /></span>{g.name}</button>}</For>
    </div>
    <button type="button" class="sac-btn" disabled={!filterActive(props.filter())} onClick={props.onClear}>Clear filters</button>
  </div>;
}
