/**
 * The model dialog for a PC's settings fields (Settings, Cron's job model,
 * the Lead model sheet…): the ONE model dialog the chat and the New-chat draft
 * open — Now line, Single / Fusion tabs, search over every model, the Cycle set
 * filter and ⋯ row menu, "Use at startup" — with a field as its target instead
 * of a chat.
 *
 * It needs no open chat. The catalogue is the PC's own: a live chat's list when
 * one is open on it, else the host's registry (`host_cmd available_models`, the
 * same answer the draft gets), else the last list the app saw. Readiness is the
 * host's `auth_providers`. The cycle set and "Use at startup" keep their GLOBAL
 * meaning (pi's `enabledModels` / `defaultProvider` + `defaultModel`) and are
 * read and written through the host's settings.
 *
 * A row saves the field (`onPick`) and closes; "Use default" clears it. A field
 * stores one model, so the Fusion tab is shown (when Fusion is on) but disabled.
 * Typing a `provider/model` is only the collapsed "Advanced" escape hatch.
 */
import { createMemo, createResource, createSignal, Show, type JSX } from 'solid-js';
import { Plug } from 'lucide-solid';
import { z } from 'zod';
import { hostCmd, listExtensions } from '../../settings/hostCmd';
import { FUSION_EXTENSION_ID, fetchFusion, fusionAvailable, type FusionCurated } from '../../lib/model/fusion';
import { modelKey, modelName, parseModels, shortModelName, type ModelInfo } from '../../lib/model/picker';
import { prettyModelId } from '../../lib/model/displayName';
import { hostReadiness, type Readiness } from '../../lib/model/readiness';
import { openSettingsAt } from '../../lib/settings/open';
import { sessions } from '../../lib/session/store';
import { toasts } from '../../lib/toasts';
import { cachedModels, rememberModels } from '../draft/hostCache';
import { hostSettingsTarget } from './hostTarget';
import { modelView } from './ModelList';
import { startupKey } from './suggested';
import { createCycleSet, fusionStartup, ModeTabs, ModelDialog, ModelPickerPanel, PickerError, PickerSecondary, StartupButton, writeStartupModel } from './ModelPopover';

const valueOf = (reply: unknown): unknown => (reply as { value?: unknown } | undefined)?.value;

/** The PC's models without an open chat: a live chat's list, else the host's registry, else the remembered list. */
export async function loadHostModels(hostId: string): Promise<{ models: ModelInfo[]; peer?: ReturnType<ReturnType<typeof sessions>['peek']> }> {
  const reg = sessions();
  const key = reg.keys().find((k) => k.startsWith(`${hostId}/`));
  const found = key ? reg.peek(hostId, key.slice(hostId.length + 1)) : undefined;
  const peer = found && found.state.conn === 'live' ? found : undefined;
  const reply = peer
    ? await peer.cmd<unknown>({ type: 'get_available_models' }).catch(() => undefined)
    : await hostCmd({ kind: 'available_models' }, z.unknown(), hostId)
        .then((res) => (res.ok ? res.value : undefined))
        .catch(() => undefined);
  const models = reply ? parseModels(reply) : [];
  if (models.length) rememberModels(hostId, models);
  return { models: models.length ? models : (cachedModels(hostId) ?? []), ...(peer ? { peer } : {}) };
}

type Data = { models: ModelInfo[]; readiness?: Readiness; curated?: FusionCurated; startup?: string; cycle?: unknown; fusionTab: boolean };

async function load(hostId: string): Promise<Data> {
  const target = hostSettingsTarget(hostId);
  const read = (key: string) => target.cmd<unknown>({ type: '_oino/get_settings', key }).catch(() => undefined);
  const [{ models, peer }, readiness, extensions, cycle, provider, model] = await Promise.all([
    loadHostModels(hostId),
    hostReadiness(hostId).catch(() => undefined),
    listExtensions(hostId).catch(() => undefined),
    read('enabledModels'),
    read('defaultProvider'),
    read('defaultModel'),
  ]);
  const fusionTab = !!(extensions?.ok && extensions.value.some((e) => e.id === FUSION_EXTENSION_ID && e.enabled));
  const [curated, fusionSettings] = await Promise.all([
    peer && fusionTab ? fusionAvailable(peer).then((on) => (on ? fetchFusion(peer) : undefined)).catch(() => undefined) : Promise.resolve(undefined),
    fusionTab ? target.cmd<unknown>({ type: '_oino/get_extension_settings', namespace: 'fusion' }).catch(() => undefined) : Promise.resolve(undefined),
  ]);
  const startup = fusionStartup(fusionSettings) ?? startupKey(valueOf(provider), valueOf(model));
  return {
    models,
    fusionTab,
    cycle: valueOf(cycle),
    ...(readiness ? { readiness } : {}),
    ...(curated?.curated ? { curated: curated.curated } : {}),
    ...(startup ? { startup } : {}),
  };
}

