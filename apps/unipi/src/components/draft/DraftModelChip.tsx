/**
 * The draft's model chip (Z-19): looks and sits exactly like the chat's
 * model chip and opens the SAME model popover (`ModelPopover`): this PC's
 * catalogue (a live chat's list on the same PC, else the host's), readiness,
 * the curated ranking. The pick is applied to the new session before its
 * first prompt. Footer: "Use at startup" (the model every new chat starts
 * with — the host's `defaultProvider` / `defaultModel`).
 */
import { createMemo, createResource, createSignal, Show } from 'solid-js';
import { ChevronUp, Cpu, Plug, Star } from 'lucide-solid';
import { modelKey, modelName, parseModels, shortModelName, type ModelInfo, type Selection } from '../../lib/model/picker';
import { fetchFusion, fusionAvailable, type FusionCurated } from '../../lib/model/fusion';
import { openSettingsAt } from '../../lib/settings/open';
import { toasts } from '../../lib/toasts';
import { modelView } from '../model/ModelList';
import { startupKey } from '../model/suggested';
import { ModelPickerPanel, ModelPopover, PickerFootRow, writeStartupModel } from '../model/ModelPopover';
import { modelChipLabel, modelChipTitle } from '../model/ModelChip';
import { RollingLabel } from '../RollingLabel';
import type { DraftSession } from './draftSession';

type DraftData = { models: ModelInfo[]; curated?: FusionCurated; startup?: string; fusionOn?: boolean };

const valueOf = (reply: unknown): unknown => (reply as { value?: unknown } | undefined)?.value;

export function DraftModelChip(props: { session: DraftSession; disabled?: boolean }) {
  const [anchor, setAnchor] = createSignal<DOMRect | undefined>();
  // D-01: no usable model on show → say so, never a name the new chat can't run.
  const label = () => (props.session.state.model?.id ? modelChipLabel(props.session.state.model) : 'Choose a model');
  const current = () => modelKey(props.session.state.model ?? {});
  // F-02 / G-06: the SAME catalogue, readiness and curated ranking as the in-chat picker.
  const [data, { mutate }] = createResource(
    () => (anchor() ? props.session.hostId : undefined),
    // The draft answers from a live chat on this PC, else the host's list (P0-09).
    async (): Promise<DraftData> => {
      const peer = props.session.peer();
      const fusionOn = peer ? await fusionAvailable(peer).catch(() => false) : false;
      const [reply, fusion, provider, model] = await Promise.all([
        props.session.cmd<unknown>({ type: 'get_available_models' }).catch(() => undefined),
        peer && fusionOn ? fetchFusion(peer).catch(() => undefined) : Promise.resolve(undefined),
        // The draft's `_oino/get_settings` reads the host's settings (no chat needed).
        props.session.cmd<unknown>({ type: '_oino/get_settings', key: 'defaultProvider' }).catch(() => undefined),
        props.session.cmd<unknown>({ type: '_oino/get_settings', key: 'defaultModel' }).catch(() => undefined),
      ]);
      const startup = startupKey(valueOf(provider), valueOf(model));
      return {
        models: parseModels(reply),
        ...(fusion?.curated ? { curated: fusion.curated } : {}),
        ...(startup ? { startup } : {}),
        fusionOn,
      };
    },
  );
  const models = () => data.latest?.models;
  const [query, setQuery] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  let chip: HTMLButtonElement | undefined;
  const currentSel = (): Selection | undefined => (props.session.state.model?.id ? { kind: 'single', key: current() } : undefined);
  const view = createMemo(() => {
    const all = models() ?? [];
    const m = props.session.state.model;
    // Before any list is known the default model still shows (never a name the user can't see).
    const rows = all.length || !m?.id ? all : [{ provider: m.provider ?? '', id: m.id, name: m.name ?? m.id } as ModelInfo];
    return modelView(rows, props.session.readiness(), data.latest?.curated, currentSel(), query(), data.latest?.startup);
  });
  const noUsable = () => !data.loading && view().usable.length === 0 && !query().trim();
  const close = () => setAnchor(undefined);
  const pick = (m: ModelInfo) => {
    close();
    props.session.pickModel({ provider: m.provider ?? '', modelId: m.id, name: modelName(m) });
  };
  const nameOf = (key: string) => {
    const m = (models() ?? []).find((x) => modelKey(x) === key);
    return shortModelName(m ? modelName(m) : props.session.state.model?.name ?? key);
  };
  const useAtStartup = async (key: string) => {
    if (busy()) return;
    setBusy(true);
    try {
      await writeStartupModel(props.session, key, { effort: data.latest?.curated?.effort[key], fusionOn: data.latest?.fusionOn });
      if (data.latest) mutate({ ...data.latest, startup: key });
      toasts().push(`New chats start on ${nameOf(key)}`, 'success');
    } catch (e) {
      toasts().push(`Couldn’t set the startup model: ${String((e as { message?: unknown })?.message ?? e)}`, 'error');
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
        aria-haspopup="menu"
        aria-expanded={!!anchor()}
        title={modelChipTitle(props.session.state.model)}
        disabled={props.disabled}
        ref={chip}
        onClick={(e) => {
          setQuery('');
          if (anchor()) close();
          else setAnchor((e.currentTarget as HTMLElement).getBoundingClientRect());
        }}
        data-testid="model-chip"
      >
        <Cpu size={15} class="tb-ic" />
        <RollingLabel class="tb-label" text={label()} testId="model-chip-label" />
        <ChevronUp size={12} class="tb-chev" />
      </button>
      <Show when={anchor()}>
        {(rect) => (
          <ModelPopover anchor={rect()} label="Model" testId="draft-model-menu" onClose={close} trigger={() => chip}>
            <ModelPickerPanel
              view={view()}
              query={query()}
              onQuery={setQuery}
              loading={!!data.loading && !data.latest}
              header={<p class="mpop-label">Model for this chat</p>}
              showEmpty={noUsable()}
              empty={
                  <div class="mpop-connect" data-testid="draft-model-connect">
                    <Plug size={18} class="mpop-connect-ic" aria-hidden="true" />
                    <p class="mpop-connect-text">
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
              onSelect={(m) => pick(m)}
              footer={
                <Show when={props.session.state.model?.id && !noUsable()}>
                  <PickerFootRow
                    icon={<Star size={15} />}
                    label={data.latest?.startup === current() ? 'Startup model' : 'Use at startup'}
                    detail={data.latest?.startup === current() ? 'New chats start on it' : `New chats start on ${nameOf(current())}`}
                    disabled={busy() || !data.latest || data.latest.startup === current()}
                    onSelect={() => void useAtStartup(current())}
                    testId="model-use-startup"
                  />
                </Show>
              }
            />
          </ModelPopover>
        )}
      </Show>
    </>
  );
}
