/**
 * The wide editors of the OSS-2 form engine (ui-protocol §7, WP-U11): secret,
 * order, model, action, list, dict and json — the field types whose control
 * takes the full row width.
 */
import { For, Show, createEffect, createResource, createSignal, onCleanup, onMount } from 'solid-js';
import { EditorState } from '@codemirror/state';
import { EditorView, keymap, lineNumbers, placeholder } from '@codemirror/view';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { json as jsonLang } from '@codemirror/lang-json';
import { ChevronDown, ChevronUp, Minus, Plus, Trash2 } from 'lucide-solid';
import { useSettings } from '../../settings/context';
import { setPiSetting, hostCmd } from '../../settings/hostCmd';
import { z } from 'zod';
import { CommandResult, CommandSheet, createSettingsCommand } from './command';
import { commandOverride } from '../../lib/settings/run';
import { ModelPickerField } from './ModelPickerField';
import { desktopShell } from '../../lib/layout';
import type { Field, Json } from './types';
import { jsonEqual, lookup } from './semantics';
import { FieldRow, NumberControl, type EditContext } from './fields';
import { settingsHostId } from '../../settings/target';

const asString = (value: Json): string => (typeof value === 'string' ? value : '');
const asArray = (value: Json): Json[] => (Array.isArray(value) ? value : []);
const asObject = (value: Json): Record<string, Json> =>
  typeof value === 'object' && value !== null && !Array.isArray(value) ? (value as Record<string, Json>) : {};

/** Types whose control takes the full row width (the rest sit on the right). */
const WIDE: readonly Field['type'][] = ['order', 'list', 'dict', 'json'];

/** One complex field: the row chrome plus its own editor. */
export function ComplexField(props: {
  field: Field;
  ctx: EditContext;
  disabled: boolean;
  set: (value: Json | undefined) => void;
  error: string | undefined;
  onError: (message: string | undefined) => void;
}) {
  const field = () => props.field;
  const value = () => lookup(props.ctx.values, field().key);
  return (
    <Show when={field().type !== 'action'} fallback={<ActionField field={field()} ctx={props.ctx} disabled={props.disabled} />}>
    <FieldRow field={field()} ctx={props.ctx} wide={WIDE.includes(field().type)} error={props.error}>
      <Show when={field().type === 'secret'}>
        <SecretField field={field()} value={value()} disabled={props.disabled} set={props.set} onError={props.onError} />
      </Show>
      <Show when={field().type === 'order'}>
        <OrderField field={field()} value={value()} disabled={props.disabled} set={props.set} />
      </Show>
      <Show when={field().type === 'model'}>
        <ModelField field={field()} ctx={props.ctx} value={value()} disabled={props.disabled} set={props.set} />
      </Show>
      <Show when={field().type === 'list'}>
        <ListField field={field()} value={value()} disabled={props.disabled} set={props.set} />
      </Show>
      <Show when={field().type === 'dict'}>
        <DictField field={field()} value={value()} disabled={props.disabled} set={props.set} />
      </Show>
      <Show when={field().type === 'json'}>
        <Show when={shapeOf(field())} fallback={<JsonField field={field()} value={value()} disabled={props.disabled} set={props.set} />}>
          {(shape) => <ShapeField field={field()} shape={shape()} value={value()} disabled={props.disabled} set={props.set} />}
        </Show>
      </Show>
    </FieldRow>
    </Show>
  );
}

