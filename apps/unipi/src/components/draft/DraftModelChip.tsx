/**
 * The draft's model chip (Z-19): looks and sits exactly like the chat's
 * model chip; the chat's picker sheet is session-bound, so the draft opens a
 * compact list instead — this PC's model catalogue (borrowed from a live chat
 * on the same PC when one is open) or, before any chat ran, the default. The
 * pick is applied to the new session before its first prompt.
 */
import { createMemo, createResource, createSignal, onCleanup, Show } from 'solid-js';
import { ChevronUp, Cpu, Plug } from 'lucide-solid';
import { AnchoredMenu } from '../board/AnchoredMenu';
import { modelKey, modelName, parseModels, type ModelInfo, type Selection } from '../../lib/model/picker';
import { fetchFusion, fusionAvailable, type FusionCurated } from '../../lib/model/fusion';
import { openSettingsAt } from '../../lib/settings/open';
import { ModelList, modelView } from '../model/ModelList';
import { startupKey } from '../model/suggested';
import '../model/model.css';
import { modelChipLabel, modelChipTitle } from '../model/ModelChip';
import { RollingLabel } from '../RollingLabel';
import { menuKeys } from '../composer/menuKeys';
import type { DraftSession } from './draftSession';

export function DraftModelChip(props: { session: DraftSession; disabled?: boolean }) {
  const [anchor, setAnchor] = createSignal<DOMRect | undefined>();
  // D-01: no usable model on show → say so, never a name the new chat can't run.
  const label = () => (props.session.state.model?.id ? modelChipLabel(props.session.state.model) : 'Choose a model');
  const current = () => modelKey(props.session.state.model ?? {});
  // F-02 / G-06: the SAME catalogue, readiness and curated ranking as the in-chat dialog (`ModelList`).
  const [data] = createResource(
    () => (anchor() ? props.session.hostId : undefined),
    // The draft answers from a live chat on this PC, else the list it last saw (P0-09).
    async (): Promise<{ models: ModelInfo[]; curated?: FusionCurated; startup?: string }> => {
      const peer = props.session.peer();
      const [reply, fusion, provider, model] = await Promise.all([
        props.session.cmd<unknown>({ type: 'get_available_models' }).catch(() => undefined),
        peer ? fusionAvailable(peer).then((on) => (on ? fetchFusion(peer) : undefined)).catch(() => undefined) : Promise.resolve(undefined),
        peer ? peer.cmd<{ value?: unknown }>({ type: '_oino/get_settings', key: 'defaultProvider' }).catch(() => undefined) : Promise.resolve(undefined),
        peer ? peer.cmd<{ value?: unknown }>({ type: '_oino/get_settings', key: 'defaultModel' }).catch(() => undefined) : Promise.resolve(undefined),
      ]);
      const startup = startupKey(provider?.value, model?.value);
      return { models: parseModels(reply), ...(fusion?.curated ? { curated: fusion.curated } : {}), ...(startup ? { startup } : {}) };
    },
  );
  const models = () => data.latest?.models;
  const [query, setQuery] = createSignal('');
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
  const pick = (m: ModelInfo) => {
    setAnchor(undefined);
    props.session.pickModel({ provider: m.provider ?? '', modelId: m.id, name: modelName(m) });
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
          if (anchor()) setAnchor(undefined);
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
          <AnchoredMenu anchor={rect()} alignEnd width={340} label="Model" onDismiss={() => setAnchor(undefined)}>
            <div
              class="ws-menu dm-menu"
              data-testid="draft-model-menu"
              ref={(el) => onCleanup(menuKeys(el, { close: () => setAnchor(undefined), trigger: () => chip, autoFocus: false }))}
            >
              <input
                class="dm-search"
                type="search"
                placeholder="Search models"
                aria-label="Search models"
                value={query()}
                onInput={(e) => setQuery(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') {
                    const first = view().suggested[0]?.model ?? view().rest[0];
                    if (first) pick(first);
                  }
                }}
                ref={(el) => queueMicrotask(() => el.focus())}
              />
              <div class="ws-label">Model for this chat</div>
              <Show when={!data.loading || data.latest} fallback={<div class="ws-browse-note">Loading…</div>}>
                <Show
                  when={!noUsable()}
                  fallback={
                    <div class="dm-connect" data-testid="draft-model-connect">
                      <Plug size={18} class="dm-connect-ic" aria-hidden="true" />
                      <p class="dm-connect-text">
                        {view().unavailable.length || props.session.readiness()?.providers.some((p) => p.configured)
                          ? 'The providers on this PC aren’t working right now.'
                          : (models() ?? []).length || props.session.readiness()?.known
                            ? 'No model provider is connected yet.'
                            : 'The models show here once a provider is connected.'}
                      </p>
                      <button
                        type="button"
                        class="btn btn-secondary"
                        onClick={() => {
                          setAnchor(undefined);
                          openSettingsAt('providers', 'sec-providers');
                        }}
                        data-testid="draft-model-connect-open"
                      >
                        Connect a provider
                      </button>
                    </div>
                  }
                >
                  <ModelList
                    view={view()}
                    query={query()}
                    currentKey={current()}
                    startupKey={data.latest?.startup}
                    onSelect={(m) => pick(m)}
                  />
                </Show>
              </Show>
            </div>
          </AnchoredMenu>
        )}
      </Show>
    </>
  );
}
