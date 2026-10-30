/**
 * Model picker (WP-A14, Z-69 + Z-76): unipi's sheet kept, made wider, with
 * one search over Single **and** the Fusion columns (QA#4), a sentence-case
 * "Now …" line (QA#3a), compact right-aligned footer buttons (QA#3c),
 * Vision / context badges on the rows and the current row checked. The
 * thinking levels moved out to the composer's brain chip (Z-71).
 *
 * Data is the host's: `get_available_models` for the catalogue, plus
 * `unipi.fusion`'s `fusion.status` / `fusion.set-pair` when the extension is
 * enabled (the Fusion tab is hidden otherwise — no dead control). Applying
 * sends `set_model` (or the fusion pair), and a success toast says what
 * changed (Z-72).
 *
 * Entry points (Z-70): the composer's model chip, Ctrl/Cmd+M and `/model`.
 * Dismissing sends nothing.
 */
import { createEffect, createMemo, createResource, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { Check, ChevronDown, ChevronRight, X } from 'lucide-solid';
import { BoardSheet } from '../board/BoardSheet';
import { layout } from '../../lib/layout';
import { wrapTab } from '../../lib/focusTrap';
import { sessions, type SessionHandle } from '../../lib/session/store';
import { toasts } from '../../lib/toasts';
import { liveSource } from '../session/modalData';
import { fetchFusion, fusionAvailable, setFusionActive, type FusionState } from '../../lib/model/fusion';
import {
  columnModels,
  contextBadge,
  FUSION_EFFORTS,
  hasVision,
  modelKey,
  modelName,
  nextSinglePick,
  nowLabel,
  pairToast,
  pairUnchanged,
  parseModels,
  selectionOf,
  shortModelName,
  singleAction,
  singleModels,
  switchToast,
  type ModelInfo,
  type Selection,
} from '../../lib/model/picker';
import { closeModelPicker, modelPickerTarget, type ModelPickerTarget } from './target';
import { cycleKeyOf, cycleKeysOf, cycleLabel, toggleCycleKey } from '../../lib/model/cycle';
import '../session/session-modal.css';
import './model.css';

/** Rows rendered per column (the catalogue can hold hundreds). */
const LIST_CAP = 120;

const errText = (e: unknown): string => String((e as { message?: unknown })?.message ?? e);

type PickerData = { models: ModelInfo[]; fusion?: FusionState; cycle?: unknown };

/** Loads the catalogue, the Fusion pair (when enabled) and the cycle set (`enabledModels`). */
async function loadPicker(s: SessionHandle): Promise<PickerData> {
  const fusionOn = await fusionAvailable(s);
  const [modelsReply, fusion, settings] = await Promise.all([
    s.cmd<unknown>({ type: 'get_available_models' }).catch(() => undefined),
    fusionOn ? fetchFusion(s) : Promise.resolve(undefined),
    // WP-A25 (G-15 `/scoped-models`): pi's `enabledModels` — the cycle set.
    s.cmd<unknown>({ type: '_oino/get_settings', key: 'enabledModels' }).catch(() => undefined),
  ]);
  const cycle = (settings as { value?: unknown } | undefined)?.value;
  return { models: parseModels(modelsReply), ...(fusion ? { fusion } : {}), cycle };
}

/** Scrolls the chosen row of each Fusion column into view (often below its fold). */
export function revealPicked(root: HTMLElement): void {
  for (const list of root.querySelectorAll<HTMLElement>('.mp-col-list')) {
    const row = list.querySelector<HTMLElement>('[aria-current="true"]');
    if (!row) continue;
    const lr = list.getBoundingClientRect();
    const rr = row.getBoundingClientRect();
    if (rr.top < lr.top || rr.bottom > lr.bottom) list.scrollTop += rr.top - lr.top - (lr.height - rr.height) / 2;
  }
}

export function ModelPickerBody(props: { target: ModelPickerTarget }) {
  // ensure() subscribes; acquire() pins the session against the LRU while shown.
  const s = createMemo(() => sessions().ensure(props.target.hostId, props.target.sessionId));
  onCleanup(s().acquire());
  const [data] = createResource(liveSource(s), async (state) => (state === 'live' ? loadPicker(s()) : undefined));

  const models = () => data.latest?.models ?? [];
  const fusion = () => data.latest?.fusion;
  const byKey = createMemo(() => new Map(models().map((m) => [modelKey(m), m])));
  const current = (): Selection | undefined => selectionOf(s().state.model, fusion()?.active);
  const fusionOn = () => !!fusion();
  const activePair = () => {
    const c = current();
    return c?.kind === 'fusion' ? c : undefined;
  };
  const nameOf = (key: string | undefined): string => {
    if (!key) return '';
    const m = byKey().get(key);
    if (m) return modelName(m);
    const st = fusion()?.status;
    if (st?.leadKey === key) return st.leadName;
    if (st?.sidekickKey === key) return st.sidekickName;
    return key;
  };
  const now = () => nowLabel(current(), nameOf);

  // ── mode, search and pending picks ──────────────────────────────────
  const [mode, setMode] = createSignal<'single' | 'fusion'>('single');
  const [query, setQuery] = createSignal('');
  const [pick, setPick] = createSignal<string | undefined>();
  const [pendingLead, setPendingLead] = createSignal<string | undefined>();
  const [pendingSidekick, setPendingSidekick] = createSignal<string | undefined>();
  const [leadEffort, setLeadEffort] = createSignal('medium');
  const [sidekickEffort, setSidekickEffort] = createSignal('medium');
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  // WP-A25: the cycle-set block (`/scoped-models` opens it).
  const [cycleOpen, setCycleOpen] = createSignal(props.target.focus === 'cycle');
  const [cycle, setCycle] = createSignal<{ exact: string[]; custom: string[] }>({ exact: [], custom: [] });

  // The first load opens the tab (and the effort pills) the running selection implies.
  let settled = false;
  createEffect(() => {
    if (settled || !data.latest) return;
    settled = true;
    setCycle(cycleKeysOf(data.latest.cycle));
    setMode(current()?.kind === 'fusion' ? 'fusion' : 'single');
    const pair = activePair();
    const st = fusion()?.status;
    setLeadEffort(pair?.leadEffort ?? st?.leadEffort ?? 'medium');
    setSidekickEffort(pair?.sidekickEffort ?? st?.sidekickEffort ?? 'medium');
  });
  const chooseMode = (next: 'single' | 'fusion') => {
    settled = true;
    setMode(next);
    setError(null);
  };

  const rows = createMemo(() => singleModels(models(), current(), query()));
  const colRows = createMemo(() => columnModels(models(), query()));
  const leadKey = () => pendingLead() ?? activePair()?.lead ?? fusion()?.status?.leadKey ?? (models()[0] ? modelKey(models()[0]!) : undefined);
  const sidekickKey = () =>
    pendingSidekick() ?? activePair()?.sidekick ?? fusion()?.status?.sidekickKey ?? (models()[1] ? modelKey(models()[1]!) : models()[0] ? modelKey(models()[0]!) : undefined);
  const action = () => singleAction(current(), pick());
  const unchanged = () => pairUnchanged(current(), leadKey(), sidekickKey(), leadEffort(), sidekickEffort());

  const run = async (op: () => Promise<void>) => {
    if (busy()) return;
    setBusy(true);
    setError(null);
    try {
      await op();
      closeModelPicker();
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const applySingle = () => {
    const key = pick();
    if (!key) return;
    const from = now();
    void run(async () => {
      if (fusionOn() && fusion()?.active) {
        await setFusionActive(s(), { kind: 'single', model: key });
      } else {
        const slash = key.indexOf('/');
        await s().cmd({ type: 'set_model', provider: key.slice(0, slash), modelId: key.slice(slash + 1) });
      }
      toasts().push(switchToast(from, shortModelName(nameOf(key))), 'success');
    });
  };
  const applyPair = () => {
    const lead = leadKey();
    const sidekick = sidekickKey();
    if (!lead || !sidekick) return;
    const from = now();
    void run(async () => {
      await setFusionActive(s(), { kind: 'fusion', lead, sidekick, leadEffort: leadEffort(), sidekickEffort: sidekickEffort() });
      toasts().push(pairToast(from, nowLabel({ kind: 'fusion', lead, sidekick }, nameOf)), 'success');
    });
  };

  /** Flip one model in the cycle set: `_oino/set_setting{key:'enabledModels'}` (global). */
  const setInCycle = async (key: string, on: boolean) => {
    const cur = cycle();
    const nextExact = on ? [...new Set([...cur.exact, key])] : cur.exact.filter((k) => k !== key);
    const value = toggleCycleKey(cur.exact, cur.custom, key, on);
    setError(null);
    try {
      await s().cmd({ type: '_oino/set_setting', key: 'enabledModels', value: value ?? null, scope: 'global' });
      setCycle({ exact: nextExact, custom: cur.custom });
    } catch (e) {
      setError(errText(e));
    }
  };
  const cycleOf = (m: ModelInfo) => cycle().exact.includes(cycleKeyOf(m.provider, m.id));

  return (
    <div class="mp" data-testid="model-picker">
      <p class="mp-now" data-testid="model-picker-now">
        <span class="mp-now-word">Now</span>
        <span class="mp-now-value">{now()}</span>
      </p>

      <Show when={fusionOn()}>
        <div class="sm-tabs" role="tablist" aria-label="Model mode">
          <button type="button" role="tab" aria-selected={mode() === 'single'} class="sm-tab" onClick={() => chooseMode('single')} data-testid="model-tab-single">
            Single
          </button>
          <button type="button" role="tab" aria-selected={mode() === 'fusion'} class="sm-tab" onClick={() => chooseMode('fusion')} data-testid="model-tab-fusion">
            Fusion
          </button>
        </div>
      </Show>

      <Show when={mode() === 'fusion' || models().length > 6}>
        <input
          class="mp-search"
          type="search"
          placeholder={mode() === 'fusion' ? `Search both columns (${models().length} models)` : `Search all ${models().length} models`}
          aria-label="Search models"
          value={query()}
          onInput={(e) => setQuery(e.currentTarget.value)}
          data-testid="model-search"
        />
      </Show>

      <Show
        when={mode() === 'fusion'}
        fallback={
          <Show
            when={!data.loading || data.latest}
            fallback={<div class="mp-skeleton"><span /><span /><span /></div>}
          >
            <div class="mp-list" data-testid="model-picker-list">
              <For each={rows().slice(0, LIST_CAP)} fallback={<p class="mp-empty">No model matches “{query().trim()}”.</p>}>
                {(m) => {
                  const key = modelKey(m);
                  const isCurrent = () => current()?.kind === 'single' && (current() as { key: string }).key === key;
                  const picked = () => pick() === key;
                  return (
                    <button
                      type="button"
                      class="mp-row"
                      aria-current={isCurrent() ? 'true' : undefined}
                      aria-pressed={picked() ? 'true' : 'false'}
                      data-picked={picked() ? '' : undefined}
                      onClick={() => {
                        setError(null);
                        setPick(nextSinglePick(current(), pick(), key));
                      }}
                    >
                      <span class="mp-row-main">
                        <span class="mp-row-name">{modelName(m)}</span>
                        <span class="mp-row-sub">
                          {m.provider}/{m.id}
                        </span>
                      </span>
                      <Show when={hasVision(m)}>
                        <span class="mp-badge" title="Accepts images">
                          Vision
                        </span>
                      </Show>
                      <Show when={contextBadge(m)}>{(badge) => <span class="mp-badge mp-ctx">{badge()}</span>}</Show>
                      <Show when={isCurrent()}>
                        <Check size={16} class="mp-check" aria-label="Current model" />
                      </Show>
                    </button>
                  );
                }}
              </For>
            </div>
          </Show>
        }
      >
        <Show when={!data.loading || data.latest} fallback={<div class="mp-skeleton"><span /><span /><span /></div>}>
          <div class="mp-fusion" ref={(el) => queueMicrotask(() => revealPicked(el))} data-testid="model-picker-fusion">
            <div class="mp-col">
              <p class="mp-label">Lead</p>
              <div class="mp-list mp-col-list">
                <For each={colRows().slice(0, LIST_CAP)} fallback={<p class="mp-empty">No model matches.</p>}>
                  {(m) => {
                    const key = modelKey(m);
                    return (
                      <button
                        type="button"
                        class="mp-row"
                        aria-current={leadKey() === key ? 'true' : undefined}
                        onClick={() => {
                          setError(null);
                          setPendingLead(key);
                        }}
                      >
                        <span class="mp-row-name">{shortModelName(modelName(m))}</span>
                        <Show when={leadKey() === key}>
                          <Check size={16} class="mp-check" />
                        </Show>
                      </button>
                    );
                  }}
                </For>
              </div>
            </div>
            <div class="mp-col">
              <p class="mp-label">Sidekick</p>
              <div class="mp-list mp-col-list">
                <For each={colRows().slice(0, LIST_CAP)} fallback={<p class="mp-empty">No model matches.</p>}>
                  {(m) => {
                    const key = modelKey(m);
                    return (
                      <button
                        type="button"
                        class="mp-row"
                        aria-current={sidekickKey() === key ? 'true' : undefined}
                        onClick={() => {
                          setError(null);
                          setPendingSidekick(key);
                        }}
                      >
                        <span class="mp-row-name">{shortModelName(modelName(m))}</span>
                        <Show when={sidekickKey() === key}>
                          <Check size={16} class="mp-check" />
                        </Show>
                      </button>
                    );
                  }}
                </For>
              </div>
            </div>
          </div>
          <div class="mp-effort" role="radiogroup" aria-label="Lead effort">
            <span class="mp-label">Lead effort</span>
            <For each={FUSION_EFFORTS}>
              {(level) => (
                <button type="button" role="radio" aria-checked={leadEffort() === level} class="mp-pill" onClick={() => setLeadEffort(level)}>
                  {level}
                </button>
              )}
            </For>
          </div>
          <div class="mp-effort" role="radiogroup" aria-label="Sidekick effort">
            <span class="mp-label">Sidekick effort</span>
            <For each={FUSION_EFFORTS}>
              {(level) => (
                <button type="button" role="radio" aria-checked={sidekickEffort() === level} class="mp-pill" onClick={() => setSidekickEffort(level)}>
                  {level}
                </button>
              )}
            </For>
          </div>
        </Show>
      </Show>

      <div class="mp-cycle" data-testid="model-picker-cycle">
        <button
          type="button"
          class="mp-cycle-toggle"
          aria-expanded={cycleOpen()}
          onClick={() => setCycleOpen((v) => !v)}
          data-testid="model-picker-cycle-toggle"
        >
          <Show when={cycleOpen()} fallback={<ChevronRight size={14} />}>
            <ChevronDown size={14} />
          </Show>
          <span>{cycleLabel(cycle().exact, cycle().custom, models().length)}</span>
        </button>
        <Show when={cycleOpen()}>
          <div class="mp-cycle-body">
            <p class="mp-hint">The set Ctrl+P cycles in the terminal (pi’s <code>enabledModels</code>). Every model stays selectable here.</p>
            <div class="mp-cycle-list" role="group" aria-label="Cycle set">
              <For each={models().slice(0, LIST_CAP)}>
                {(m) => {
                  const key = cycleKeyOf(m.provider, m.id);
                  return (
                    <button
                      type="button"
                      class="mp-cycle-row"
                      aria-pressed={cycleOf(m) ? 'true' : 'false'}
                      data-testid={`cycle-${key}`}
                      onClick={() => void setInCycle(key, !cycleOf(m))}
                    >
                      <span class="mp-cycle-box" aria-hidden="true">
                        <Show when={cycleOf(m)}>
                          <Check size={12} />
                        </Show>
                      </span>
                      <span class="truncate">{modelName(m)}</span>
                      <span class="mp-cycle-key">{key}</span>
                    </button>
                  );
                }}
              </For>
            </div>
            <Show when={cycle().custom.length > 0}>
              <p class="mp-hint" data-testid="model-picker-cycle-custom">
                Custom patterns (kept as written): {cycle().custom.join(', ')}
              </p>
            </Show>
          </div>
        </Show>
      </div>

      <Show when={error()}>
        <p class="mp-error" role="alert" data-testid="model-picker-error">
          {error()}
        </p>
      </Show>

      <div class="mp-foot" data-testid="model-picker-foot">
        <Show when={mode() === 'single'}>
          <Show
            when={action()}
            fallback={<p class="mp-hint">{current()?.kind === 'fusion' ? 'Fusion is on. Pick a model to switch to it.' : 'Pick a model to switch.'}</p>}
          >
            {(act) => (
              <button type="button" class="pair-primary mp-apply" disabled={busy()} aria-busy={busy() ? 'true' : undefined} onClick={applySingle}>
                <Show when={busy()}>
                  <span class="mp-spinner" aria-hidden="true" />
                </Show>
                {busy() ? 'Switching…' : act().label}
              </button>
            )}
          </Show>
        </Show>
        <Show when={mode() === 'fusion' && fusionOn()}>
          <button type="button" class="pair-primary mp-apply" disabled={busy() || !leadKey() || !sidekickKey() || unchanged()} aria-busy={busy() ? 'true' : undefined} onClick={applyPair}>
            <Show when={busy()}>
              <span class="mp-spinner" aria-hidden="true" />
            </Show>
            {busy() ? 'Applying…' : unchanged() ? 'Current pair' : activePair() ? 'Update pair' : 'Use this pair'}
          </button>
        </Show>
      </div>
    </div>
  );
}

/** The picker: a wide centred dialog on the desktop (Z-69), the sheet on the phone. */
export function ModelPicker(props: { target: ModelPickerTarget }) {
  const desktop = () => layout().desktop();
  onMount(() => {
    if (!desktop()) return; // the sheet's pushBack owns Esc / Back on the phone
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && modelPickerTarget() && !e.defaultPrevented) {
        e.preventDefault();
        closeModelPicker();
      }
    };
    window.addEventListener('keydown', onKey, true);
    onCleanup(() => window.removeEventListener('keydown', onKey, true));
  });

  return (
    <Show
      when={desktop()}
      fallback={
        <BoardSheet open onClose={closeModelPicker} label="Model" title="Model" testId="model-picker-sheet" class="model-picker-sheet">
          <ModelPickerBody target={props.target} />
        </BoardSheet>
      }
    >
      <div class="sm-overlay" onClick={(e) => e.target === e.currentTarget && closeModelPicker()} data-testid="model-picker-overlay">
        <div
          role="dialog"
          aria-modal="true"
          aria-label="Model"
          class="sm-dialog model-picker-dialog"
          tabindex="-1"
          ref={(el) => queueMicrotask(() => el.focus({ preventScroll: true }))}
          onKeyDown={(e: KeyboardEvent) => {
            if (wrapTab(e, e.currentTarget as HTMLElement)) e.stopPropagation();
          }}
        >
          <div class="sm-head">
            <span class="sm-title">
              <h2>Model</h2>
            </span>
            <button type="button" class="ghost-icon-button" aria-label="Close" onClick={closeModelPicker} data-testid="model-picker-close">
              <X size={20} />
            </button>
          </div>
          <div class="sm-body mp-body">
            <ModelPickerBody target={props.target} />
          </div>
        </div>
      </div>
    </Show>
  );
}

/** Mounted once by the shell: renders the picker for the open target. */
export function ModelPickerLayer() {
  return (
    <Show when={modelPickerTarget()} keyed>
      {(target) => <ModelPicker target={target} />}
    </Show>
  );
}
