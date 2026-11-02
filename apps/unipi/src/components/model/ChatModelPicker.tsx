/**
 * The in-chat model picker (WP-A14, Z-69 + Z-76): the ONE model dialog
 * (`ModelDialog` + `ModelPickerPanel`) with this chat's data — the old
 * "Model" dialog: Now line, Single / Fusion tabs (Fusion only when the
 * extension is on), one search over the list *and* both Fusion columns, a ⋯
 * menu per row (Fusion lead / sidekick, cycle set, startup), the "Cycle set"
 * filter chip, the scope line and "Use at startup". Entry points (Z-70): the
 * composer's model chip, Ctrl/Cmd+M, `/model` and `/scoped-models` (opens
 * with the Cycle set filter on).
 *
 * Data is the host's: `get_available_models` for the catalogue, plus
 * `unipi.fusion`'s `fusion.status` / `fusion.set-pair` when the extension is
 * enabled. A pick switches THIS chat only (`set_model`, or Fusion's single
 * pick while a pair runs, after an explicit "Turn Fusion off & switch"), and
 * a success toast says what changed (Z-72). Dismissing sends nothing.
 *
 * "Use at startup" writes the model a fresh session opens with (pi
 * `defaultProvider` / `defaultModel`, plus Fusion's `startup.model` when the
 * extension is on — it wins on a fresh session), like the TUI's Alt+Enter.
 *
 * P0-10: Fusion's curated preset ranks the list (Suggested group, Lead /
 * Sidekick badges, the curated effort applied with the pick) and puts the
 * Fusion columns' curated lists first.
 */
import { createEffect, createMemo, createResource, createSignal, For, onCleanup, Show } from 'solid-js';
import { Check } from 'lucide-solid';
import { sessions, type SessionHandle } from '../../lib/session/store';
import { toasts } from '../../lib/toasts';
import { liveSource } from '../session/modalData';
import { fetchFusion, fusionAvailable, setFusionActive, type FusionState } from '../../lib/model/fusion';
import {
  columnModels,
  FUSION_EFFORTS,
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
  switchToast,
  type ModelInfo,
  type Selection,
} from '../../lib/model/picker';
import { setChatModel } from '../../lib/model/chatModel';
import { hostReadiness, type Readiness } from '../../lib/model/readiness';
import { calmModelError } from '../../lib/model/availability';
import { closeModelPicker, modelPickerTarget, type ModelPickerTarget } from './target';
import { curatedFirst, rowTitle, startupKey } from './suggested';
import { LIST_CAP, modelView, type RowActions } from './ModelList';
import {
  createCycleSet,
  fusionStartup,
  ModeTabs,
  ModelDialog,
  ModelPickerPanel,
  PickerError,
  PickerSecondary,
  PickerSkeleton,
  writeStartupModel,
} from './ModelPopover';

const errText = (e: unknown): string => String((e as { message?: unknown })?.message ?? e);

type PickerData = { readiness?: Readiness; models: ModelInfo[]; fusion?: FusionState; cycle?: unknown; startup?: string };

const valueOf = (reply: unknown): unknown => (reply as { value?: unknown } | undefined)?.value;