/** `secret`: never echoed — a status ("Not set" / "•••• set") and a real button; entering a key opens an input with Save / Cancel. */
function SecretField(props: { field: Field; value: Json; disabled: boolean; set: (value: Json | undefined) => void; onError: (message: string | undefined) => void }) {
  const [replacing, setReplacing] = createSignal(false);
  const [draft, setDraft] = createSignal('');
  const isSet = () => typeof props.value === 'string' && props.value !== '';
  const save = () => {
    if (!draft()) {
      props.onError('Enter a value first');
      return;
    }
    props.onError(undefined);
    props.set(draft());
    setDraft('');
    setReplacing(false);
  };
  const cancel = () => {
    setReplacing(false);
    setDraft('');
    props.onError(undefined);
  };
  return (
    <div class="settings-secret">
      <Show when={!replacing()} fallback={
        <div class="settings-secret-replace">
          <input
            type="password"
            class="settings-input"
            aria-label={`New ${props.field.label}`}
            autocomplete="new-password"
            data-testid={`set-${props.field.key}`}
            value={draft()}
            onInput={(e) => setDraft(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') save();
              if (e.key === 'Escape') cancel();
            }}
            ref={(el) => queueMicrotask(() => el.focus())}
          />
          <button type="button" class="btn-secondary settings-btn" onClick={save} data-testid={`secret-save-${props.field.key}`}>Save</button>
          <button type="button" class="btn-secondary settings-btn" onClick={cancel}>Cancel</button>
        </div>
      }>
        <span class="settings-secret-value" data-set={isSet() ? '' : undefined} data-testid={`secret-${props.field.key}`}>
          {isSet() ? '•••• set' : 'Not set'}
        </span>
        <button type="button" class="btn-secondary settings-btn" disabled={props.disabled} onClick={() => setReplacing(true)} data-testid={`replace-${props.field.key}`}>
          {isSet() ? 'Replace…' : 'Set key…'}
        </button>
        <Show when={isSet()}>
          <button type="button" class="btn-secondary settings-btn" disabled={props.disabled} onClick={() => props.set(undefined)}>Clear</button>
        </Show>
      </Show>
    </div>
  );
}

