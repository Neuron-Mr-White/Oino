/**
 * The settings model control: the SAME picker the composer's model chip opens
 * (search, Suggested group, provider rows, readiness — dead-key providers in the
 * collapsed Unavailable group), as a popover anchored to the field's button.
 * The button shows the model's display name + provider; an empty value shows
 * the field's documented fallback ("Session model" / "Inherit") and the menu
 * has a "Use default" row. Typing an id is only an escape for models the
 * catalogue can't list.
 */
import { Show, createMemo, createResource, createSignal } from 'solid-js';
import { Check, ChevronDown, Cpu, RotateCcw } from 'lucide-solid';
import { AnchoredMenu } from '../../components/board/AnchoredMenu';
import { ModelList, modelView } from '../../components/model/ModelList';
import { cachedModels, rememberModels } from '../../components/draft/hostCache';
import { startupKey } from '../../components/model/suggested';
import { fetchFusion, fusionAvailable, type FusionCurated } from '../../lib/model/fusion';
import { modelKey, modelName, parseModels, type ModelInfo } from '../../lib/model/picker';
import { prettyModelId } from '../../lib/model/displayName';
import { hostReadiness, type Readiness } from '../../lib/model/readiness';
import { sessions } from '../../lib/session/store';
import '../../components/model/model.css';
import '../../components/draft/draft.css';

type Data = { models: ModelInfo[]; readiness?: Readiness; curated?: FusionCurated };

/** Models of a PC: a live chat's list when one is open, else the last list the app saw. */
async function loadModels(hostId: string): Promise<Data> {
  const reg = sessions();
  const key = reg.keys().find((k) => k.startsWith(`${hostId}/`));
  const peer = key ? reg.peek(hostId, key.slice(hostId.length + 1)) : undefined;
  const live = peer && peer.state.conn === 'live';
  const [reply, readiness, curated] = await Promise.all([
    live ? peer.cmd<unknown>({ type: 'get_available_models' }).catch(() => undefined) : Promise.resolve(undefined),
    hostReadiness(hostId).catch(() => undefined),
    live ? fusionAvailable(peer).then((on) => (on ? fetchFusion(peer) : undefined)).catch(() => undefined) : Promise.resolve(undefined),
  ]);
  const found = reply ? parseModels(reply) : [];
  if (found.length) rememberModels(hostId, found);
  return {
    models: found.length ? found : (cachedModels(hostId) ?? []),
    ...(readiness ? { readiness } : {}),
    ...(curated?.curated ? { curated: curated.curated } : {}),
  };
}

export function ModelPickerField(props: {
  hostId: string;
  label: string;
  /** `provider/model` (empty = unset). */
  value: string;
  /** What an empty value means ("Session model", "Inherit"…). */
  fallback: string;
  disabled?: boolean;
  testId: string;
  onPick: (key: string | undefined) => void;
}) {
  const [anchor, setAnchor] = createSignal<DOMRect | undefined>();
  const [query, setQuery] = createSignal('');
  const [typed, setTyped] = createSignal('');
  const [data] = createResource(() => props.hostId, loadModels);
  const known = createMemo(() => {
    const value = props.value.trim();
    if (!value) return undefined;
    return (data.latest?.models ?? cachedModels(props.hostId) ?? []).find((m) => modelKey(m) === value);
  });
  const name = () => {
    const value = props.value.trim();
    if (!value) return props.fallback;
    const model = known();
    return model ? modelName(model) : prettyModelId(value);
  };
  const provider = () => {
    const value = props.value.trim();
    return value.includes('/') ? value.slice(0, value.indexOf('/')) : '';
  };
  const view = createMemo(() =>
    modelView(
      data.latest?.models ?? [],
      data.latest?.readiness,
      data.latest?.curated,
      props.value.trim() ? { kind: 'single', key: props.value.trim() } : undefined,
      query(),
      startupKey(undefined, undefined),
    ),
  );
  const close = () => {
    setAnchor(undefined);
    setTyped('');
  };
  const pick = (key: string | undefined) => {
    props.onPick(key);
    close();
  };
  return (
    <>
      <button
        type="button"
        class="settings-model-btn"
        aria-label={`${props.label}: ${name()}. Change`}
        aria-haspopup="menu"
        aria-expanded={!!anchor()}
        disabled={props.disabled}
        data-empty={props.value.trim() ? undefined : ''}
        data-testid={props.testId}
        onClick={(e) => {
          if (anchor()) return close();
          setQuery('');
          setAnchor((e.currentTarget as HTMLElement).getBoundingClientRect());
        }}
      >
        <Cpu size={14} class="settings-model-ic" aria-hidden="true" />
        <span class="settings-model-name">{name()}</span>
        <Show when={provider()}>
          <span class="settings-model-prov">{provider()}</span>
        </Show>
        <ChevronDown size={14} class="settings-model-chev" aria-hidden="true" />
      </button>
      <Show when={anchor()}>
        {(rect) => (
          <AnchoredMenu anchor={rect()} alignEnd width={340} label={props.label} onDismiss={close}>
            <div class="ws-menu dm-menu" data-testid={`model-pop-${props.testId}`}>
              <input
                type="search"
                class="dm-search"
                placeholder="Search models"
                aria-label="Search models"
                value={query()}
                onInput={(e) => setQuery(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') {
                    const first = view().suggested[0]?.model ?? view().rest[0];
                    if (first) pick(modelKey(first));
                  }
                }}
                ref={(el) => queueMicrotask(() => el.focus())}
              />
              <button
                type="button"
                role="menuitemradio"
                aria-checked={!props.value.trim()}
                class="ws-row"
                onClick={() => pick(undefined)}
                data-testid="model-use-default"
              >
                <RotateCcw size={15} class="ws-row-ic" />
                <span class="ws-row-text">
                  <span class="ws-row-label">Use default</span>
                  <span class="ws-row-detail">{props.fallback}</span>
                </span>
                <Show when={!props.value.trim()}>
                  <Check size={15} class="ws-row-check" />
                </Show>
              </button>
              <Show when={!data.loading || data.latest} fallback={<div class="ws-browse-note">Loading…</div>}>
                <Show
                  when={(data.latest?.models.length ?? 0) > 0}
                  fallback={
                    <div class="ws-browse-note">
                      The model list shows once a chat is open on this PC. You can enter an id below.
                    </div>
                  }
                >
                  <ModelList
                    view={view()}
                    query={query()}
                    currentKey={props.value.trim() || undefined}
                    onSelect={(m) => pick(modelKey(m))}
                  />
                </Show>
              </Show>
              <Show when={(data.latest?.models.length ?? 0) === 0}>
                <form
                  class="settings-model-advanced"
                  onSubmit={(e) => {
                    e.preventDefault();
                    const id = typed().trim();
                    if (id) pick(id);
                  }}
                >
                  <input
                    class="dm-search"
                    aria-label="Advanced: model id"
                    placeholder="Advanced: provider/model"
                    spellcheck={false}
                    value={typed()}
                    onInput={(e) => setTyped(e.currentTarget.value)}
                  />
                </form>
              </Show>
            </div>
          </AnchoredMenu>
        )}
      </Show>
    </>
  );
}
