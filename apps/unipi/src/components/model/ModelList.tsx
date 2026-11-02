/**
 * The one model list (F-02 / G-06): every model picker (the one model dialog, `ModelPopover.tsx` — the
 * in-chat chip, the New-chat chip, the settings fields) renders the SAME rows — Suggested group on top, then all usable chat
 * models, then the collapsed "Unavailable" group with the provider's reason.
 * The data comes from `modelView` (availability + readiness + kind filter +
 * curated ranking); this is only markup. Styles: `model.css` (`.mp-*`).
 */
import { createMemo, createSignal, For, Show, type JSX } from 'solid-js';
import { Check, ChevronDown, ChevronRight, Crown, Ellipsis, Minus, Plus, Rocket, Users } from 'lucide-solid';
import { AnchoredMenu } from '../board/AnchoredMenu';
import { contextBadge, hits as modelHits, modelKey, modelName, type ModelInfo, type Selection } from '../../lib/model/picker';
import type { FusionCurated } from '../../lib/model/fusion';
import type { Readiness } from '../../lib/model/readiness';
import { splitAvailability } from '../../lib/model/availability';
import { thinkingLabel } from '../../lib/model/thinking';
import { ModalityIcons } from './ModalityIcons';
import { groupedModels, routeLabel, rowTitle, type SuggestedRow } from './suggested';

/** Rows rendered per list: the whole catalogue (P0-10 — no 120 cap); a guard only. */
export const LIST_CAP = 2000;

export type ModelView = {
  /** Chat models of working providers (the running model always stays). */
  usable: ModelInfo[];
  unavailable: Array<{ model: ModelInfo; reason: string }>;
  suggested: SuggestedRow[];
  rest: ModelInfo[];
};

/** The shared data rule: availability split, then the curated grouping, then the search. */
export function modelView(
  all: readonly ModelInfo[],
  readiness: Readiness | undefined,
  curated: FusionCurated | undefined,
  current: Selection | undefined,
  query: string,
  startup?: string,
): ModelView {
  const curKey = current?.kind === 'single' ? current.key : undefined;
  const curProvider = curKey ? curKey.slice(0, curKey.indexOf('/')) : undefined;
  const keepKey = curKey && (!curProvider || !readiness?.known || readiness.providerReady(curProvider)) ? curKey : undefined;
  const split = splitAvailability(all, readiness, keepKey);
  const grouped = groupedModels(split.usable, curated, current, query, startup);
  const q = query.trim().toLowerCase();
  return {
    usable: split.usable,
    unavailable: q ? split.unavailable.filter((u) => modelHits(u.model, q)) : split.unavailable,
    ...grouped,
  };
}

/**
 * The per-row ⋯ menu's actions (owner: "each row gets an ellipsis"): Fusion
 * roles (only when the extension is available), cycle-set membership and
 * "Use at startup". A context without actions (Cron, settings fields) shows
 * no ⋯ at all.
 */
export type RowActions = {
  fusion?: {
    /** The pair's current lead / sidekick key (checked in the menu). */
    lead?: string;
    sidekick?: string;
    setLead: (m: ModelInfo) => void;
    setSidekick: (m: ModelInfo) => void;
  };
  cycle?: { has: (key: string) => boolean; toggle: (m: ModelInfo, on: boolean) => void };
  startup?: { set: (m: ModelInfo) => void };
};

/** Keys that open a row's ⋯ menu from the row itself (→, Shift+F10, the context-menu key). */
export const opensRowMenu = (e: KeyboardEvent): boolean =>
  !e.altKey && !e.ctrlKey && !e.metaKey && (e.key === 'ArrowRight' || e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey));

