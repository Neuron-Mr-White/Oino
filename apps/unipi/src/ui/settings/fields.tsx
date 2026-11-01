/**
 * The OSS-2 form engine's row chrome and per-type editors (ui-protocol §7,
 * WP-U11). Every row shows its label, description, hint/badge, a scope badge
 * and "default" or "modified · Reset"; the control follows the §7 table:
 * boolean → switch, enum → segmented (≤ 4 options) or select, multiselect →
 * checkbox list, string → input/textarea, number → stepper with unit,
 * secret → masked + "Replace…", order → drag list (desktop) / up-down (phone),
 * model → picker popover, action → `.btn-secondary`, page → drill-in row,
 * list/dict → repeatable rows, json → CodeMirror JSON.
 */
import { For, Show, createMemo, createSignal, type JSX } from 'solid-js';
import { Dynamic } from 'solid-js/web';
import { Select as KSelect } from '@kobalte/core/select';
import {
  ArrowLeft, ArrowUpDown, Braces, Check, ChevronsUpDown, ChevronRight, Cpu, FileJson, Hash, Info,
  KeyRound, List as ListIcon, ListChecks, ListFilter, Minus, Plus, TextCursorInput, ToggleLeft, Zap,
} from 'lucide-solid';
import { SwitchField } from '../../components/Switch';
import type { Field, FieldType, Json } from './types';
import { fieldAnchor, fieldScope, pageAnchor, storesValue } from './types';
import { fieldEnabled, lookup, type Diagnostic, type WriteLayer } from './semantics';
import { ComplexField } from './complex';

export type EditContext = {
  ns: string;
  /** Effective values in the view scope (conditions read these). */
  values: Record<string, Json>;
  /** Layer writes go to. */
  layer: WriteLayer;
  /** Stored global / project layers (for "modified" and Reset). */
  global: Record<string, Json>;
  project: Record<string, Json>;
  diagnostics: Diagnostic[];
  /** The whole form is read-only (e.g. a disabled extension). */
  readonly?: boolean;
  /** Write one value; `undefined` unsets it in `layer`. */
  set: (field: Field, value: Json | undefined) => void;
  /** Closes an open `page` drill-in (its own Back). */
  closePage?: () => void;
};

const FIELD_ICONS: Record<FieldType, typeof Info> = {
  boolean: ToggleLeft, enum: ListFilter, multiselect: ListChecks, string: TextCursorInput,
  number: Hash, secret: KeyRound, order: ArrowUpDown, model: Cpu, action: Zap, page: ChevronRight,
  list: ListIcon, dict: Braces, json: FileJson, unknown: Info,
};

/** Stored value of `field` in the write layer, if any (→ "modified"). */
export function storedValue(ctx: EditContext, field: Field): Json | undefined {
  const stored = lookup(ctx.layer === 'project' ? ctx.project : ctx.global, field.key);
  return stored === null ? undefined : stored; // a stored null is an unset optional key
}

/** The layer below the write layer at `field.key` (what a default write falls back to). */
export function belowValue(ctx: EditContext, field: Field): Json | undefined {
  const global = lookup(ctx.global, field.key);
  return ctx.layer === 'project' ? global : undefined;
}

function defaultValue(ctx: EditContext, field: Field): Json | undefined {
  const below = belowValue(ctx, field);
  return below !== undefined ? below : field.default;
}

/** First resolve diagnostic for the field (§7.1 invalid stored value). */
export function fieldDiagnostic(ctx: EditContext, field: Field): string | undefined {
  return ctx.diagnostics.find((entry) => entry.key === field.key)?.message;
}

function scopeLabel(field: Field): string | undefined {
  const scope = fieldScope(field);
  return scope === 'both' ? undefined : scope;
}

