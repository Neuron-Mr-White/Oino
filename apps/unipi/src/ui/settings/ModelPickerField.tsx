/**
 * The settings model control: the ONE model dialog (`ModelDialog` +
 * `ModelPickerPanel`) the composer's model chips open — titled by the
 * field's label, with its Now line, full-width search, the shared list
 * (Suggested group, provider rows, readiness — dead-key providers in the
 * collapsed Unavailable group) and "Use default" in the footer (no Fusion
 * tab, Cycle set or "Use at startup": those belong to a chat). The button
 * shows the model's display name + provider; an empty value shows the
 * field's documented fallback ("Session model" / "Inherit"). Typing an id is
 * only an escape for models the catalogue can't list.
 */
import { Show, createMemo, createResource, createSignal } from 'solid-js';
import { ChevronDown, Cpu } from 'lucide-solid';
import { modelView } from '../../components/model/ModelList';
import { ModelDialog, ModelPickerPanel, PickerSecondary } from '../../components/model/ModelPopover';
import { cachedModels, rememberModels } from '../../components/draft/hostCache';
import { startupKey } from '../../components/model/suggested';
import { fetchFusion, fusionAvailable, type FusionCurated } from '../../lib/model/fusion';
import { modelKey, modelName, parseModels, type ModelInfo } from '../../lib/model/picker';
import { prettyModelId } from '../../lib/model/displayName';
import { hostReadiness, type Readiness } from '../../lib/model/readiness';
import { sessions } from '../../lib/session/store';

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
  const [open, setOpen] = createSignal(false);
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
    setOpen(false);
    setTyped('');
  };
  let btn: HTMLButtonElement | undefined;
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
        aria-haspopup="dialog"
        aria-expanded={open()}
        disabled={props.disabled}
        data-empty={props.value.trim() ? undefined : ''}
        data-testid={props.testId}
        ref={btn}
        onClick={() => {
          if (open()) return close();
          setQuery('');
          setOpen(true);
        }}
      >
        <Cpu size={14} class="settings-model-ic" aria-hidden="true" />
        <span class="settings-model-name">{name()}</span>
        <Show when={provider()}>
          <span class="settings-model-prov">{provider()}</span>
        </Show>
        <ChevronDown size={14} class="settings-model-chev" aria-hidden="true" />
      </button>
      <Show when={open()}>
        <ModelDialog title={props.label} testId={`model-pop-${props.testId}`} onClose={close} trigger={() => btn}>
          <ModelPickerPanel
            view={view()}
            query={query()}
            onQuery={setQuery}
            now={props.value.trim() ? `${name()}${provider() ? ` · ${provider()}` : ''}` : `Default — ${props.fallback}`}
            loading={!!data.loading && !data.latest}
            showEmpty={(data.latest?.models.length ?? 0) === 0}
            empty={
              <div class="mp-empty-note">
                <p class="mp-hint">The model list shows once a chat is open on this PC. You can enter an id below.</p>
                <form
                  class="settings-model-advanced"
                  onSubmit={(e) => {
                    e.preventDefault();
                    const id = typed().trim();
                    if (id) pick(id);
                  }}
                >
                  <input
                    class="mp-search"
                    type="text"
                    aria-label="Advanced: model id"
                    placeholder="Advanced: provider/model"
                    spellcheck={false}
                    value={typed()}
                    onInput={(e) => setTyped(e.currentTarget.value)}
                  />
                </form>
              </div>
            }
            currentKey={props.value.trim() || undefined}
            onSelect={(m) => pick(modelKey(m))}
            scope={`“Use default” leaves it unset: ${props.fallback}.`}
            actions={
              <PickerSecondary
                label="Use default"
                pressed={!props.value.trim()}
                title={`Leave it unset (${props.fallback})`}
                onClick={() => pick(undefined)}
                testId="model-use-default"
              />
            }
          />
        </ModelDialog>
      </Show>
    </>
  );
}