/** The ⋯ menu of one row (portalled, anchored to the ⋯ button; Esc closes back to the row). */
function RowMenu(props: { m: ModelInfo; name: string; anchor: DOMRect; startup?: boolean; actions: RowActions; onDismiss: () => void }) {
  const key = modelKey(props.m);
  const run = (fn: () => void) => {
    props.onDismiss();
    fn();
  };
  const Item = (p: { label: string; icon: JSX.Element; checked?: boolean; testId: string; onSelect: () => void }) => (
    <button
      type="button"
      role={p.checked === undefined ? 'menuitem' : 'menuitemcheckbox'}
      aria-checked={p.checked === undefined ? undefined : p.checked}
      class="recent-popover-row board-menu-row mp-menu-row"
      data-testid={p.testId}
      onClick={() => run(p.onSelect)}
    >
      <span class="board-menu-icon">{p.checked ? <Check size={16} /> : p.icon}</span>
      <span class="min-w-0 flex-1 truncate text-left">{p.label}</span>
    </button>
  );
  const f = () => props.actions.fusion;
  const c = () => props.actions.cycle;
  const inCycle = () => !!c()?.has(key);
  return (
    <AnchoredMenu anchor={props.anchor} label={`${props.name} options`} width={236} alignEnd class="mp-row-menu" onDismiss={props.onDismiss}>
      {/* Esc / Tab close the menu only — the dialog under it (whose key handler the portal's events
          still reach through Solid's delegation) stays open; focus goes back to the row. */}
      <div
        data-testid="model-row-menu"
        data-key={key}
        onKeyDown={(e) => {
          if (e.key !== 'Escape' && e.key !== 'Tab') return;
          e.preventDefault();
          e.stopPropagation();
          props.onDismiss();
        }}
      >
        <Show when={f()}>
          {(fu) => (
            <>
              <Item
                label="Set as Fusion lead"
                icon={<Crown size={16} />}
                checked={fu().lead === key}
                testId="model-menu-lead"
                onSelect={() => fu().lead !== key && fu().setLead(props.m)}
              />
              <Item
                label="Set as Fusion sidekick"
                icon={<Users size={16} />}
                checked={fu().sidekick === key}
                testId="model-menu-sidekick"
                onSelect={() => fu().sidekick !== key && fu().setSidekick(props.m)}
              />
              <div class="board-menu-sep" role="separator" />
            </>
          )}
        </Show>
        <Show when={c()}>
          {(cy) => (
            <Item
              label={inCycle() ? 'Remove from cycle set' : 'Add to cycle set'}
              icon={inCycle() ? <Minus size={16} /> : <Plus size={16} />}
              testId="model-menu-cycle"
              onSelect={() => cy().toggle(props.m, !inCycle())}
            />
          )}
        </Show>
        <Show when={props.actions.startup}>
          {(st) => (
            <Item
              label="Use at startup"
              icon={<Rocket size={16} />}
              checked={!!props.startup}
              testId="model-menu-startup"
              onSelect={() => !props.startup && st().set(props.m)}
            />
          )}
        </Show>
      </div>
    </AnchoredMenu>
  );
}

/** One row: display name, route sub-line, badges; provider · id in the tooltip. */
export function ModelRow(props: {
  m: ModelInfo;
  current: boolean;
  picked?: boolean;
  startup?: boolean;
  lead?: boolean;
  sidekick?: boolean;
  effort?: string;
  actions?: RowActions;
  onSelect: () => void;
}) {
  const key = modelKey(props.m);
  const name = modelName(props.m);
  const [menu, setMenu] = createSignal<DOMRect | undefined>();
  let rowEl: HTMLButtonElement | undefined;
  let moreEl: HTMLButtonElement | undefined;
  /** Opens the ⋯ menu; focus sits on the row first so Esc lands back on it. */
  const openMenu = () => {
    if (!props.actions || !moreEl) return;
    rowEl?.focus({ preventScroll: true });
    setMenu(moreEl.getBoundingClientRect());
  };
  const row = (
    <button
      type="button"
      role="menuitemradio"
      class="mp-row"
      ref={rowEl}
      aria-haspopup={props.actions ? 'menu' : undefined}
      onKeyDown={(e) => {
        if (!props.actions || !opensRowMenu(e)) return;
        e.preventDefault();
        e.stopPropagation();
        openMenu();
      }}
      onContextMenu={(e) => {
        if (!props.actions) return;
        e.preventDefault();
        openMenu();
      }}
      title={rowTitle(name, props.m)}
      aria-checked={props.current}
      aria-current={props.current ? 'true' : undefined}
      data-picked={props.picked ? '' : undefined}
      data-key={key}
      onClick={props.onSelect}
    >
      <span class="mp-row-main">
        <span class="mp-row-name">{name}</span>
        <span class="mp-row-sub">{routeLabel(props.m)}</span>
      </span>
      <Show when={props.lead}>
        <span class="mp-badge mp-role" title="Curated Fusion lead">Lead</span>
      </Show>
      <Show when={props.sidekick}>
        <span class="mp-badge mp-role" title="Curated Fusion sidekick">Sidekick</span>
      </Show>
      <Show when={props.effort}>
        {(e) => (
          <span class="mp-badge mp-effort-badge" title={`Curated thinking: ${thinkingLabel(e())} (applied with the switch)`}>
            {thinkingLabel(e())}
          </span>
        )}
      </Show>
      <Show when={props.startup}>
        <span class="mp-badge mp-startup" title="New chats start on this model">Startup</span>
      </Show>
      <ModalityIcons m={props.m} />
      <Show when={contextBadge(props.m)}>{(badge) => <span class="mp-badge mp-ctx" title="Context window">{badge()}</span>}</Show>
      <Show when={props.current}>
        <Check size={16} class="mp-check" aria-label="Current model" />
      </Show>
    </button>
  );
  return (
    <Show when={props.actions} fallback={row}>
      {(actions) => (
        <div class="mp-row-wrap" data-menu-open={menu() ? '' : undefined}>
          {row}
          <button
            type="button"
            class="mp-more"
            tabindex="-1"
            ref={moreEl}
            aria-label={`More options for ${name}`}
            aria-haspopup="menu"
            aria-expanded={!!menu()}
            title="More options (→)"
            onClick={(e) => {
              e.stopPropagation();
              if (menu()) setMenu(undefined);
              else openMenu();
            }}
            data-testid="model-row-more"
          >
            <Ellipsis size={16} />
          </button>
          <Show when={menu()}>
            {(anchor) => <RowMenu m={props.m} name={name} anchor={anchor()} startup={props.startup} actions={actions()} onDismiss={() => setMenu(undefined)} />}
          </Show>
        </div>
      )}
    </Show>
  );
}

