/**
 * The in-chat model picker (WP-A14, Z-69 + Z-76): the ONE model popover
 * (`ModelPopover`) anchored to the chat's model chip, with this chat's data
 * and footer rows. Entry points (Z-70): the composer's model chip,
 * Ctrl/Cmd+M, `/model` and `/scoped-models` (opens the Cycle set view).
 *
 * Data is the host's: `get_available_models` for the catalogue, plus
 * `unipi.fusion`'s `fusion.status` / `fusion.set-pair` when the extension is
 * enabled (the Fusion row is hidden otherwise — no dead control). A pick
 * switches THIS chat only (`set_model`, or Fusion's single pick while a pair
 * runs, after an explicit "Turn Fusion off & switch"), and a success toast
 * says what changed (Z-72). Dismissing sends nothing.
 *
 * Footer rows:
 * - "Use at startup" writes the model a fresh session opens with (pi
 *   `defaultProvider` / `defaultModel`, plus Fusion's `startup.model` when the
 *   extension is on — it wins on a fresh session), like the TUI's Alt+Enter.
 * - "Fusion…" (extension on) opens the Lead / Sidekick columns and effort
 *   pills in the same popover; the shared search filters both columns.
 * - "Cycle set…" opens pi's `enabledModels` (what Ctrl+Alt+P steps through).
 *
 * P0-10: Fusion's curated preset ranks the list (Suggested group, Lead /
 * Sidekick badges, the curated effort applied with the pick) and puts the
 * Fusion columns' curated lists first.
 */