/** The shared row: icon, label + badge + restart chip, description, meta line, control. */
export function FieldRow(props: {
  field: Field;
  ctx: EditContext;
  control?: JSX.Element;
  /** Long controls (textarea, json, list, order, multiselect) take the full row width. */
  wide?: boolean;
  /** Inline validation message (below the description). */
  error?: string;
  /** The control (or use `control`; both spell the same slot). */
  children?: JSX.Element;
}) {
  const icon = () => FIELD_ICONS[props.field.type] ?? Info;
  const modified = () => storedValue(props.ctx, props.field) !== undefined;
  const hasDefault = () => defaultValue(props.ctx, props.field) !== undefined;
  const diagnostic = () => fieldDiagnostic(props.ctx, props.field);
  return (
    <div
      class="settings-row settings-field"
      id={fieldAnchor(props.ctx.ns, props.field.key)}
      data-testid={`field-${props.field.key}`}
      data-type={props.field.type}
      data-wide={props.wide ? '' : undefined}
    >
      <span class="settings-icon">
        <Dynamic component={icon()} size={18} />
      </span>
      <div class="min-w-0">
        <p class="text-[15px] text-text flex items-center gap-2 flex-wrap">
          <span>{props.field.label}</span>
          <Show when={props.field.badge}>
            <span class="settings-chip" data-testid="field-badge">{props.field.badge}</span>
          </Show>
          <Show when={props.field.restart}>
            <span class="settings-chip settings-chip-quiet" data-testid="field-restart">
              applies after {props.field.restart === 'host' ? 'a host restart' : 'a session restart'}
            </span>
          </Show>
          <Show when={scopeLabel(props.field)}>
            <span class="settings-chip settings-chip-quiet" data-testid="field-scope">{scopeLabel(props.field)} only</span>
          </Show>
        </p>
        <Show when={props.field.description}>
          <p class="text-xs text-text-dim mt-0.5">{props.field.description}</p>
        </Show>
        <Show when={props.field.hint}>
          <p class="settings-field-hint">{props.field.hint}</p>
        </Show>
        <Show when={storesValue(props.field)}>
        <p class="settings-field-meta">
          <Show when={modified()} fallback={
            <Show when={hasDefault()} fallback={<span class="settings-field-unset">not set</span>}>
              <span data-testid="field-default">default</span>
            </Show>
          }>
            <span data-testid="field-modified">modified</span>
            <span class="settings-field-dot">·</span>
            <button
              type="button"
              class="settings-reset"
              data-testid="field-reset"
              disabled={props.ctx.readonly}
              onClick={() => props.ctx.set(props.field, undefined)}
            >
              Reset
            </button>
          </Show>
        </p>
        </Show>
        <Show when={props.error ?? diagnostic()} keyed>
          {(message) => (
            <p class="settings-field-error" role="alert" data-testid="field-error">{message}</p>
          )}
        </Show>
      </div>
      <div class={props.wide ? 'settings-field-wide' : 'settings-field-control'}>{props.control ?? props.children}</div>
    </div>
  );
}

/** One field of a namespace, rendered by type. */
export function SettingsField(props: { field: Field; ctx: EditContext }) {
  const field = () => props.field;
  const ctx = () => props.ctx;
  const disabled = () => !!ctx().readonly || !editable() || !fieldEnabled(field(), ctx().values);
  const editable = () => {
    const scope = fieldScope(field());
    return scope === 'both' || scope === ctx().layer;
  };
  const value = () => lookup(ctx().values, field().key);
  const [page, setPage] = createSignal(false);
  const [error, setError] = createSignal<string | undefined>();

  const set = (next: Json | undefined) => {
    setError(undefined);
    ctx().set(field(), next);
  };

  return (
    <>
      <Show when={field().type !== 'page'} fallback={<PageRow field={field()} ctx={ctx()} open={page()} onToggle={() => setPage((cur) => !cur)} />}>
        <Show
          when={!isComplex(field().type)}
          fallback={<ComplexField field={field()} ctx={ctx()} disabled={disabled()} set={set} error={error()} onError={setError} />}
        >
          <FieldRow field={field()} ctx={ctx()} error={error()}>
            <ScalarControl field={field()} value={value()} disabled={disabled()} set={set} onError={setError} />
          </FieldRow>
        </Show>
      </Show>
      <Show when={field().type === 'page' && page()}>
        <PageBody field={field()} ctx={{ ...ctx(), closePage: () => setPage(false) }} />
      </Show>
    </>
  );
}

