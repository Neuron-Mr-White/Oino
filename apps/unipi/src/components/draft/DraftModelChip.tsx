/**
 * The draft's model chip (Z-19): looks and sits exactly like the chat's
 * model chip and opens the SAME model dialog (`ModelDialog` +
 * `ModelPickerPanel`): this PC's catalogue (a live chat's list on the same
 * PC, else the host's), readiness, the curated ranking. "Now" is the model
 * the new chat will start with; a pick applies to the chat about to start
 * (before its first prompt). The rows' ⋯ menu (cycle set — pi's
 * `enabledModels`, written to the host — and "Use at startup", the host's
 * `defaultProvider` / `defaultModel`), the Cycle set filter chip and the
 * footer's "Use at startup" sit where the chat's do. No Fusion tab: a pair needs a
 * running chat (`fusion.set-pair`), so it is set from the chat's picker.
 */
import { createEffect, createMemo, createResource, createSignal, Show } from 'solid-js';
import { Brain, ChevronUp, Cpu, Plug } from 'lucide-solid';
import { layout } from '../../lib/layout';
import type { SessionHandle } from '../../lib/session/store';
import { ThinkingFooter } from '../model/ThinkingFooter';
import { canThink, ensureThinkingLevels, thinkingLevels } from '../../lib/model/thinking';
import { modelKey, modelName, parseModels, shortModelName, type ModelInfo, type Selection } from '../../lib/model/picker';
import { fetchFusion, fusionAvailable, type FusionCurated } from '../../lib/model/fusion';
import { openSettingsAt } from '../../lib/settings/open';
import { toasts } from '../../lib/toasts';
import { modelView } from '../model/ModelList';
import { startupKey } from '../model/suggested';
import { createCycleSet, ModelDialog, ModelPickerPanel, PickerError, PickerSecondary, writeStartupModel } from '../model/ModelPopover';
import { modelChipLabel, modelChipTitle, thinkingOn } from '../model/ModelChip';
import { RollingLabel } from '../RollingLabel';
import type { DraftSession } from './draftSession';

type DraftData = { models: ModelInfo[]; curated?: FusionCurated; startup?: string; fusionOn?: boolean; cycle?: unknown };

const valueOf = (reply: unknown): unknown => (reply as { value?: unknown } | undefined)?.value;

export function DraftModelChip(props: { session: DraftSession; disabled?: boolean }) {
  const [open, setOpen] = createSignal(false);
  // Phone: no Thinking chip fetches the levels — the chip's glyph needs them (§3.5).
  createEffect(() => {
    void props.session.state.model?.id;
    void props.session.state.conn;
    if (!layout().desktop()) ensureThinkingLevels(props.session as unknown as SessionHandle);
  });
  // D-01: no usable model on show → say so, never a name the new chat can't run.
  const label = () => (props.session.state.model?.id ? modelChipLabel(props.session.state.model) : 'Choose a model');
  const current = () => modelKey(props.session.state.model ?? {});
  // F-02 / G-06: the SAME catalogue, readiness and curated ranking as the in-chat picker.
  const [data, { mutate }] = createResource(
    () => (open() ? props.session.hostId : undefined),
    // The draft answers from a live chat on this PC, else the host's list (P0-09).
    async (): Promise<DraftData> => {
      const peer = props.session.peer();
      const fusionOn = peer ? await fusionAvailable(peer).catch(() => false) : false;
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
  const currentSel = (): Selection | undefined => (props.session.state.model?.id ? { kind: 'single', key: current() } : undefined);
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
    return shortModelName(m ? modelName(m) : props.session.state.model?.name ?? key);
  };
  /** "Now": the model the new chat will start with. */
  const nowName = () => {
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
          else setOpen(true);
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
            currentKey={current() !== '/' ? current() : undefined}
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
            notice={<PickerError error={error()} />}
            scope={noUsable() ? undefined : 'Your pick applies to the chat about to start. “Use at startup” sets the model new chats start with.'}
            actions={
              <Show when={props.session.state.model?.id && !noUsable()}>
                <PickerSecondary
                  label={data.latest?.startup === current() ? 'Startup model' : 'Use at startup'}
                  disabled={busy() || !data.latest || data.latest.startup === current()}
                  title={data.latest?.startup === current() ? `New chats already start on ${nameOf(current())}` : `Start new chats on ${nameOf(current())} (${current()})`}
                  onClick={() => void useAtStartup(current())}
                  testId="model-use-startup"
                />
              </Show>
            }
          />
        </ModelDialog>
      </Show>
    </>
  );
}
