/**
 * Tab overview (Z-5, WP-A3): every tab of the current view with a search
 * box, Close / Close others / Close all (current view only), and Recently
 * closed (10 per scope) with Reopen (Cmd/Ctrl+Shift+T reopens the newest).
 */
import { createSignal, For, Show } from 'solid-js';
import { History, Search, X } from 'lucide-solid';
import type { PaneTab } from '../../lib/panes';
import { paneKind, tabTitle } from '../../lib/panes/kinds';
import { modKey } from '../../lib/shell';
import { AnchoredMenu, MenuRow, MenuSeparator } from '../board/AnchoredMenu';

/** Pure: tabs whose title or kind matches every word of the query. */
export function filterTabs(tabs: PaneTab[], query: string): PaneTab[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return tabs;
  return tabs.filter((t) => {
    const hay = `${tabTitle(t)} ${t.kind}`.toLowerCase();
    return words.every((w) => hay.includes(w));
  });
}

function TabIcon(props: { tab: PaneTab }) {
  const Icon = paneKind(props.tab.kind)?.icon;
  return Icon ? <Icon size={16} /> : null;
}

export function TabOverview(props: {
  anchor: DOMRect;
  tabs: PaneTab[];
  activeId?: string;
  closed: PaneTab[];
  onActivate: (id: string) => void;
  onClose: (id: string) => void;
  onCloseOthers: (id: string) => void;
  onCloseAll: () => void;
  onReopen: (id?: string) => void;
  onDismiss: () => void;
}) {
  const [query, setQuery] = createSignal('');
  const shown = () => filterTabs(props.tabs, query());
  const done = (fn: () => void) => () => (props.onDismiss(), fn());
  return (
    <AnchoredMenu anchor={props.anchor} alignEnd width={300} label="All tabs" onDismiss={props.onDismiss}>
      <div class="overview" data-testid="tab-overview">
        <label class="overview-search">
          <Search size={14} />
          <input
            type="search"
            placeholder="Search tabs"
            aria-label="Search tabs"
            value={query()}
            onInput={(e) => setQuery(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && shown()[0]) done(() => props.onActivate(shown()[0]!.id))();
            }}
            ref={(el) => queueMicrotask(() => el.focus())}
          />
        </label>
        <div class="overview-list">
          <For each={shown()} fallback={<p class="launcher-note">{props.tabs.length ? 'No tab matches.' : 'No tabs open.'}</p>}>
            {(tab) => (
              <div class="overview-row" data-active={tab.id === props.activeId ? '' : undefined}>
                <button type="button" role="menuitem" class="overview-open" onClick={done(() => props.onActivate(tab.id))}>
                  <TabIcon tab={tab} />
                  <span class="launcher-row-label">{tabTitle(tab)}</span>
                </button>
                <button type="button" class="overview-close" aria-label={`Close ${tabTitle(tab)}`} onClick={() => props.onClose(tab.id)}>
                  <X size={13} />
                </button>
              </div>
            )}
          </For>
        </div>
        <Show when={props.tabs.length > 0}>
          <MenuSeparator />
          <Show when={props.activeId}>
            <MenuRow label="Close tab" onSelect={done(() => props.onClose(props.activeId!))} />
            <MenuRow label="Close other tabs" disabled={props.tabs.length < 2} onSelect={done(() => props.onCloseOthers(props.activeId!))} />
          </Show>
          <MenuRow label="Close all tabs" onSelect={done(props.onCloseAll)} />
        </Show>
        <Show when={props.closed.length > 0}>
          <MenuSeparator />
          <div class="board-menu-label">Recently closed</div>
          <For each={props.closed}>
            {(tab) => <MenuRow icon={<TabIcon tab={tab} />} label={tabTitle(tab)} onSelect={done(() => props.onReopen(tab.id))} />}
          </For>
          <MenuRow icon={<History size={16} />} label="Reopen closed tab" hint={`${modKey()} ⇧ T`} onSelect={done(() => props.onReopen())} />
        </Show>
      </div>
    </AnchoredMenu>
  );
}
