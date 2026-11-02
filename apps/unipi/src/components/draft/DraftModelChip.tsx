/**
 * The draft's model chip (Z-19): looks and sits exactly like the chat's
 * model chip and opens the SAME model dialog (`ModelDialog` +
 * `ModelPickerPanel`): this PC's catalogue (a live chat's list on the same
 * PC, else the host's), readiness, the curated ranking. "Now" is the model
 * the new chat will start with; a pick applies to the chat about to start
 * (before its first prompt). The rows' ⋯ menu (cycle set — pi's
 * `enabledModels`, written to the host — and "Use at startup", the host's
 * `defaultProvider` / `defaultModel`), the Cycle set filter chip and the
 * footer's "Use at startup" sit where the chat's do.
 *
 * Fusion tab (when `unipi.fusion` is enabled on this PC): the same lead /
 * sidekick columns and effort pills as the chat's; "Use this pair" keeps the
 * pair as a draft choice (the chip and "Now" say `Fusion · A ◆ B`), and the
 * new chat applies it right after it starts through the extension's own
 * `fusion.set-pair` (startSession). A Single pick turns the draft's pair off.
 */
import { createEffect, createMemo, createResource, createSignal, Show } from 'solid-js';
import { listExtensions } from '../../settings/hostCmd';
import { createFusionPair } from '../model/FusionPair';
import { Brain, ChevronUp, Cpu, Plug } from 'lucide-solid';
import { layout } from '../../lib/layout';
import type { SessionHandle } from '../../lib/session/store';
import { ThinkingFooter } from '../model/ThinkingFooter';
import { canThink, ensureThinkingLevels, thinkingLevels } from '../../lib/model/thinking';
import { modelKey, modelName, nowLabel, pairUnchanged, parseModels, shortModelName, type ModelInfo, type Selection } from '../../lib/model/picker';
import { FUSION_EXTENSION_ID, fetchFusion, fusionAvailable, type FusionCurated } from '../../lib/model/fusion';
import { openSettingsAt } from '../../lib/settings/open';
import { toasts } from '../../lib/toasts';
import { modelView } from '../model/ModelList';
import { startupKey } from '../model/suggested';
import { createCycleSet, ModeTabs, ModelDialog, ModelPickerPanel, PickerError, PickerSecondary, writeStartupModel } from '../model/ModelPopover';
import { modelChipLabel, modelChipTitle, thinkingOn } from '../model/ModelChip';
import { RollingLabel } from '../RollingLabel';
import type { DraftSession } from './draftSession';

type DraftData = { models: ModelInfo[]; curated?: FusionCurated; startup?: string; fusionOn?: boolean; fusionTab?: boolean; cycle?: unknown };

const valueOf = (reply: unknown): unknown => (reply as { value?: unknown } | undefined)?.value;