/** Loads the catalogue, the Fusion pair (when enabled) and the cycle set (`enabledModels`). */
async function loadPicker(s: SessionHandle): Promise<PickerData> {
  const fusionOn = await fusionAvailable(s);
  const [modelsReply, fusion, settings, provider, model, fusionSettings, readiness] = await Promise.all([
    s.cmd<unknown>({ type: 'get_available_models' }).catch(() => undefined),
    fusionOn ? fetchFusion(s) : Promise.resolve(undefined),
    // WP-A25 (G-15 `/scoped-models`): pi's `enabledModels` — the cycle set.
    s.cmd<unknown>({ type: '_oino/get_settings', key: 'enabledModels' }).catch(() => undefined),
    s.cmd<unknown>({ type: '_oino/get_settings', key: 'defaultProvider' }).catch(() => undefined),
    s.cmd<unknown>({ type: '_oino/get_settings', key: 'defaultModel' }).catch(() => undefined),
    fusionOn ? s.cmd<unknown>({ type: '_oino/get_extension_settings', namespace: 'fusion' }).catch(() => undefined) : Promise.resolve(undefined),
    hostReadiness(s.hostId).catch(() => undefined),
  ]);
  const cycle = valueOf(settings);
  const startup = fusionStartup(fusionSettings) ?? startupKey(valueOf(provider), valueOf(model));
  return { models: parseModels(modelsReply), ...(readiness ? { readiness } : {}), ...(fusion ? { fusion } : {}), cycle, ...(startup ? { startup } : {}) };
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

/** The chat picker's content (inside the dialog / sheet). */
export function ChatModelPickerBody(props: { target: ModelPickerTarget }) {
  // ensure() subscribes; acquire() pins the session against the LRU while shown.
  const s = createMemo(() => sessions().ensure(props.target.hostId, props.target.sessionId));
  onCleanup(s().acquire());
  const [data, { refetch }] = createResource(liveSource(s), async (state) => (state === 'live' ? loadPicker(s()) : undefined));

  const allModels = () => data.latest?.models ?? [];
  const fusion = () => data.latest?.fusion;
  const byKey = createMemo(() => new Map(allModels().map((m) => [modelKey(m), m])));
  const current = (): Selection | undefined => selectionOf(s().state.model, fusion()?.active);
  const currentKey = () => {
    const c = current();
    return c?.kind === 'single' ? c.key : undefined;
  };

  const [mode, setMode] = createSignal<'single' | 'fusion'>('single');
  const [query, setQuery] = createSignal('');
  /** F-02: one view shared with every picker (`ModelList.modelView`). */
  const list = createMemo(() => modelView(allModels(), data.latest?.readiness, fusion()?.curated, current(), query(), data.latest?.startup));
  const models = () => list().usable;
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
  const [pick, setPick] = createSignal<string | undefined>();
  const [pendingLead, setPendingLead] = createSignal<string | undefined>();
  const [pendingSidekick, setPendingSidekick] = createSignal<string | undefined>();
  const [leadEffort, setLeadEffort] = createSignal('medium');
  const [sidekickEffort, setSidekickEffort] = createSignal('medium');
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  // The first load opens the tab (and the effort pills) the running selection implies.
  let settled = false;
  createEffect(() => {
    if (settled || !data.latest) return;
    settled = true;
    setMode(current()?.kind === 'fusion' && fusion() ? 'fusion' : 'single');
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

  const curated = () => fusion()?.curated;
  const colRows = createMemo(() => columnModels(models(), query()));
  const leadRows = createMemo(() => curatedFirst(colRows(), (curated()?.lead ?? []).map((e) => e.key)));
  const sidekickRows = createMemo(() => curatedFirst(colRows(), (curated()?.sidekick ?? []).map((e) => e.key)));
  /** A key the catalogue knows (a curated default can name a provider this PC lacks). */
  const known = (key: string | undefined): string | undefined => (key && byKey().has(key) ? key : undefined);
  const firstKnown = (l: ReadonlyArray<{ key: string }> | undefined) => l?.find((e) => byKey().has(e.key))?.key;
  const leadKey = () =>
    pendingLead() ??
    activePair()?.lead ??
    fusion()?.status?.leadKey ??
    known(curated()?.default.lead) ??
    firstKnown(curated()?.lead) ??
    (models()[0] ? modelKey(models()[0]!) : undefined);
  const sidekickKey = () =>
    pendingSidekick() ??
    activePair()?.sidekick ??
    fusion()?.status?.sidekickKey ??
    known(curated()?.default.sidekick) ??
    firstKnown(curated()?.sidekick) ??
    (models()[1] ? modelKey(models()[1]!) : models()[0] ? modelKey(models()[0]!) : undefined);
  /** A curated effort the Fusion pills can show (they offer low / medium / high). */
  const pillEffort = (key: string): string | undefined => {
    const e = curated()?.effort[key];
    return e && (FUSION_EFFORTS as readonly string[]).includes(e) ? e : undefined;
  };
  const [startup, setStartup] = createSignal<string | undefined>();
  createEffect(() => setStartup(data.latest?.startup));
  /** The model "Use at startup" acts on: the pending pick, else the running single model. */
  const startupTarget = (): string | undefined => pick() ?? currentKey();
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
      const k = pick();
      setError(calmModelError(e, k ? shortModelName(nameOf(k)) : undefined));
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
        await setChatModel(s(), key.slice(0, slash), key.slice(slash + 1));
      }
      // The curated default effort comes with a suggested pick (the TUI picker's per-model effort).
      const effort = curated()?.effort[key];
      if (effort) await s().cmd({ type: 'set_thinking_level', level: effort }).catch(() => undefined);
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
  const onSelect = (m: ModelInfo, isCurrent: boolean) => {
    const key = modelKey(m);
    setError(null);
    // D-07: one click switches. Only a running Fusion pair keeps the explicit
    // "Turn Fusion off & switch" step (it ends something the user set up).
    if (isCurrent) {
      setPick(undefined);
      return;
    }
    if (current()?.kind === 'fusion') {
      setPick(nextSinglePick(current(), pick(), key));
      return;
    }
    setPick(key);
    applySingle();
  };

  const useAtStartup = async (key = startupTarget()) => {
    if (!key || busy()) return;
    setBusy(true);
    setError(null);
    try {
      await writeStartupModel(s(), key, { effort: curated()?.effort[key], fusionOn: fusionOn() });
      setStartup(key);
      toasts().push(`New chats start on ${shortModelName(nameOf(key))}`, 'success');
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const cycle = createCycleSet({ target: s, stored: () => data.latest?.cycle, loaded: () => !!data.latest, onError: setError });
  /** The pair's roles as the extension knows them (the running pair, else its configured one). */
  const roleLead = () => activePair()?.lead ?? fusion()?.status?.leadKey;
  const roleSidekick = () => activePair()?.sidekick ?? fusion()?.status?.sidekickKey;
  /** ⋯ → "Set as Fusion lead / sidekick": the Fusion tab's write (`fusion.set-pair`) with the other seat kept. */
  const setRole = async (side: 'lead' | 'sidekick', key: string) => {
    if (busy()) return;
    const lead = side === 'lead' ? key : leadKey();
    const sidekick = side === 'sidekick' ? key : sidekickKey();
    if (!lead || !sidekick) return;
    const effort = pillEffort(key);
    const le = side === 'lead' && effort ? effort : leadEffort();
    const se = side === 'sidekick' && effort ? effort : sidekickEffort();
    const from = now();
    setBusy(true);
    setError(null);
    try {
      await setFusionActive(s(), { kind: 'fusion', lead, sidekick, leadEffort: le, sidekickEffort: se });
      setPendingLead(lead);
      setPendingSidekick(sidekick);
      setLeadEffort(le);
      setSidekickEffort(se);
      toasts().push(pairToast(from, nowLabel({ kind: 'fusion', lead, sidekick }, nameOf)), 'success');
      void refetch();
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  };
  const rowActions = (): RowActions => ({
    ...(fusionOn()
      ? {
          fusion: {
            lead: roleLead(),
            sidekick: roleSidekick(),
            setLead: (m: ModelInfo) => void setRole('lead', modelKey(m)),
            setSidekick: (m: ModelInfo) => void setRole('sidekick', modelKey(m)),
          },
        }
      : {}),
    cycle: { has: cycle.has, toggle: (m, on) => void cycle.toggle(modelKey(m), on) },
    startup: { set: (m) => void useAtStartup(modelKey(m)) },
  });

  /** One Fusion column row. */
  const ColRow = (p: { m: ModelInfo; side: 'lead' | 'sidekick' }) => {
    const key = modelKey(p.m);
    const name = shortModelName(modelName(p.m));
    const chosen = () => (p.side === 'lead' ? leadKey() : sidekickKey()) === key;
    return (
      <button
        type="button"
        role="menuitemradio"
        aria-checked={chosen()}
        class="mp-row"
        title={rowTitle(modelName(p.m), p.m)}
        aria-current={chosen() ? 'true' : undefined}
        onClick={() => {
          setError(null);
          const effort = pillEffort(key);
          if (p.side === 'lead') {
            setPendingLead(key);
            if (effort) setLeadEffort(effort);
          } else {
            setPendingSidekick(key);
            if (effort) setSidekickEffort(effort);
          }
        }}
      >
        <span class="mp-row-name">{name}</span>
        <Show when={chosen()}>
          <Check size={16} class="mp-check" />
        </Show>
      </button>
    );
  };
  const Column = (p: { side: 'lead' | 'sidekick' }) => {
    const rows = () => (p.side === 'lead' ? leadRows() : sidekickRows());
    return (
      <div class="mp-col">
        <p class="mp-label">{p.side === 'lead' ? 'Lead' : 'Sidekick'}</p>
        <div class="mp-list mp-col-list">
          <Show when={rows().curated.length > 0}>
            <p class="mp-group">Curated</p>
            <For each={rows().curated}>{(m) => <ColRow m={m} side={p.side} />}</For>
            <p class="mp-group">All models</p>
          </Show>
          <For each={rows().rest.slice(0, LIST_CAP)} fallback={<Show when={rows().curated.length === 0}><p class="mp-empty">No model matches.</p></Show>}>
            {(m) => <ColRow m={m} side={p.side} />}
          </For>
        </div>
      </div>
    );
  };
  const Effort = (p: { side: 'lead' | 'sidekick' }) => {
    const value = () => (p.side === 'lead' ? leadEffort() : sidekickEffort());
    const set = (level: string) => (p.side === 'lead' ? setLeadEffort(level) : setSidekickEffort(level));
    return (
      <div class="mp-effort" role="group" aria-label={p.side === 'lead' ? 'Lead effort' : 'Sidekick effort'}>
        <span class="mp-label">{p.side === 'lead' ? 'Lead effort' : 'Sidekick effort'}</span>
        <For each={FUSION_EFFORTS}>
          {(level) => (
            <button type="button" role="menuitemradio" aria-checked={value() === level} class="mp-pill" onClick={() => set(level)}>
              {level}
            </button>
          )}
        </For>
      </div>
    );
  };

  const loading = () => !!data.loading && !data.latest;
  const fusionContent = () => (
    <Show when={!loading()} fallback={<PickerSkeleton />}>
      <div class="mp-fusion" ref={(el) => queueMicrotask(() => revealPicked(el))} data-testid="model-picker-fusion">
        <Column side="lead" />
        <Column side="sidekick" />
      </div>
      <Effort side="lead" />
      <Effort side="sidekick" />
    </Show>
  );

  return (
    <ModelPickerPanel
      view={list()}
      query={query()}
      onQuery={setQuery}
      now={now()}
      tabs={
        <Show when={fusionOn()}>
          <ModeTabs mode={mode()} onMode={chooseMode} />
        </Show>
      }
      searchPlaceholder={mode() === 'fusion' ? `Search both columns (${models().length} models)` : undefined}
      loading={loading()}
      content={mode() === 'fusion' ? fusionContent() : undefined}
      currentKey={currentKey()}
      pickedKey={pick()}
      startupKey={startup()}
      rowActions={data.latest ? rowActions() : undefined}
      cycleFilter={{ set: cycle, initial: props.target.focus === 'cycle' }}
      onSelect={onSelect}
      notice={
        <>
          <Show when={mode() === 'single' && current()?.kind === 'fusion' && !action()}>
            <p class="mp-hint">Fusion is on. Pick a model to switch to it.</p>
          </Show>
          <PickerError error={error()} />
        </>
      }
      scope={
        mode() === 'single'
          ? 'Your pick applies to this chat only. “Use at startup” sets the model new chats start with.'
          : 'Your pair applies to this chat only.'
      }
      actions={
        <Show
          when={mode() === 'single'}
          fallback={
            <button type="button" class="pair-primary mp-apply" disabled={busy() || !leadKey() || !sidekickKey() || unchanged()} aria-busy={busy() ? 'true' : undefined} onClick={applyPair}>
              <Show when={busy()}>
                <span class="mp-spinner" aria-hidden="true" />
              </Show>
              {busy() ? 'Applying…' : unchanged() ? 'Current pair' : activePair() ? 'Update pair' : 'Use this pair'}
            </button>
          }
        >
          <Show when={startupTarget()}>
            {(target) => (
              <PickerSecondary
                label={startup() === target() ? 'Startup model' : 'Use at startup'}
                disabled={busy() || startup() === target()}
                title={
                  startup() === target()
                    ? `New chats already start on ${shortModelName(nameOf(target()))}`
                    : `Start new chats on ${shortModelName(nameOf(target()))} (${target()})`
                }
                onClick={() => void useAtStartup()}
                testId="model-use-startup"
              />
            )}
          </Show>
          <Show when={action()}>
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
      }
    />
  );
}

/** The chat picker: the one model dialog (a full-screen sheet on the phone). */
export function ChatModelPicker(props: { target: ModelPickerTarget }) {
  return (
    // No `trigger`: focus goes back where it was (the chip when clicked, the composer after Ctrl/Cmd+M).
    <ModelDialog title="Model" testId="model-picker-dialog" onClose={closeModelPicker}>
      <ChatModelPickerBody target={props.target} />
    </ModelDialog>
  );
}

/** Mounted once by the shell: renders the picker for the open target. */
export function ModelPickerLayer() {
  return (
    <Show when={modelPickerTarget()} keyed>
      {(target) => <ChatModelPicker target={target} />}
    </Show>
  );
}