const COMPLEX: readonly FieldType[] = ['action', 'list', 'dict', 'json', 'secret', 'order', 'model'];

function isComplex(type: FieldType): boolean {
  return COMPLEX.includes(type);
}

/** Boolean / enum / multiselect-free scalars: switch, segmented, select, input, number. */
function ScalarControl(props: {
  field: Field;
  value: Json;
  disabled: boolean;
  set: (value: Json | undefined) => void;
  onError: (message: string | undefined) => void;
}) {
  const label = props.field.label;
  const asString = () => (typeof props.value === 'string' ? props.value : '');
  const asNumber = () => (typeof props.value === 'number' ? props.value : 0);
  const enumSegmented = () => props.field.display === 'segmented' || (!props.field.display && props.field.options.length <= 4);

  return (
    <>
      <Show when={props.field.type === 'boolean'}>
        <SwitchField
          label={label}
          checked={props.value === true}
          disabled={props.disabled}
          testId={`set-${props.field.key}`}
          onChange={(checked) => props.set(checked)}
        />
      </Show>

      <Show when={props.field.type === 'enum' && enumSegmented()}>
        <div role="radiogroup" aria-label={label} class="flex rounded-pill bg-surface-3 p-1 gap-1">
          <For each={props.field.options}>
            {(option) => (
              <button
                type="button"
                role="radio"
                aria-checked={props.value === option.value}
                class={`px-3 py-1 text-xs rounded-pill transition-colors ${props.value === option.value ? 'bg-surface-2 text-text' : 'text-text-muted'}`}
                disabled={props.disabled}
                onClick={() => props.set(option.value)}
              >
                {option.label}
              </button>
            )}
          </For>
        </div>
      </Show>

      <Show when={props.field.type === 'enum' && !enumSegmented()}>
        <EnumSelect
          field={props.field}
          value={props.value}
          disabled={props.disabled}
          onChange={(next) => props.set(next)}
        />
      </Show>

      <Show when={props.field.type === 'multiselect'}>
        <MultiSelect
          field={props.field}
          value={props.value}
          disabled={props.disabled}
          onChange={(next) => props.set(next)}
        />
      </Show>

      <Show when={props.field.type === 'string' && props.field.multiline}>
        <textarea
          class="settings-textarea"
          aria-label={label}
          rows={3}
          disabled={props.disabled}
          placeholder={props.field.placeholder}
          data-testid={`set-${props.field.key}`}
          value={asString()}
          onInput={(e) => props.set(e.currentTarget.value)}
        />
      </Show>

      <Show when={props.field.type === 'string' && !props.field.multiline}>
        <StringInput
          field={props.field}
          value={asString()}
          disabled={props.disabled}
          set={props.set}
          onError={props.onError}
        />
      </Show>

      <Show when={props.field.type === 'number'}>
        <NumberControl field={props.field} value={asNumber()} disabled={props.disabled} set={props.set} />
      </Show>
    </>
  );
}

