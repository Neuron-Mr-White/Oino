/**
 * The one model list (F-02 / G-06): every model picker (the one model dialog, `ModelPopover.tsx` — the
 * in-chat chip, the New-chat chip, the settings fields) renders the SAME rows — Suggested group on top, then all usable chat
 * models, then the collapsed "Unavailable" group with the provider's reason.
 * The data comes from `modelView` (availability + readiness + kind filter +
 * curated ranking); this is only markup. Styles: `model.css` (`.mp-*`).
 */
import { createMemo, createSignal, For, Show } from 'solid-js';
import { Check, ChevronDown, ChevronRight } from 'lucide-solid';
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

/** One row: display name, route sub-line, badges; provider · id in the tooltip. */
export function ModelRow(props: {
  m: ModelInfo;
  current: boolean;
  picked?: boolean;
  startup?: boolean;
  lead?: boolean;
  sidekick?: boolean;
  effort?: string;
  onSelect: () => void;
}) {
  const key = modelKey(props.m);
  const name = modelName(props.m);
  return (
    <button
      type="button"
      role="menuitemradio"
      class="mp-row"
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
}

/** Suggested / All models / Unavailable, as one list. */
export function ModelList(props: {
  view: ModelView;
  query: string;
  /** The running model's key (checked). */
  currentKey?: string;
  pickedKey?: string;
  startupKey?: string;
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
