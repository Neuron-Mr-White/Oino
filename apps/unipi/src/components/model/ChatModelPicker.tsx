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
import { createEffect, createMemo, createResource, createSignal, onCleanup, Show } from 'solid-js';
import { sessions, type SessionHandle } from '../../lib/session/store';
import { toasts } from '../../lib/toasts';
import { liveSource } from '../session/modalData';
import { fetchFusion, fusionAvailable, setFusionActive, type FusionState } from '../../lib/model/fusion';
import {
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
import { ThinkingFooter } from './ThinkingFooter';
import { layout } from '../../lib/layout';
import { startupKey } from './suggested';
import { createFusionPair } from './FusionPair';
import { modelView, type RowActions } from './ModelList';
import {
  createCycleSet,
  fusionStartup,
  ModeTabs,
  ModelDialog,
  ModelPickerPanel,
  PickerError,
  StartupButton,
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

export { revealPicked } from './FusionPair';

/** The chat picker's content (inside the dialog / sheet). */
export function ChatModelPickerBody(props: { target: ModelPickerTarget }) {
  // ensure() subscribes; acquire() pins the session against the LRU while shown.
  const s = createMemo(() => sessions().ensure(props.target.hostId, props.target.sessionId));
  // Per handle: a retargeted picker must release the chat it pinned (a stale pin per
  // retarget filled the 3-chat LRU — chat-vanish).
  createEffect(() => onCleanup(s().acquire()));
  const [data, { refetch }] = createResource(liveSource(s), async (state) => (state === 'live' ? loadPicker(s()) : undefined));

  const allModels = () => data.latest?.models ?? [];
  const fusion = () => data.latest?.fusion;
  const byKey = createMemo(() => new Map(allModels().map((m) => [modelKey(m), m])));
  /** [owner-fix-8] the model of a switch in flight (the host confirms later). */
  const [pending, setPending] = createSignal<string | undefined>();
  const current = (): Selection | undefined => {
    // [owner-fix-8] A switch in flight shows its model right away: the host
    // answers `set_model` only once the chat's runtime is up (a chat opened
    // from its file wakes behind its history), and the picker must not sit on
    // "Switching…" for those seconds.
    const p = pending();
    if (p) return { kind: 'single', key: p };
    return selectionOf(s().state.model, fusion()?.active);
  };
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
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  // The host caught up: the optimistic model is the real one now.
  createEffect(() => {
    const p = pending();
    if (!p) return;
    const live = selectionOf(s().state.model, undefined);
    if (live?.kind === 'single' && live.key === p) setPending(undefined);
  });

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
  /** The Fusion columns + effort pills (shared with the draft's picker). */
  const pair = createFusionPair({
    models,
    catalogue: allModels,
    query,
    curated,
    base: () => ({ lead: activePair()?.lead ?? fusion()?.status?.leadKey, sidekick: activePair()?.sidekick ?? fusion()?.status?.sidekickKey }),
    onPick: () => setError(null),
  });
  const { leadKey, sidekickKey, leadEffort, sidekickEffort, setLeadEffort, setSidekickEffort, setPendingLead, setPendingSidekick, pillEffort } = pair;
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
    // [owner-fix-8] The switch is optimistic: the picker closes and the toast
    // lands as soon as the command is on its way. The host answers `set_model`
    // only when the chat's runtime is up — a chat opened from its file wakes
    // behind its whole history — and waiting for that reply held the picker on
    // "Switching…" for seconds. A failure comes back as an error toast and the
    // chip returns to the host's model.
    if (busy()) return;
    setError(null);
    setPending(key);
    closeModelPicker();
    toasts().push(switchToast(from, shortModelName(nameOf(key))), 'success');
    void (async () => {
      try {
        if (fusionOn() && fusion()?.active) {
          await setFusionActive(s(), { kind: 'single', model: key });
        } else {
          const slash = key.indexOf('/');
          await setChatModel(s(), key.slice(0, slash), key.slice(slash + 1));
        }
        // The curated default effort comes with a suggested pick (the TUI picker's per-model effort).
        const effort = curated()?.effort[key];
        if (effort) await s().cmd({ type: 'set_thinking_level', level: effort }).catch(() => undefined);
      } catch (e) {
        setPending(undefined);
        toasts().push(calmModelError(e, shortModelName(nameOf(key))), 'error');
      }
    })();
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

  const loading = () => !!data.loading && !data.latest;
  const fusionContent = () => pair.content(loading());

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
              <StartupButton
                key={target()}
                startup={startup()}
                busy={busy()}
                name={shortModelName(nameOf(target()))}
                onSet={() => void useAtStartup()}
                onAlready={() => toasts().push(`New chats already start on ${shortModelName(nameOf(target()))}`, 'info')}
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
    <ModelDialog
      title="Model"
      testId="model-picker-dialog"
      onClose={closeModelPicker}
      footer={layout().desktop() ? undefined : <ThinkingFooter session={sessions().ensure(props.target.hostId, props.target.sessionId)} />}
    >
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