/** `number`: one control — decrement, the numeric value with its unit, increment. */
function NumberControl(props: { field: Field; value: number; disabled: boolean; set: (value: Json | undefined) => void }) {
  const step = () => props.field.step ?? 1;
  const min = () => props.field.min ?? Number.NEGATIVE_INFINITY;
  const max = () => props.field.max ?? Number.POSITIVE_INFINITY;
  const clamp = (value: number) => Math.min(max(), Math.max(min(), Math.round(value * 100) / 100));
  return (
    <div class="settings-number" role="group" aria-label={props.field.label}>
      <button
        type="button"
        class="settings-iconbtn"
        aria-label={`Decrease ${props.field.label}`}
        disabled={props.disabled || props.value <= min()}
        onClick={() => props.set(clamp(props.value - step()))}
      >
        <Minus size={15} />
      </button>
      <span class="settings-number-box">
        <input
          type="number"
          class="settings-number-input"
          aria-label={`${props.field.label} value`}
          disabled={props.disabled}
          min={props.field.min}
          max={props.field.max}
          step={step()}
          value={String(props.value)}
          data-testid={`set-${props.field.key}`}
          onChange={(e) => {
            const parsed = Number(e.currentTarget.value);
            if (Number.isFinite(parsed)) props.set(clamp(parsed));
          }}
        />
        <Show when={props.field.unit}>
          <span class="settings-unit">{props.field.unit}</span>
        </Show>
      </span>
      <button
        type="button"
        class="settings-iconbtn"
        aria-label={`Increase ${props.field.label}`}
        disabled={props.disabled || props.value >= max()}
        onClick={() => props.set(clamp(props.value + step()))}
      >
        <Plus size={15} />
      </button>
    </div>
  );
}

/** Text input that validates on input via the field's zod schema (§7.3). */
function StringInput(props: {
  field: Field;
  value: string;
  disabled: boolean;
  set: (value: Json | undefined) => void;
  onError: (message: string | undefined) => void;
}) {
  const [draft, setDraft] = createSignal<string | undefined>();
  const shown = () => draft() ?? props.value;
  return (
    <input
      type={props.field.format === 'url' ? 'url' : 'text'}
      class={`settings-input ${!props.field.multiline ? '' : 'w-full'}`}
      aria-label={props.field.label}
      disabled={props.disabled}
      placeholder={props.field.placeholder}
      spellcheck={false}
      autocomplete="off"
      data-testid={`set-${props.field.key}`}
      value={shown()}
      onInput={(e) => {
        setDraft(e.currentTarget.value);
        props.onError(undefined);
      }}
      onChange={(e) => {
        setDraft(undefined);
        props.set(e.currentTarget.value);
      }}
    />
  );
}

/** Kobalte select for enums with > 4 options or `display: "select"`. */
function EnumSelect(props: { field: Field; value: Json; disabled: boolean; onChange: (value: string) => void }) {
  const current = createMemo(() => props.field.options.find((option) => option.value === props.value) ?? null);
  return (
    <div class="ui-select settings-select">
      <KSelect
        options={props.field.options}
        optionValue="value"
        optionTextValue={(option) => option.label}
        value={current()}
        onChange={(option) => option && props.onChange(option.value)}
        disabled={props.disabled}
        placeholder="Choose…"
        gutter={6}
        sameWidth
        itemComponent={(p) => (
          <KSelect.Item item={p.item} class="ui-select-item">
            <span class="ui-select-item-main">
              <KSelect.ItemLabel class="ui-select-item-label">{p.item.rawValue.label}</KSelect.ItemLabel>
              <Show when={p.item.rawValue.description}>
                <KSelect.ItemDescription class="ui-select-item-desc">{p.item.rawValue.description}</KSelect.ItemDescription>
              </Show>
            </span>
            <KSelect.ItemIndicator class="ui-select-check">
              <Check size={16} aria-hidden="true" />
            </KSelect.ItemIndicator>
          </KSelect.Item>
        )}
      >
        <KSelect.Trigger class="ui-select-trigger" aria-label={props.field.label} data-testid={`set-${props.field.key}`}>
          <KSelect.Value<{ value: string; label: string }> class="ui-select-value">
            {(state) => state.selectedOption()?.label ?? 'Choose…'}
          </KSelect.Value>
          <KSelect.Icon class="ui-select-icon">
            <ChevronsUpDown size={16} aria-hidden="true" />
          </KSelect.Icon>
        </KSelect.Trigger>
        <KSelect.Portal>
          <KSelect.Content class="ui-select-content">
            <KSelect.Listbox class="ui-select-listbox" />
          </KSelect.Content>
        </KSelect.Portal>
      </KSelect>
    </div>
  );
}