/** `order`: drag on the desktop, up/down buttons on the phone; `items` is the universe. */
function OrderField(props: { field: Field; value: Json; disabled: boolean; set: (value: Json) => void }) {
  const rows = () => asArray(props.value).filter((item): item is string => typeof item === 'string');
  const universe = () => (props.field.items.length ? props.field.items : rows());
  const missing = () => universe().filter((item) => !rows().includes(item));
  const [dragging, setDragging] = createSignal<number | undefined>();
  const move = (from: number, to: number) => {
    const next = [...rows()];
    if (to < 0 || to >= next.length) return;
    const [item] = next.splice(from, 1);
    if (item === undefined) return;
    next.splice(to, 0, item);
    props.set(next);
  };
  return (
    <div class="settings-order" data-testid={`set-${props.field.key}`}>
      <ul class="settings-order-list">
        <For each={rows()}>
          {(item, index) => (
            <li
              class="settings-order-row"
              draggable={!props.disabled && desktopShell()}
              onDragStart={() => setDragging(index())}
              onDragOver={(e) => e.preventDefault()}
              onDrop={() => {
                const from = dragging();
                setDragging(undefined);
                if (from !== undefined && from !== index()) move(from, index());
              }}
              data-testid={`order-row-${item}`}
            >
              <span class="settings-drag" aria-hidden="true">⠿</span>
              <span class="text-sm flex-1 min-w-0 truncate">{item}</span>
              <button type="button" class="settings-iconbtn" aria-label={`Move ${item} up`} disabled={props.disabled || index() === 0} onClick={() => move(index(), index() - 1)}>
                <ChevronUp size={15} />
              </button>
              <button type="button" class="settings-iconbtn" aria-label={`Move ${item} down`} disabled={props.disabled || index() === rows().length - 1} onClick={() => move(index(), index() + 1)}>
                <ChevronDown size={15} />
              </button>
              <button type="button" class="settings-iconbtn" aria-label={`Remove ${item}`} disabled={props.disabled} onClick={() => props.set(rows().filter((row) => row !== item))}>
                <Minus size={15} />
              </button>
            </li>
          )}
        </For>
      </ul>
      <Show when={missing().length > 0}>
        <div class="settings-add-chips">
          <For each={missing()}>
            {(item) => (
              <button type="button" class="settings-chip-btn" disabled={props.disabled} onClick={() => props.set([...rows(), item])}>
                <Plus size={12} /> {item}
              </button>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}

const ProviderReply = z.object({ value: z.unknown().optional() });
/** The core `defaultModel` stores the bare id; `defaultProvider` sits beside it. */
const pairsProvider = (ctx: EditContext, field: Field): boolean => ctx.ns === 'oino.model' && field.key === 'defaultModel';

/** The empty-value meaning: the field's placeholder, "Inherit" when it says so, else the session model. */
function modelFallback(field: Field): string {
  const holder = field.placeholder?.trim();
  if (holder && !holder.includes('/')) return holder;
  if (/inherit/i.test(`${field.description ?? ''} ${field.hint ?? ''}`)) return 'Inherit';
  if (field.key === 'defaultModel' && /^default/i.test(field.label)) return 'First working model';
  return 'Session model';
}

/** `model`: the composer's model picker as a popover on a button. */
function ModelField(props: { field: Field; ctx: EditContext; value: Json; disabled: boolean; set: (value: Json | undefined) => void }) {
  const hostId = () => props.ctx.hostId ?? settingsHostId();
  const paired = () => pairsProvider(props.ctx, props.field);
  const [bump, setBump] = createSignal(0);
  const [provider] = createResource(
    () => (paired() ? ([hostId(), bump()] as const) : undefined),
    async ([id]) => {
      const res = await hostCmd({ kind: 'get_settings', key: 'defaultProvider' }, ProviderReply, id).catch(() => undefined);
      return res?.ok && typeof res.value.value === 'string' ? res.value.value : '';
    },
  );
  const stored = () => asString(props.value);
  const full = () => {
    const id = stored();
    if (!id || id.includes('/') || !paired()) return id;
    const prov = provider.latest ?? '';
    return prov ? `${prov}/${id}` : id;
  };
  const pick = (key: string | undefined) => {
    if (!paired()) {
      props.set(key);
      return;
    }
    if (!key) {
      props.set(undefined);
      void setPiSetting('defaultProvider', null, 'global', hostId()).then(() => setBump((n) => n + 1));
      return;
    }
    const slash = key.indexOf('/');
    if (slash < 0) {
      props.set(key);
      return;
    }
    props.set(key.slice(slash + 1));
    void setPiSetting('defaultProvider', key.slice(0, slash), 'global', hostId()).then(() => setBump((n) => n + 1));
  };
  return (
    <ModelPickerField
      hostId={hostId()}
      label={props.field.label}
      value={full()}
      fallback={modelFallback(props.field)}
      disabled={props.disabled}
      testId={`set-${props.field.key}`}
      onPick={pick}
    />
  );
}

/**
 * `action`: runs the field's command without leaving Settings (owner5 #5):
 * background commands answer inline under the row's text, UI commands open
 * a Settings sheet, chat-bound ones say "Opens in chat ↗".
 */
function ActionField(props: { field: Field; ctx: EditContext; disabled: boolean }) {
  const [confirming, setConfirming] = createSignal(false);
  const spec = () => ({ label: props.field.label, command: props.field.command ?? '' });
  const cmd = createSettingsCommand(spec, () => props.ctx.hostId ?? settingsHostId());
  const native = () => commandOverride(props.field.command ?? '');
  const run = () => {
    setConfirming(false);
    const open = native();
    if (open) open();
    else void cmd.run();
  };
  return (
    <FieldRow field={props.field} ctx={props.ctx} below={<CommandResult cmd={cmd} testId={`note-${props.field.key}`} />}>
      <div class="settings-action">
        <Show when={!confirming()} fallback={
          <div class="settings-action-confirm">
            <span class="text-xs text-text-muted">{props.field.confirm ?? 'Run this?'}</span>
            <button type="button" class="btn-secondary settings-btn" onClick={run} data-testid={`run-${props.field.key}`}>Run</button>
            <button type="button" class="settings-link" onClick={() => setConfirming(false)}>Cancel</button>
          </div>
        }>
          <button
            type="button"
            class="btn-secondary settings-btn"
            aria-label={`${props.field.label}${cmd.mode() === 'chat' ? ' (opens in chat)' : ''}`}
            disabled={props.disabled || cmd.state().status === 'running'}
            data-testid={`run-${props.field.key}`}
            onClick={() => (props.field.confirm && !native() ? setConfirming(true) : run())}
          >
            {native() ? 'Open…' : cmd.mode() === 'chat' ? 'Open in chat ↗' : cmd.mode() === 'sheet' ? 'Open…' : 'Run'}
          </button>
        </Show>
      </div>
      <Show when={cmd.sheet()}>
        <CommandSheet cmd={cmd} spec={spec()} hostId={props.ctx.hostId ?? settingsHostId()} />
      </Show>
    </FieldRow>
  );
}

/** One repeatable inline editor for `list` items / `dict` values. */
function InlineValue(props: { field: Field; value: Json; disabled: boolean; onChange: (value: Json) => void }) {
  const field = () => props.field;
  const input = (type: string) => (
    <input
      type={type}
      class="settings-input"
      aria-label={field().label}
      disabled={props.disabled}
      value={type === 'number' ? String(props.value ?? '') : asString(props.value)}
      onChange={(e) => props.onChange(type === 'number' ? Number(e.currentTarget.value) : e.currentTarget.value)}
    />
  );
  return (
    <>
      <Show when={field().type === 'boolean'} fallback={<Show when={field().type === 'number'} fallback={input('text')}>{input('number')}</Show>}>
        <input
          type="checkbox"
          class="settings-check"
          aria-label={field().label}
          disabled={props.disabled}
          checked={props.value === true}
          onChange={(e) => props.onChange(e.currentTarget.checked)}
        />
      </Show>
    </>
  );
}

/** `list`: repeatable rows of the item template (§7). */
function ListField(props: { field: Field; value: Json; disabled: boolean; set: (value: Json) => void }) {
  const items = () => asArray(props.value);
  const template = () => props.field.item;
  const change = (index: number, next: Json) => {
    const list = [...items()];
    list[index] = next;
    props.set(list);
  };
  const atMax = () => props.field.max !== undefined && items().length >= props.field.max;
  const atMin = () => props.field.min !== undefined && items().length <= props.field.min;
  return (
    <div class="settings-list" data-testid={`set-${props.field.key}`}>
      <For each={items()}>
        {(item, index) => (
          <div class="settings-list-row">
            <Show when={template()} fallback={<InlineValue field={props.field} value={item} disabled={props.disabled} onChange={(next) => change(index(), next)} />}>
              {(inner) => <InlineValue field={inner()} value={item} disabled={props.disabled} onChange={(next) => change(index(), next)} />}
            </Show>
            <button type="button" class="settings-iconbtn" aria-label={`Remove row ${index() + 1}`} disabled={props.disabled || atMin()} onClick={() => props.set(items().filter((_, at) => at !== index()))}>
              <Trash2 size={15} />
            </button>
          </div>
        )}
      </For>
      <button type="button" class="settings-add" disabled={props.disabled || atMax()} onClick={() => props.set([...items(), emptyOf(template())])} data-testid={`add-${props.field.key}`}>
        <Plus size={14} /> {props.field.addLabel ?? 'Add'}
      </button>
    </div>
  );
}

/** `dict`: key/value rows; `keyPattern` / `keyLabel` shape the key column. */
function DictField(props: { field: Field; value: Json; disabled: boolean; set: (value: Json) => void }) {
  const entries = () => Object.entries(asObject(props.value));
  const rename = (from: string, to: string) => {
    if (!to || to === from) return;
    const next: Record<string, Json> = {};
    for (const [key, value] of entries()) next[key === from ? to : key] = value;
    props.set(next);
  };
  const setValue = (key: string, value: Json) => props.set({ ...asObject(props.value), [key]: value });
  return (
    <div class="settings-list" data-testid={`set-${props.field.key}`}>
      <For each={entries()}>
        {([key, value]) => (
          <div class="settings-list-row">
            <input
              type="text"
              class="settings-input settings-key"
              aria-label={props.field.keyLabel ?? 'Key'}
              value={key}
              disabled={props.disabled}
              onChange={(e) => rename(key, e.currentTarget.value)}
            />
            <Show when={props.field.value} fallback={<InlineValue field={props.field} value={value} disabled={props.disabled} onChange={(next) => setValue(key, next)} />}>
              {(template) => <InlineValue field={template()} value={value} disabled={props.disabled} onChange={(next) => setValue(key, next)} />}
            </Show>
            <button
              type="button"
              class="settings-iconbtn"
              aria-label={`Remove ${key}`}
              disabled={props.disabled}
              onClick={() => {
                const next = { ...asObject(props.value) };
                delete next[key];
                props.set(next);
              }}
            >
              <Trash2 size={15} />
            </button>
          </div>
        )}
      </For>
      <button
        type="button"
        class="settings-add"
        disabled={props.disabled}
        data-testid={`add-${props.field.key}`}
        onClick={() => {
          const next = { ...asObject(props.value) };
          let index = Object.keys(next).length + 1;
          while (`${props.field.keyLabel ?? 'key'}${index}` in next) index += 1;
          next[`${props.field.keyLabel ? 'key' : 'key'}${index}`] = emptyOf(props.field.value);
          props.set(next);
        }}
      >
        <Plus size={14} /> Add
      </button>
    </div>
  );
}

/** The zero value of a template (a new list row / dict entry). */
function emptyOf(field: Field | undefined): Json {
  switch (field?.type) {
    case 'number': return 0;
    case 'boolean': return false;
    case 'list': return [];
    case 'dict': return {};
    default: return '';
  }
}

type ShapeProp = { key: string; label: string; kind: 'number' | 'string' | 'boolean'; min?: number; integer: boolean; unit?: string };

/** Known level keys when a host sends `thinkingBudgets` without a schema (core's `ThinkingBudgets`). */
const KNOWN_SHAPES: Record<string, ShapeProp[]> = {
  thinkingBudgets: ['minimal', 'low', 'medium', 'high'].map((key) => ({
    key, label: key[0]!.toUpperCase() + key.slice(1), kind: 'number' as const, min: 0, integer: true, unit: 'tokens',
  })),
};

/** The typed properties of an object field with a known shape (JSON-schema `properties` of scalars). */
function shapeOf(field: Field): ShapeProp[] | undefined {
  const schema = asObject(field.schema);
  const props = asObject(schema.properties as Json);
  const out: ShapeProp[] = [];
  for (const [key, raw] of Object.entries(props)) {
    const def = asObject(raw);
    const type = def.type;
    const kind = type === 'integer' || type === 'number' ? 'number' : type === 'string' ? 'string' : type === 'boolean' ? 'boolean' : undefined;
    if (!kind) return KNOWN_SHAPES[field.key];
    out.push({
      key,
      label: typeof def.title === 'string' ? def.title : key[0]!.toUpperCase() + key.slice(1),
      kind,
      ...(typeof def.minimum === 'number' ? { min: def.minimum } : {}),
      integer: type === 'integer',
      ...(typeof def.unit === 'string' ? { unit: def.unit } : {}),
    });
  }
  return out.length ? out : KNOWN_SHAPES[field.key];
}

/** A typed editor for an object of known shape: one control per property, blank = default. */
function ShapeField(props: { field: Field; shape: ShapeProp[]; value: Json; disabled: boolean; set: (value: Json | undefined) => void }) {
  const current = () => asObject(props.value);
  const write = (key: string, next: Json | undefined) => {
    const out = { ...current() };
    if (next === undefined || next === '') delete out[key];
    else out[key] = next;
    props.set(Object.keys(out).length ? out : undefined);
  };
  return (
    <div class="settings-shape" data-testid={`set-${props.field.key}`}>
      <For each={props.shape}>
        {(prop) => (
          <div class="settings-shape-row">
            <span class="settings-shape-label">{prop.label}</span>
            <Show when={prop.kind === 'number'}>
              <NumberControl
                field={{ ...props.field, key: `${props.field.key}.${prop.key}`, label: `${props.field.label} · ${prop.label}`, type: 'number', default: undefined, min: prop.min, max: undefined, step: undefined, unit: prop.unit, integer: prop.integer }}
                value={typeof current()[prop.key] === 'number' ? (current()[prop.key] as number) : undefined}
                disabled={props.disabled}
                set={(next) => write(prop.key, next)}
              />
            </Show>
            <Show when={prop.kind === 'string'}>
              <input
                class="settings-input"
                aria-label={`${props.field.label} · ${prop.label}`}
                placeholder="default"
                disabled={props.disabled}
                value={asString(current()[prop.key])}
                onChange={(e) => write(prop.key, e.currentTarget.value.trim())}
              />
            </Show>
            <Show when={prop.kind === 'boolean'}>
              <input
                type="checkbox"
                class="settings-check"
                aria-label={`${props.field.label} · ${prop.label}`}
                disabled={props.disabled}
                checked={current()[prop.key] === true}
                onChange={(e) => write(prop.key, e.currentTarget.checked)}
              />
            </Show>
          </div>
        )}
      </For>
    </div>
  );
}

/** `json`: the escape hatch, edited in a CodeMirror JSON document (§7). */
function JsonField(props: { field: Field; value: Json; disabled: boolean; set: (value: Json) => void }) {
  const [settings] = useSettings();
  const [error, setError] = createSignal<string | undefined>();
  let host!: HTMLDivElement;
  let view: EditorView | undefined;
  // Unset = an empty document (not `null`: the key is simply absent).
  const text = () => (props.value === undefined ? '' : JSON.stringify(props.value, null, 2));

  onMount(() => {
    const state = EditorState.create({
      doc: text(),
      extensions: [
        lineNumbers(),
        history(),
        keymap.of([...defaultKeymap, ...historyKeymap]),
        jsonLang(),
        ...(settings.appearance.codeWrap ? [EditorView.lineWrapping] : []),
        placeholder('Not set — type JSON to add it'),
        EditorView.theme({
          '&': { fontSize: `${settings.appearance.codeFontSize}px`, maxHeight: '220px' },
          '.cm-scroller': { fontFamily: 'var(--font-mono)', overflow: 'auto' },
          '&.cm-focused': { outline: 'none' },
        }),
        EditorView.updateListener.of((update) => {
          if (!update.docChanged) return;
          const doc = update.state.doc.toString();
          if (doc.trim() === '') {
            if (props.value !== undefined) props.set(undefined);
            setError(undefined);
            return;
          }
          try {
            const parsed: Json = JSON.parse(doc);
            if (!jsonEqual(parsed, props.value)) props.set(parsed);
            setError(undefined);
          } catch (e) {
            setError(String((e as Error).message ?? e));
          }
        }),
        EditorState.readOnly.of(!!props.disabled),
      ],
    });
    view = new EditorView({ state, parent: host });
    onCleanup(() => view?.destroy());
  });

  // A write from elsewhere (Reset, scope switch) re-points the document; a
  // document that already parses to the current value is left alone (typing).
  createEffect(() => {
    const next = text();
    if (!view) return;
    const doc = view.state.doc.toString();
    if (doc === next) return;
    try {
      if (jsonEqual(JSON.parse(doc), props.value)) return;
    } catch {
      // An invalid document mid-edit stays as typed.
      return;
    }
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: next } });
  });

  return (
    <div class="settings-json" data-testid={`set-${props.field.key}`}>
      <p class="settings-field-hint settings-json-note">Free-form JSON — no fixed shape.</p>
      <div ref={host} class="settings-json-editor" />
      <Show when={error()} keyed>
        {(message) => <p class="settings-field-error" role="alert" data-testid="field-error">Invalid JSON — {message}</p>}
      </Show>
    </div>
  );
}
