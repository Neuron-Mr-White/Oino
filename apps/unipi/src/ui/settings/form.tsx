/**
 * The settings form engine (ui-protocol §7, WP-U11): one namespace → its
 * sections and fields, rendered in the one Settings screen. Values resolve
 * defaults → global → project in the app (same rules as core), the scope
 * toggle in the header picks the write layer when any field is `both` /
 * `project`, `visibleIf` / `disabledIf` run on the effective values, and every
 * row shows "default" or "modified · Reset" plus its scope badge.
 *
 * Writes go to `_oino/set_extension_setting` (simplified: writing the inherited
 * value unsets the key), so a disabled extension keeps its stored values.
 */
import { For, Show, createMemo, createResource, createSignal, type JSX } from 'solid-js';
import type { AppError } from '@unipi/protocol';
import { RefreshCw } from 'lucide-solid';
import { ErrorBanner } from '../../components/ErrorBanner';
import type { Field, Json, Namespace, Section } from './types';
import { appHidden, fieldScope, storesValue } from './types';
import { fieldVisible, resolve, setPath, removePath, checkValue, zodMessage, toZod, type WriteLayer } from './semantics';
import { loadNamespaceLayers, simplifyWrite, writeFieldValue, type NamespaceLayers } from './data';
import { FieldRow, SettingsField, type EditContext } from './fields';
import './settings-form.css';
import { settingsHostId } from '../../settings/target';

const emptyLayers = (namespace: Namespace): NamespaceLayers => ({ defaults: namespace.defaults, global: {}, project: {}, diagnostics: [] });

/** Whether any field can be written in the project layer (drives the scope toggle). */
function hasProjectScope(namespace: Namespace): boolean {
  return namespace.sections.some((section) =>
    section.fields.some((field) => storesValue(field) && fieldScope(field) !== 'global'),
  );
}

/** Whether any field is behind "Advanced". */
function hasAdvanced(namespace: Namespace): boolean {
  return namespace.sections.some((section) => section.advanced || section.fields.some((field) => field.advanced || field.type === 'page'));
}

function sectionAdvanced(section: Section): boolean {
  return section.advanced;
}

/** Copies a layer object (writes stay immutable so Solid sees the change). */
const clone = <T extends Record<string, Json>>(value: T): T => JSON.parse(JSON.stringify(value)) as T;