/** Checkbox list for `multiselect`; `min` / `max` gate the toggles. */
function MultiSelect(props: { field: Field; value: Json; disabled: boolean; onChange: (value: string[]) => void }) {
  const selected = () => (Array.isArray(props.value) ? (props.value.filter((item) => typeof item === 'string') as string[]) : []);
  const atMax = () => props.field.max !== undefined && selected().length >= props.field.max;
  const atMin = () => props.field.min !== undefined && selected().length <= props.field.min;
  const toggle = (value: string) => {
    const current = selected();
    const next = current.includes(value) ? current.filter((item) => item !== value) : [...current, value];
    props.onChange(next);
  };
  return (
    <ul class="settings-checklist" aria-label={props.field.label} data-testid={`set-${props.field.key}`}>
      <For each={props.field.options}>
        {(option) => {
          const on = () => selected().includes(option.value);
          return (
            <li>
              <label class="settings-check-row" data-disabled={props.disabled ? '' : undefined}>
                <input
                  type="checkbox"
                  class="settings-check"
                  checked={on()}
                  disabled={props.disabled || (on() ? atMin() : atMax())}
                  onChange={() => toggle(option.value)}
                />
                <span class="min-w-0">
                  <span class="block text-sm text-text">{option.label}</span>
                  <Show when={option.description}>
                    <span class="block text-xs text-text-dim">{option.description}</span>
                  </Show>
                </span>
              </label>
            </li>
          );
        }}
      </For>
    </ul>
  );
}

/** `page`: a drill-in row (the whole row opens it); its nested sections render
 *  under a standard Back (Z-113). */
function PageRow(props: { field: Field; ctx: EditContext; open: boolean; onToggle: () => void }) {
  return (
    <button
      type="button"
      class="settings-row settings-field settings-drill-row"
      id={fieldAnchor(props.ctx.ns, props.field.key)}
      data-type="page"
      data-testid={`field-${props.field.key}`}
      aria-expanded={props.open}
      onClick={props.onToggle}
    >
      <span class="settings-icon">
        <ChevronRight size={18} />
      </span>
      <div class="min-w-0">
        <p class="text-[15px] text-text">{props.field.label}</p>
        <Show when={props.field.description}>
          <p class="text-xs text-text-dim mt-0.5">{props.field.description}</p>
        </Show>
      </div>
      <div class="settings-field-control">
        <span class="settings-drill" data-testid={`page-${props.field.key}`}>
          Open
          <ChevronRight size={15} />
        </span>
      </div>
    </button>
  );
}

/** The nested page's rows (rendered by the form when a page row is open). */
function PageBody(props: { field: Field; ctx: EditContext }) {
  return (
    <div class="settings-page-body" id={pageAnchor(props.ctx.ns, props.field.key)} data-testid={`page-body-${props.field.key}`}>
      <div class="settings-page-back">
        <button type="button" class="settings-back" onClick={() => props.ctx.closePage?.()} data-testid={`page-back-${props.field.key}`}>
          <ArrowLeft size={16} /> Back
        </button>
        <span class="settings-page-title">{props.field.label}</span>
      </div>
      <For each={props.field.sections}>
        {(section) => (
          <div>
            <Show when={section.title && section.title !== props.field.label}>
              <div class="settings-in-card-label">{section.title}</div>
            </Show>
            <For each={section.fields}>
              {(inner) => <SettingsField field={inner} ctx={props.ctx} />}
            </For>
          </div>
        )}
      </For>
    </div>
  );
}