export function DraftModelChip(props: { session: DraftSession; disabled?: boolean }) {
  const [open, setOpen] = createSignal(false);
  // Phone: no Thinking chip fetches the levels — the chip's glyph needs them (§3.5).
  createEffect(() => {
    void props.session.state.model?.id;
    void props.session.state.conn;
    if (!layout().desktop()) ensureThinkingLevels(props.session as unknown as SessionHandle);
  });
  /** The draft's Fusion pair, when one is picked (applied right after the chat starts). */
  const draftPair = () => props.session.choices().fusion;
  // D-01: no usable model on show → say so, never a name the new chat can't run.
  const label = () => {
    const pair = draftPair();
    if (pair) return nowLabel({ kind: 'fusion', ...pair }, nameOf);
    return props.session.state.model?.id ? modelChipLabel(props.session.state.model) : 'Choose a model';
  };
  const current = () => modelKey(props.session.state.model ?? {});
  // F-02 / G-06: the SAME catalogue, readiness and curated ranking as the in-chat picker.
  const [data, { mutate }] = createResource(
    () => (open() ? props.session.hostId : undefined),
    // The draft answers from a live chat on this PC, else the host's list (P0-09).
    async (): Promise<DraftData> => {
      const peer = props.session.peer();
      const fusionOn = peer ? await fusionAvailable(peer).catch(() => false) : false;
      // The Fusion tab needs no live chat: the PC's own extension list says whether it runs.
      const fusionTab =
        fusionOn ||
        (await listExtensions(props.session.hostId)
          .then((res) => res.ok && res.value.some((e) => e.id === FUSION_EXTENSION_ID && e.enabled))
          .catch(() => false));
      const [reply, fusion, provider, model, cycle] = await Promise.all([
        props.session.cmd<unknown>({ type: 'get_available_models' }).catch(() => undefined),
        peer && fusionOn ? fetchFusion(peer).catch(() => undefined) : Promise.resolve(undefined),
        // The draft's `_oino/get_settings` reads the host's settings (no chat needed).
        props.session.cmd<unknown>({ type: '_oino/get_settings', key: 'defaultProvider' }).catch(() => undefined),
        props.session.cmd<unknown>({ type: '_oino/get_settings', key: 'defaultModel' }).catch(() => undefined),
        props.session.cmd<unknown>({ type: '_oino/get_settings', key: 'enabledModels' }).catch(() => undefined),
      ]);
      const startup = startupKey(valueOf(provider), valueOf(model));
      return {
        models: parseModels(reply),
        ...(fusion?.curated ? { curated: fusion.curated } : {}),
        ...(startup ? { startup } : {}),
        fusionOn,
        fusionTab,
        cycle: valueOf(cycle),
      };
    },
  );
  const models = () => data.latest?.models;
  const [query, setQuery] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  let chip: HTMLButtonElement | undefined;
  const cycle = createCycleSet({ target: () => props.session, stored: () => data.latest?.cycle, loaded: () => !!data.latest, onError: setError });
  const currentSel = (): Selection | undefined => {
    const pair = draftPair();
    if (pair) return { kind: 'fusion', ...pair };
    return props.session.state.model?.id ? { kind: 'single', key: current() } : undefined;
  };
  const view = createMemo(() => {
    const all = models() ?? [];
    const m = props.session.state.model;
    // Before any list is known the default model still shows (never a name the user can't see).
    const rows = all.length || !m?.id ? all : [{ provider: m.provider ?? '', id: m.id, name: m.name ?? m.id } as ModelInfo];
    return modelView(rows, props.session.readiness(), data.latest?.curated, currentSel(), query(), data.latest?.startup);
  });
  const noUsable = () => !data.loading && view().usable.length === 0 && !query().trim();
  const close = () => setOpen(false);
  const pick = (m: ModelInfo) => {
    close();
    props.session.pickModel({ provider: m.provider ?? '', modelId: m.id, name: modelName(m) });
  };
  const nameOf = (key: string) => {
    const m = (models() ?? []).find((x) => modelKey(x) === key);
    if (m) return shortModelName(modelName(m));
    // Before the list loads, a pair seat still reads as a name, not a `provider/id` path.
    const cur = props.session.state.model;
    if (cur?.id && modelKey(cur) === key) return shortModelName(cur.name ?? cur.id);
    return key.slice(key.indexOf('/') + 1);
  };
  // Fusion tab (unipi.fusion on this PC): the pair is a draft choice, applied after the chat starts.
  const fusionTab = () => !!data.latest?.fusionTab;
  const [mode, setMode] = createSignal<'single' | 'fusion'>('single');
  const pair = createFusionPair({
    models: () => view().usable,
    catalogue: () => models() ?? [],
    query,
    curated: () => data.latest?.curated,
    base: () => ({ lead: draftPair()?.lead, sidekick: draftPair()?.sidekick }),
    onPick: () => setError(null),
  });
  const openOn = () => {
    setMode(draftPair() ? 'fusion' : 'single');
    const p = draftPair();
    pair.setPendingLead(undefined);
    pair.setPendingSidekick(undefined);
    pair.setLeadEffort(p?.leadEffort ?? 'medium');
    pair.setSidekickEffort(p?.sidekickEffort ?? 'medium');
  };
  const pairSame = () => pairUnchanged(currentSel(), pair.leadKey(), pair.sidekickKey(), pair.leadEffort(), pair.sidekickEffort());
  const usePair = () => {
    const lead = pair.leadKey();
    const sidekick = pair.sidekickKey();
    if (!lead || !sidekick) return;
    const row = (models() ?? []).find((x) => modelKey(x) === lead);
    const slash = lead.indexOf('/');
    props.session.pickFusion(
      { lead, sidekick, leadEffort: pair.leadEffort(), sidekickEffort: pair.sidekickEffort() },
      { provider: lead.slice(0, slash), modelId: lead.slice(slash + 1), ...(row ? { name: modelName(row) } : {}) },
    );
    close();
  };
  /** "Now": the model (or Fusion pair) the new chat will start with. */
  const nowName = () => {
    const p = draftPair();
    if (p) return nowLabel({ kind: 'fusion', ...p }, nameOf);
    const m = props.session.state.model;
    if (!m?.id) return 'No model chosen yet';
    const row = (models() ?? []).find((x) => modelKey(x) === current());
    return row ? modelName(row) : (m.name ?? m.id);
  };
  const useAtStartup = async (key: string) => {
    if (busy()) return;
    setBusy(true);
    setError(null);
    try {
      await writeStartupModel(props.session, key, { effort: data.latest?.curated?.effort[key], fusionOn: data.latest?.fusionOn });
      if (data.latest) mutate({ ...data.latest, startup: key });
      toasts().push(`New chats start on ${nameOf(key)}`, 'success');
    } catch (e) {
      setError(`Couldn’t set the startup model: ${String((e as { message?: unknown })?.message ?? e)}`);
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <button
        type="button"
        class="tb-chip tb-chip-model"
        aria-label={`Model: ${label()}. Change model`}
        aria-haspopup="dialog"
        aria-expanded={open()}
        title={modelChipTitle(props.session.state.model)}
        disabled={props.disabled}
        ref={chip}
        onClick={() => {
          setQuery('');
          setError(null);
          if (open()) close();
          else {
            openOn();
            setOpen(true);
          }
        }}
        data-testid="model-chip"
      >
        <Cpu size={15} class="tb-ic" />
        <RollingLabel class="tb-label" text={label()} testId="model-chip-label" />
        <Show when={!layout().desktop() && thinkingOn(props.session.state.thinkingLevel) && canThink(thinkingLevels(props.session as unknown as SessionHandle))}>
          <Brain size={14} class="tb-think-glyph" aria-hidden="true" />
        </Show>
        <ChevronUp size={12} class="tb-chev" />
      </button>
      <Show when={open()}>
        <ModelDialog
          title="Model"
          testId="draft-model-menu"
          onClose={close}
          trigger={() => chip}
          footer={layout().desktop() ? undefined : <ThinkingFooter session={props.session as unknown as SessionHandle} />}
        >
          <ModelPickerPanel
            view={view()}
            query={query()}
            onQuery={setQuery}
            now={nowName()}
            tabs={
              <Show when={fusionTab() && !noUsable()}>
                <ModeTabs
                  mode={mode()}
                  onMode={(m) => {
                    setMode(m);
                    setError(null);
                  }}
                />
              </Show>
            }
            searchPlaceholder={mode() === 'fusion' && fusionTab() ? `Search both columns (${view().usable.length} models)` : undefined}
            content={mode() === 'fusion' && fusionTab() ? pair.content(!!data.loading && !data.latest) : undefined}
            loading={!!data.loading && !data.latest}
            showEmpty={noUsable()}
            empty={
              <div class="mp-connect" data-testid="draft-model-connect">
                <Plug size={18} class="mp-connect-ic" aria-hidden="true" />
                <p class="mp-connect-text">
                  {view().unavailable.length || props.session.readiness()?.providers.some((p) => p.configured)
                    ? 'The providers on this PC aren’t working right now.'
                    : (models() ?? []).length || props.session.readiness()?.known
                      ? 'No model provider is connected yet.'
                      : 'The models show here once a provider is connected.'}
                </p>
                <button
                  type="button"
                  role="menuitem"
                  class="btn btn-secondary"
                  onClick={() => {
                    close();
                    openSettingsAt('providers', 'sec-providers');
                  }}
                  data-testid="draft-model-connect-open"
                >
                  Connect a provider
                </button>
              </div>
            }
            currentKey={current() !== '/' && !draftPair() ? current() : undefined}
            startupKey={data.latest?.startup}
            rowActions={
              data.latest
                ? {
                    cycle: { has: cycle.has, toggle: (m, on) => void cycle.toggle(modelKey(m), on) },
                    startup: { set: (m) => void useAtStartup(modelKey(m)) },
                  }
                : undefined
            }
            onSelect={(m) => pick(m)}
            cycleFilter={data.latest ? { set: cycle } : undefined}
            notice={
              <>
                <Show when={mode() === 'single' && draftPair()}>
                  <p class="mp-hint" data-testid="draft-fusion-hint">
                    Fusion is on for the new chat. Pick a model to use one model instead.
                  </p>
                </Show>
                <PickerError error={error()} />
              </>
            }
            scope={
              noUsable()
                ? undefined
                : mode() === 'fusion' && fusionTab()
                  ? 'Your pair applies to the chat about to start, right after it starts.'
                  : 'Your pick applies to the chat about to start. “Use at startup” sets the model new chats start with.'
            }
            actions={
              <Show
                when={!(mode() === 'fusion' && fusionTab())}
                fallback={
                  <button
                    type="button"
                    class="pair-primary mp-apply"
                    disabled={!pair.leadKey() || !pair.sidekickKey() || pairSame()}
                    onClick={usePair}
                    data-testid="draft-fusion-use"
                  >
                    {pairSame() ? 'Current pair' : draftPair() ? 'Update pair' : 'Use this pair'}
                  </button>
                }
              >
              <Show when={props.session.state.model?.id && !noUsable()}>
                <PickerSecondary
                  label={data.latest?.startup === current() ? 'Startup model' : 'Use at startup'}
                  disabled={busy() || !data.latest || data.latest.startup === current()}
                  title={data.latest?.startup === current() ? `New chats already start on ${nameOf(current())}` : `Start new chats on ${nameOf(current())} (${current()})`}
                  onClick={() => void useAtStartup(current())}
                  testId="model-use-startup"
                />
              </Show>
              </Show>
            }
          />
        </ModelDialog>
      </Show>
    </>
  );
}