export function NamespaceForm(props: {
  namespace: Namespace;
  hostId?: string;
  /** Rows above the first section (the per-extension enable toggle, §7). */
  header?: JSX.Element;
  /** Rendered inside the card, above the fields. */
  notice?: JSX.Element;
  /** The page header already carries the title. */
  bare?: boolean;
  /** Rows without their own card (inside an existing Settings card). */
  flat?: boolean;
  /** Whole namespace read-only (e.g. disabled extension's values). */
  readonly?: boolean;
  /** Called after every successful write (the Extensions list refreshes its rows). */
  onChanged?: () => void;
  testId?: string;
}) {
  const hostId = () => props.hostId ?? settingsHostId();
  const [error, setError] = createSignal<AppError | undefined>();
  const [layers, setLayers] = createSignal<NamespaceLayers | undefined>();
  const [scope, setScope] = createSignal<WriteLayer>('global');
  const [advanced, setAdvanced] = createSignal(false);
  const [busy, setBusy] = createSignal<string | undefined>();

  const [loaded] = createResource(
    () => props.namespace,
    async (namespace): Promise<NamespaceLayers> => {
      const reply = await loadNamespaceLayers(namespace, hostId());
      if (!reply.ok) {
        setError(reply.error);
        return emptyLayers(namespace);
      }
      setError(undefined);
      setScope('global');
      setLayers(reply.value);
      return reply.value;
    },
  );

  const current = () => layers() ?? loaded.latest ?? emptyLayers(props.namespace);

  /** Effective values of the view scope (project view includes the global layer). */
  const values = createMemo<Record<string, Json>>(() => {
    const stored = current();
    const result = resolve(props.namespace, {
      defaults: stored.defaults,
      global: stored.global,
      project: scope() === 'project' ? stored.project : {},
    });
    return result.values;
  });

  const diagnostics = () => current().diagnostics;

  /** One write: validate, simplify, apply locally, then persist. */
  const write = async (field: Field, value: Json | undefined) => {
    if (props.readonly) return;
    if (value !== undefined) {
      const message = checkValue(field, value) ?? zodMessage(toZod(field), value);
      if (message) {
        setError({ code: 'E_SCHEMA', message: `${field.label}: ${message}`, retryable: false } as AppError);
        return;
      }
    }
    const layer = field.scope === 'project' ? 'project' : field.scope === 'global' ? 'global' : scope();
    const before = current();
    // The stored value the write sends (§7.2): the inherited value unsets the key.
    const stored = simplifyWrite(field, value, layer, { global: before.global, project: before.project });
    const next: NamespaceLayers = { ...before, global: clone(before.global), project: clone(before.project) };
    const target = layer === 'project' ? next.project : next.global;
    if (stored === undefined) removePath(target, field.key);
    else setPath(target, field.key, stored);
    setLayers(next);
    setBusy(field.key);
    const reply = await writeFieldValue(props.namespace, field, stored, layer, hostId());
    setBusy(undefined);
    if (!reply.ok) {
      setLayers(before);
      setError(reply.error);
      return;
    }
    setError(undefined);
    props.onChanged?.();
  };

  const ctx = (): EditContext => ({
    ns: props.namespace.namespace,
    values: values(),
    layer: scope(),
    global: current().global,
    project: current().project,
    diagnostics: diagnostics(),
    readonly: props.readonly,
    hostId: hostId(),
    set: (field, value) => void write(field, value),
  });

  const fieldsOf = (section: Section) => section.fields.filter((field) => !appHidden(props.namespace.namespace, field.key) && fieldVisible(field, values()) && (advanced() || !(field.advanced ?? false)));

  return (
    <section
      class="settings-form"
      data-testid={props.testId ?? `form-${props.namespace.namespace}`}
      data-namespace={props.namespace.namespace}
      data-loaded={layers() ? '' : undefined}
    >
      <Show when={!props.bare}>
        <div class="settings-form-head">
          <div class="min-w-0">
            <h3 class="settings-form-title">{props.namespace.title}</h3>
            <Show when={props.namespace.description}>
              <p class="settings-form-desc">{props.namespace.description}</p>
            </Show>
          </div>
          <Show when={hasProjectScope(props.namespace)}>
            <ScopeToggle value={scope()} onChange={(next) => setScope(next)} />
          </Show>
        </div>
      </Show>
      <div class={props.flat ? '' : 'settings-group rounded-card bg-surface-1'}>
        <Show when={props.header}>{props.header}</Show>
        <Show when={props.notice}>{props.notice}</Show>
        <Show when={error()} keyed>
          {(err) => (
            <div class="p-3">
              <ErrorBanner error={err} dismiss={() => setError(undefined)} />
            </div>
          )}
        </Show>
        <Show when={loaded.loading && !layers()}>
          <FieldRow field={skeletonField()} ctx={ctx()} control={<span class="settings-loading">Loading…</span>} />
        </Show>
        <For each={props.namespace.sections}>
          {(section) => (
            <Show when={!sectionAdvanced(section) || advanced()}>
              <Show when={fieldsOf(section).length > 0 || section.title}>
                <div class="settings-form-section" data-section-id={section.id}>
                  <Show when={section.title && section.title !== props.namespace.title}>
                    <div class="settings-in-card-label">
                      {section.title}
                      <Show when={section.description}>
                        <span class="settings-form-section-desc">{section.description}</span>
                      </Show>
                    </div>
                  </Show>
                  <For each={fieldsOf(section)}>
                    {(field) => <SettingsField field={field} ctx={ctx()} />}
                  </For>
                </div>
              </Show>
            </Show>
          )}
        </For>
        <Show when={hasAdvanced(props.namespace)}>
          <div class="settings-form-foot">
            <button type="button" class="settings-link" aria-pressed={advanced()} onClick={() => setAdvanced((cur) => !cur)} data-testid="form-advanced">
              {advanced() ? 'Hide advanced' : 'Show advanced'}
            </button>
            <Show when={busy()}>
              <span class="settings-saving" data-testid="form-saving"><RefreshCw size={12} /> Saving…</span>
            </Show>
          </div>
        </Show>
      </div>
    </section>
  );
}

/** Global | Project (§7): which layer `both` fields write to. */
export function ScopeToggle(props: { value: WriteLayer; onChange: (value: WriteLayer) => void }) {
  return (
    <div role="radiogroup" aria-label="Setting scope" class="settings-scope" data-testid="scope-toggle">
      <For each={['global', 'project'] as const}>
        {(option) => (
          <button
            type="button"
            role="radio"
            aria-checked={props.value === option}
            class="settings-scope-option"
            data-testid={`scope-${option}`}
            onClick={() => props.onChange(option)}
          >
            {option === 'global' ? 'Global' : 'Project'}
          </button>
        )}
      </For>
    </div>
  );
}

function skeletonField(): Field {
  return {
    key: 'loading',
    type: 'string',
    label: 'Reading settings…',
    scope: 'both',
    advanced: false,
    options: [],
    allowCustom: false,
    integer: false,
    multiline: false,
    items: [],
    sections: [],
  };
}