export function HostModelDialog(props: {
  hostId: string;
  title: string;
  testId: string;
  onClose: () => void;
  trigger?: () => HTMLElement | undefined;
  /** `provider/model` the field holds (empty = unset). */
  value: string;
  /** What an empty value means ("Session model", "Inherit"…). */
  fallback: string;
  /** Saves the field (`undefined` = "Use default"); the dialog closes after. */
  onPick: (key: string | undefined) => void;
  /** Above the panel (the field's preset chips). */
  above?: JSX.Element;
  /** Presets stand in for an empty catalogue. */
  hasPresets?: boolean;
  /** The footer's "Use default" id. */
  defaultTestId?: string;
}) {
  const [data, { mutate }] = createResource(() => props.hostId, load);
  const target = createMemo(() => hostSettingsTarget(props.hostId));
  const [query, setQuery] = createSignal('');
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [typed, setTyped] = createSignal('');
  const value = () => props.value.trim();
  const models = () => data.latest?.models ?? [];
  const known = createMemo(() => (value() ? models().find((m) => modelKey(m) === value()) : undefined));
  const nameOf = (key: string): string => {
    const m = models().find((x) => modelKey(x) === key);
    return shortModelName(m ? modelName(m) : prettyModelId(key));
  };
  const provider = () => (value().includes('/') ? value().slice(0, value().indexOf('/')) : '');
  const now = () => (value() ? `${known() ? modelName(known()!) : prettyModelId(value())}${provider() ? ` · ${provider()}` : ''}` : `Default — ${props.fallback}`);
  const view = createMemo(() =>
    modelView(models(), data.latest?.readiness, data.latest?.curated, value() ? { kind: 'single', key: value() } : undefined, query(), data.latest?.startup),
  );
  const cycle = createCycleSet({ target, stored: () => data.latest?.cycle, loaded: () => !!data.latest, onError: setError });
  const noUsable = () => !data.loading && view().usable.length === 0 && !query().trim() && !props.hasPresets;
  const pick = (key: string | undefined) => {
    props.onPick(key);
    props.onClose();
  };
  const useAtStartup = async (key: string) => {
    if (busy()) return;
    setBusy(true);
    setError(null);
    try {
      await writeStartupModel(target(), key, { effort: data.latest?.curated?.effort[key], fusionOn: data.latest?.fusionTab });
      if (data.latest) mutate({ ...data.latest, startup: key });
      toasts().push(`New chats start on ${nameOf(key)}`, 'success');
    } catch (e) {
      setError(`Couldn’t set the startup model: ${String((e as { message?: unknown })?.message ?? e)}`);
    } finally {
      setBusy(false);
    }
  };
  return (
    <ModelDialog title={props.title} testId={props.testId} onClose={props.onClose} trigger={props.trigger}>
      {props.above}
      <ModelPickerPanel
        view={view()}
        query={query()}
        onQuery={setQuery}
        now={now()}
        tabs={
          <Show when={data.latest?.fusionTab && !noUsable()}>
            <ModeTabs mode="single" onMode={() => {}} fusionDisabled="This setting takes a single model" />
          </Show>
        }
        loading={!!data.loading && !data.latest}
        showEmpty={noUsable()}
        empty={
          <div class="mp-connect" data-testid="host-model-connect">
            <Plug size={18} class="mp-connect-ic" aria-hidden="true" />
            <p class="mp-connect-text">
              {view().unavailable.length || data.latest?.readiness?.providers.some((p) => p.configured)
                ? 'The providers on this PC aren’t working right now.'
                : 'The models show here once a provider is connected.'}
            </p>
            <button
              type="button"
              class="btn btn-secondary"
              onClick={() => {
                props.onClose();
                openSettingsAt('providers', 'sec-providers');
              }}
              data-testid="host-model-connect-open"
            >
              Connect a provider
            </button>
          </div>
        }
        currentKey={value() || undefined}
        startupKey={data.latest?.startup}
        rowActions={
          data.latest
            ? {
                cycle: { has: cycle.has, toggle: (m, on) => void cycle.toggle(modelKey(m), on) },
                startup: { set: (m) => void useAtStartup(modelKey(m)) },
              }
            : undefined
        }
        cycleFilter={data.latest ? { set: cycle } : undefined}
        onSelect={(m) => pick(modelKey(m))}
        notice={<PickerError error={error()} />}
        section={
          <details class="mp-advanced" data-testid="host-model-advanced">
            <summary>Advanced: provider/model</summary>
            <form
              class="mp-advanced-form"
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
                placeholder="provider/model"
                spellcheck={false}
                value={typed()}
                onInput={(e) => setTyped(e.currentTarget.value)}
                data-testid="host-model-advanced-input"
              />
            </form>
          </details>
        }
        scope={`“Use default” leaves it unset: ${props.fallback}. The cycle set and “Use at startup” apply to every chat.`}
        actions={
          <>
            <Show when={value().includes('/') && !!known()}>
              <StartupButton
                key={value()}
                startup={data.latest?.startup}
                busy={busy() || !data.latest}
                name={nameOf(value())}
                onSet={() => void useAtStartup(value())}
                onAlready={() => toasts().push(`New chats already start on ${nameOf(value())}`, 'info')}
              />
            </Show>
            <PickerSecondary
              label="Use default"
              pressed={!value()}
              title={`Leave it unset (${props.fallback})`}
              onClick={() => pick(undefined)}
              testId={props.defaultTestId ?? 'model-use-default'}
            />
          </>
        }
      />
    </ModelDialog>
  );
}