import { createEffect, createMemo, createResource, createSignal, For, onCleanup, Show } from 'solid-js';
import { Check, Combine, Repeat, Star } from 'lucide-solid';
import { sessions, type SessionHandle } from '../../lib/session/store';
import { toasts } from '../../lib/toasts';
import { liveSource } from '../session/modalData';
import { fetchFusion, fusionAvailable, setFusionActive, type FusionState } from '../../lib/model/fusion';
import {
  columnModels,
  FUSION_EFFORTS,
  hits as modelHits,
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
import { LIST_CAP, modelView } from './ModelList';
import { cycleKeyOf, cycleKeysOf, cycleLabel, toggleCycleKey } from '../../lib/model/cycle';
import {
  centredAnchor,
  fusionStartup,
  ModelPickerPanel,
  ModelPopover,
  PickerFootRow,
  PickerSearch,
  PickerSubHead,
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

type View = 'list' | 'fusion' | 'cycle';

/** The chat picker's content (inside the popover / sheet). */
export function ChatModelPickerBody(props: { target: ModelPickerTarget }) {
  // ensure() subscribes; acquire() pins the session against the LRU while shown.
  const s = createMemo(() => sessions().ensure(props.target.hostId, props.target.sessionId));
  onCleanup(s().acquire());
  const [data] = createResource(liveSource(s), async (state) => (state === 'live' ? loadPicker(s()) : undefined));

  const allModels = () => data.latest?.models ?? [];
  const fusion = () => data.latest?.fusion;
  const byKey = createMemo(() => new Map(allModels().map((m) => [modelKey(m), m])));
  const current = (): Selection | undefined => selectionOf(s().state.model, fusion()?.active);
  const currentKey = () => {
    const c = current();
    return c?.kind === 'single' ? c.key : undefined;
  };

  const [view, setView] = createSignal<View>(props.target.focus === 'cycle' ? 'cycle' : 'list');
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
  const [cycle, setCycle] = createSignal<{ exact: string[]; custom: string[] }>({ exact: [], custom: [] });

  // The first load opens the view (and the effort pills) the running selection implies.
  let settled = false;
  createEffect(() => {
    if (settled || !data.latest) return;
    settled = true;
    setCycle(cycleKeysOf(data.latest.cycle));
    if (props.target.focus !== 'cycle' && current()?.kind === 'fusion' && fusion()) setView('fusion');
    const pair = activePair();
    const st = fusion()?.status;
    setLeadEffort(pair?.leadEffort ?? st?.leadEffort ?? 'medium');
    setSidekickEffort(pair?.sidekickEffort ?? st?.sidekickEffort ?? 'medium');
  });
  const go = (next: View) => {
    settled = true;
    setView(next);
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

  const useAtStartup = async () => {
    const key = startupTarget();
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
  const cycleRows = createMemo(() => {
    const q = query().trim().toLowerCase();
    return q ? models().filter((m) => modelHits(m, q)) : models();
  });
  const cycleText = () => cycleLabel(cycle().exact, cycle().custom, models().length, list().suggested.length);

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

  const loading = () => !!data.loading && !data.latest;
  const nowLine = () => (
    <p class="mp-now" data-testid="model-picker-now">
      <span class="mp-now-word">Now</span>
      <span class="mp-now-value">{now()}</span>
    </p>
  );
  const errorLine = () => (
    <Show when={error()}>
      <p class="mp-error" role="alert" data-testid="model-picker-error">
        {error()}
      </p>
    </Show>
  );
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

  return (
    <div class="mpop-view" data-testid="model-picker" data-view={view()}>
      <Show when={view() === 'list'}>
        <ModelPickerPanel
          view={list()}
          query={query()}
          onQuery={setQuery}
          loading={loading()}
          header={nowLine()}
          currentKey={currentKey()}
          pickedKey={pick()}
          startupKey={startup()}
          onSelect={onSelect}
          notice={
            <>
              {errorLine()}
              <Show when={action()}>
                {(act) => (
                  <div class="mpop-confirm">
                    <span class="mp-hint">{busy() ? 'Switching…' : 'Fusion is on for this chat.'}</span>
                    <button type="button" role="menuitem" class="pair-primary mp-apply" disabled={busy()} aria-busy={busy() ? 'true' : undefined} onClick={applySingle}>
                      <Show when={busy()}>
                        <span class="mp-spinner" aria-hidden="true" />
                      </Show>
                      {act().label}
                    </button>
                  </div>
                )}
              </Show>
            </>
          }
          footer={
            <>
              <Show when={startupTarget()}>
                {(target) => (
                  <PickerFootRow
                    icon={<Star size={15} />}
                    label={startup() === target() ? 'Startup model' : 'Use at startup'}
                    detail={startup() === target() ? 'New chats start on it' : `New chats start on ${shortModelName(nameOf(target()))}`}
                    disabled={busy() || startup() === target()}
                    title={
                      startup() === target()
                        ? `New chats already start on ${shortModelName(nameOf(target()))}`
                        : `Start new chats on ${shortModelName(nameOf(target()))} (${target()}). Your pick above switches this chat only.`
                    }
                    onSelect={() => void useAtStartup()}
                    testId="model-use-startup"
                  />
                )}
              </Show>
              <Show when={fusionOn()}>
                <PickerFootRow
                  icon={<Combine size={15} />}
                  label="Fusion…"
                  detail={activePair() ? 'On' : 'Lead + sidekick'}
                  expanded={false}
                  onSelect={() => go('fusion')}
                  testId="model-open-fusion"
                />
              </Show>
              <PickerFootRow
                icon={<Repeat size={15} />}
                label="Cycle set…"
                detail={cycleText().replace(/^Cycle set · /, '')}
                expanded={false}
                onSelect={() => go('cycle')}
                testId="model-picker-cycle-toggle"
              />
            </>
          }
        />
      </Show>

      <Show when={view() === 'fusion'}>
        <PickerSubHead title="Fusion" onBack={() => go('list')} testId="model-fusion-back" />
        <PickerSearch value={query()} onInput={setQuery} placeholder={`Search both columns (${models().length} models)`} />
        <div class="mpop-head">{nowLine()}</div>
        <Show when={!loading()} fallback={<div class="mp-skeleton"><span /><span /><span /></div>}>
          <div class="mpop-scroll mpop-fusion-scroll">
            <div class="mp-fusion" ref={(el) => queueMicrotask(() => revealPicked(el))} data-testid="model-picker-fusion">
              <Column side="lead" />
              <Column side="sidekick" />
            </div>
            <Effort side="lead" />
            <Effort side="sidekick" />
          </div>
        </Show>
        {errorLine()}
        <div class="mpop-foot mpop-foot-actions">
          <button type="button" role="menuitem" class="pair-primary mp-apply" disabled={busy() || !leadKey() || !sidekickKey() || unchanged()} aria-busy={busy() ? 'true' : undefined} onClick={applyPair}>
            <Show when={busy()}>
              <span class="mp-spinner" aria-hidden="true" />
            </Show>
            {busy() ? 'Applying…' : unchanged() ? 'Current pair' : activePair() ? 'Update pair' : 'Use this pair'}
          </button>
        </div>
      </Show>

      <Show when={view() === 'cycle'}>
        <PickerSubHead title={cycleText()} onBack={() => go('list')} testId="model-cycle-back" />
        <PickerSearch value={query()} onInput={setQuery} />
        <p class="mp-hint mpop-note">Models Ctrl+Alt+P steps through in a chat. Every model stays selectable in the list.</p>
        <div class="mpop-scroll" data-testid="model-picker-cycle">
          <div class="mp-cycle-list" role="group" aria-label="Cycle set" data-testid="model-picker-cycle-list">
            <For each={cycleRows().slice(0, LIST_CAP)}>
              {(m) => {
                const key = cycleKeyOf(m.provider, m.id);
                return (
                  <button
                    type="button"
                    role="menuitemcheckbox"
                    class="mp-cycle-row"
                    aria-checked={cycleOf(m)}
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
        </div>
        <Show when={cycle().custom.length > 0}>
          <p class="mp-hint mpop-note" data-testid="model-picker-cycle-custom">
            Custom patterns (kept as written): {cycle().custom.join(', ')}
          </p>
        </Show>
        {errorLine()}
      </Show>
    </div>
  );
}

/** The chat chip a picker is anchored to (the shown composer's), if on screen. */
function chipFor(target: ModelPickerTarget): HTMLElement | undefined {
  const chips = [...document.querySelectorAll<HTMLElement>(`[data-testid="model-chip"][data-session="${target.hostId}/${target.sessionId}"]`)];
  return chips.find((c) => c.getClientRects().length > 0) ?? chips[0];
}

/** The chat picker: the one model popover anchored to the chat's chip (a sheet on the phone). */
export function ChatModelPicker(props: { target: ModelPickerTarget }) {
  const chip = chipFor(props.target);
  const anchor = chip?.getBoundingClientRect() ?? centredAnchor();
  return (
    // No `trigger`: focus goes back where it was (the chip when clicked, the composer after Ctrl/Cmd+M).
    <ModelPopover anchor={anchor} label="Model" testId="model-picker-pop" onClose={closeModelPicker}>
      <ChatModelPickerBody target={props.target} />
    </ModelPopover>
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
