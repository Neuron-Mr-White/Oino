/**
 * Pane tab strip (Z-5, WP-A3), shared by the side pane and the bottom panel:
 * icon + title (+ badge) per tab, drag to reorder (order persisted per
 * project), middle-click or ✕ closes, `+` opens the launcher, the overview
 * button opens the tab overview (search, close variants, recently closed).
 * `lead` renders before the tabs (full view's "Chat" tab, sidebar reveal),
 * `tools` after the overview button (full view / hide pane).
 */
import { createSignal, For, Show, type JSX } from 'solid-js';
import { ChevronDown, Plus, X } from 'lucide-solid';
import { paneKind, tabTitle } from '../../lib/panes/kinds';
import type { PaneTab } from '../../lib/panes';

export function TabStrip(props: {
  label: string;
  tabs: PaneTab[];
  activeId?: string;
  /** No tab is shown as active (full view's Chat tab is). */
  noneActive?: boolean;
  badge?: (tab: PaneTab) => string | number | undefined;
  onActivate: (id: string) => void;
  onClose: (id: string) => void;
  onMove: (id: string, beforeId?: string) => void;
  onLauncher: (anchor: DOMRect) => void;
  onOverview: (anchor: DOMRect) => void;
  lead?: JSX.Element;
  tools?: JSX.Element;
  testId?: string;
}) {
  const [dragId, setDragId] = createSignal<string | undefined>();
  const [dropAt, setDropAt] = createSignal<{ id: string; after: boolean } | undefined>();
  const onKey = (e: KeyboardEvent, index: number) => {
    const step = e.key === 'ArrowRight' ? 1 : e.key === 'ArrowLeft' ? -1 : 0;
    if (!step) return;
    const next = props.tabs[(index + step + props.tabs.length) % props.tabs.length];
    if (!next) return;
    e.preventDefault();
    props.onActivate(next.id);
    (e.currentTarget as HTMLElement).parentElement?.querySelector<HTMLElement>(`[data-tab-id="${next.id}"]`)?.focus();
  };
  return (
    <div class="tab-strip" data-testid={props.testId}>
      {props.lead}
      <div class="tab-list" role="tablist" aria-label={props.label}>
        <For each={props.tabs}>
          {(tab, i) => {
            const kind = () => paneKind(tab.kind);
            const active = () => !props.noneActive && tab.id === props.activeId;
            const Icon = kind()?.icon;
            return (
              <div
                class="pane-tab"
                role="tab"
                tabindex={active() ? 0 : -1}
                aria-selected={active()}
                data-tab-id={tab.id}
                data-kind={tab.kind}
                data-drop={dropAt()?.id === tab.id ? (dropAt()!.after ? 'after' : 'before') : undefined}
                data-dragging={dragId() === tab.id ? '' : undefined}
                title={tabTitle(tab)}
                draggable={true}
                onClick={() => props.onActivate(tab.id)}
                onKeyDown={(e) => onKey(e, i())}
                onAuxClick={(e) => {
                  if (e.button !== 1) return;
                  e.preventDefault();
                  props.onClose(tab.id);
                }}
                onMouseDown={(e) => e.button === 1 && e.preventDefault()}
                onDragStart={(e) => {
                  setDragId(tab.id);
                  e.dataTransfer?.setData('text/x-pane-tab', tab.id);
                  if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
                }}
                onDragOver={(e) => {
                  if (!dragId() || dragId() === tab.id) return;
                  e.preventDefault();
                  const r = e.currentTarget.getBoundingClientRect();
                  setDropAt({ id: tab.id, after: e.clientX > r.left + r.width / 2 });
                }}
                onDragLeave={() => setDropAt((d) => (d?.id === tab.id ? undefined : d))}
                onDrop={(e) => {
                  e.preventDefault();
                  const from = dragId();
                  const at = dropAt();
                  if (from && at) {
                    const idx = props.tabs.findIndex((t) => t.id === at.id);
                    const before = at.after ? props.tabs[idx + 1]?.id : at.id;
                    props.onMove(from, before === from ? undefined : before);
                  }
                  setDragId(undefined);
                  setDropAt(undefined);
                }}
                onDragEnd={() => {
                  setDragId(undefined);
                  setDropAt(undefined);
                }}
              >
                <Show when={Icon}>{(I) => { const C = I(); return <C size={14} strokeWidth={1.8} class="pane-tab-icon" />; }}</Show>
                <span class="pane-tab-title">{tabTitle(tab)}</span>
                <Show when={props.badge?.(tab)}>{(b) => <span class="pane-tab-badge">{b()}</span>}</Show>
                <button
                  type="button"
                  class="pane-tab-close"
                  aria-label={`Close ${tabTitle(tab)}`}
                  tabindex={-1}
                  onClick={(e) => {
                    e.stopPropagation();
                    props.onClose(tab.id);
                  }}
                >
                  <X size={13} />
                </button>
              </div>
            );
          }}
        </For>
      </div>
      <button type="button" class="shell-icon-btn tab-strip-btn" aria-label="Open a tab" title="Open a tab" onClick={(e) => props.onLauncher(e.currentTarget.getBoundingClientRect())} data-testid="pane-launcher-btn">
        <Plus size={16} />
      </button>
      <span class="tab-strip-fill" />
      <button type="button" class="shell-icon-btn tab-strip-btn" aria-label="All tabs" title="All tabs" onClick={(e) => props.onOverview(e.currentTarget.getBoundingClientRect())} data-testid="pane-overview-btn">
        <ChevronDown size={16} />
      </button>
      {props.tools}
    </div>
  );
}