/** Suggested / All models / Unavailable, as one list. */
export function ModelList(props: {
  view: ModelView;
  query: string;
  /** The running model's key (checked). */
  currentKey?: string;
  pickedKey?: string;
  startupKey?: string;
  /** The ⋯ menu's actions (none: no ⋯ on the rows). */
  actions?: RowActions;
  onSelect: (m: ModelInfo, isCurrent: boolean) => void;
}) {
  const [showUnavailable, setShowUnavailable] = createSignal(false);
  const unavailableProviders = createMemo(() => {
    const by = new Map<string, { reason: string; count: number }>();
    for (const u of props.view.unavailable) {
      const e = by.get(u.model.provider);
      if (e) e.count += 1;
      else by.set(u.model.provider, { reason: u.reason, count: 1 });
    }
    return [...by.values()];
  });
  const row = (m: ModelInfo, extra?: Partial<SuggestedRow>) => {
    const key = modelKey(m);
    return (
      <ModelRow
        m={m}
        current={props.currentKey === key}
        picked={props.pickedKey === key}
        startup={props.startupKey === key}
        lead={extra?.lead}
        sidekick={extra?.sidekick}
        effort={extra?.effort}
        actions={props.actions}
        onSelect={() => props.onSelect(m, props.currentKey === key)}
      />
    );
  };
  return (
    <div class="mp-list" data-testid="model-picker-list">
      <Show when={props.view.suggested.length > 0}>
        <p class="mp-group" data-testid="model-group-suggested">Suggested</p>
        <For each={props.view.suggested}>{(s) => row(s.model, s)}</For>
        <Show when={props.view.rest.length > 0}>
          <p class="mp-group" data-testid="model-group-all">All models</p>
        </Show>
      </Show>
      <For
        each={props.view.rest.slice(0, LIST_CAP)}
        fallback={
          <Show when={props.view.suggested.length === 0 && props.view.usable.length > 0}>
            <p class="mp-empty">No model matches “{props.query.trim()}”.</p>
          </Show>
        }
      >
        {(m) => row(m)}
      </For>
      <Show when={props.view.unavailable.length > 0}>
        <button
          type="button"
          role="menuitem"
          class="mp-group mp-unavail-toggle"
          aria-expanded={showUnavailable()}
          onClick={() => setShowUnavailable((v) => !v)}
          data-testid="model-group-unavailable"
        >
          <Show when={showUnavailable()} fallback={<ChevronRight size={13} />}>
            <ChevronDown size={13} />
          </Show>
          Unavailable ({props.view.unavailable.length})
        </button>
        <Show when={showUnavailable()}>
          <For each={unavailableProviders()}>
            {(p) => (
              <p class="mp-hint mp-unavail-note">
                {p.reason} · {p.count} model{p.count === 1 ? '' : 's'}
              </p>
            )}
          </For>
          <For each={props.view.unavailable.slice(0, 200)}>
            {(u) => (
              <button type="button" class="mp-row" disabled aria-disabled="true" title={`${modelName(u.model)}\n${u.reason}`} data-testid="model-row-unavailable">
                <span class="mp-row-main">
                  <span class="mp-row-name">{modelName(u.model)}</span>
                  <span class="mp-row-sub">{routeLabel(u.model)}</span>
                </span>
              </button>
            )}
          </For>
        </Show>
      </Show>
    </div>
  );
}
