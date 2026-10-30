/**
 * Oino dialog bodies WP-U10 widens behind A9's `DialogPanel` (G-2):
 *
 * - `multiSelect` (v1 §4.3): checkbox rows (`selected` preseed), `min`/`max`
 *   clamps, answer `string[]` of values (or labels);
 * - `custom`: the `spec` node tree through `NodeView`; the first submitting
 *   action (a `Button`, or an `onSubmit` input) resolves the dialog with
 *   `{actionId, value?, values?}` (ui-protocol §4.6), other actions ride as
 *   events only;
 * - `editExternally` (v1 U4): the in-app editor sheet body — the text in a
 *   monospace area (max 50 vh), Send answers the edited text, ✕ cancels;
 * - `clipboardReadImage`: reads an image from the clipboard (Web Clipboard
 *   API; the desktop app's plugin lands with the Tauri build) and answers
 *   `{data, mimeType}` — or explains why it can't.
 *
 * All inside the same `.dlg` card chrome the pi kinds use (§3.2 table).
 */
import { createMemo, createSignal, For, Index, Match, Show, Switch } from 'solid-js';
import { Check, ClipboardPaste, FilePenLine, MessageCircleQuestion, X } from 'lucide-solid';
import { EntryContext, createEntryActions } from '../actions';
import { NodeView } from '../NodeView';
import type { UiNode } from '../types';
import type { UiRequest } from '../types';

const str = (v: unknown): string | undefined => (typeof v === 'string' ? v : undefined);

/** ── multiSelect ─────────────────────────────────────────────────────── */

type MsOption = { label?: string; value?: string; description?: string; selected?: boolean };

export function MultiSelectBody(props: { dialog: UiRequest; onAnswer: (value: unknown) => void }) {
  const options = (): MsOption[] => (Array.isArray(props.dialog.options) ? (props.dialog.options as MsOption[]) : []);
  const valueOf = (o: MsOption): string => o.value ?? o.label ?? '';
  const [picked, setPicked] = createSignal<Set<string>>(new Set(options().filter((o) => o.selected).map(valueOf).filter(Boolean)));
  const min = () => (typeof props.dialog.min === 'number' ? props.dialog.min : 0);
  const max = () => (typeof props.dialog.max === 'number' ? props.dialog.max : Number.POSITIVE_INFINITY);
  const toggle = (value: string) => {
    setPicked((cur) => {
      const next = new Set(cur);
      if (next.has(value)) next.delete(value);
      else if (next.size < max()) next.add(value);
      return next;
    });
  };
  const canSend = () => picked().size >= min();
  return (
    <div class="dlg" data-testid="dlg-multiselect">
      <div class="dlg-head">
        <MessageCircleQuestion size={16} class="dlg-head-icon" />
        <span class="dlg-eyebrow">Question</span>
        <span class="flex-1" />
        <button type="button" class="dlg-x" aria-label="Cancel" onClick={() => props.onAnswer(null)}>
          <X size={18} />
        </button>
      </div>
      <p class="dlg-question">{str(props.dialog.title) ?? 'Pick any'}</p>
      <div class="dlg-options">
        <Index each={options()}>
          {(o) => {
            const on = () => picked().has(valueOf(o()));
            return (
              <button type="button" class="dlg-opt" aria-pressed={on()} onClick={() => toggle(valueOf(o()))}>
                <span class="dlg-check" data-on={on() ? '' : undefined}>
                  <Show when={on()}>
                    <Check size={12} strokeWidth={3} />
                  </Show>
                </span>
                <span class="min-w-0 flex-1">
                  <span class="dlg-opt-label">{o().label ?? o().value}</span>
                  <Show when={o().description}>
                    <span class="dlg-opt-desc">{o().description}</span>
                  </Show>
                </span>
              </button>
            );
          }}
        </Index>
      </div>
      <div class="dlg-foot">
        <Show when={min() > 0 || max() !== Number.POSITIVE_INFINITY}>
          <span class="dlg-hint">
            {picked().size} picked{min() > 0 ? ` · at least ${min()}` : ''}{max() !== Number.POSITIVE_INFINITY ? ` · up to ${max()}` : ''}
          </span>
        </Show>
        <span class="flex-1" />
        <button type="button" class="dlg-btn primary" disabled={!canSend()} onClick={() => props.onAnswer([...picked()])}>
          Send
        </button>
      </div>
    </div>
  );
}

/** ── custom (spec node) ─────────────────────────────────────────────── */

/**
 * The `custom` dialog body: `spec` renders through `NodeView`; a submitting
 * action (`Button` fires with `collect`) resolves the dialog — the first one
 * wins (§4.6). The entry context carries the dialog id so keyed
 * `Input`/`Toggle`/`Select` values collect into `values`.
 */
export function CustomBody(props: { dialog: UiRequest; onAnswer: (value: unknown) => void; hostId?: string; sessionId?: string }) {
  const spec = createMemo(() => props.dialog.spec as UiNode | undefined);
  let resolved = false;
  const actions = createEntryActions(`dialog:${props.dialog.id}`, { scope: 'session', hostId: props.hostId ?? '', sessionId: props.sessionId }, (payload) => {
    if (resolved) return;
    resolved = true;
    // §4.6: `value` / `values` are optional — a plain button answers `{actionId}`.
    const value = payload.value ?? undefined;
    props.onAnswer({ actionId: payload.actionId, ...(value !== undefined ? { value } : {}), ...(payload.values && Object.keys(payload.values).length ? { values: payload.values } : {}) });
  });
  return (
    <div class="dlg dlg-custom" data-testid="dlg-custom">
      <div class="dlg-head">
        <MessageCircleQuestion size={16} class="dlg-head-icon" />
        <span class="dlg-eyebrow">{str(props.dialog.title) ?? 'Panel'}</span>
        <span class="flex-1" />
        <button type="button" class="dlg-x" aria-label="Cancel" onClick={() => props.onAnswer(null)}>
          <X size={18} />
        </button>
      </div>
      <Show when={spec()}>
        <div class="dlg-custom-body">
          <EntryContext.Provider value={actions}>
            <NodeView node={spec()!} path={[]} mode="auto" />
          </EntryContext.Provider>
        </div>
      </Show>
    </div>
  );
}

/** ── editExternally ─────────────────────────────────────────────────── */

export function EditExternallyBody(props: { dialog: UiRequest; onAnswer: (value: unknown) => void }) {
  const [text, setText] = createSignal(str(props.dialog.text) ?? '');
  const hint = () => str(props.dialog.extensionHint);
  return (
    <div class="dlg" data-testid="dlg-edit">
      <div class="dlg-head">
        <FilePenLine size={16} class="dlg-head-icon" />
        <span class="dlg-eyebrow">Edit{hint() ? ` · ${hint()}` : ''}</span>
        <span class="flex-1" />
        <button type="button" class="dlg-x" aria-label="Cancel" onClick={() => props.onAnswer(null)}>
          <X size={18} />
        </button>
      </div>
      <textarea
        class="dlg-text dlg-edit-area"
        rows={8}
        spellcheck={false}
        aria-label="Text to edit"
        value={text()}
        onInput={(e) => setText(e.currentTarget.value)}
      />
      <div class="dlg-foot">
        <span class="flex-1" />
        <button type="button" class="dlg-btn primary" onClick={() => props.onAnswer(text())}>
          Save
        </button>
      </div>
    </div>
  );
}

/** ── clipboardReadImage ─────────────────────────────────────────────── */

type ClipboardItemLike = { types: string[]; getType(type: string): Promise<Blob | null> };

function readClipboardImage(items: ClipboardItemLike[]): Promise<{ data: string; mimeType: string } | null> {
  const item = items.find((i) => i.types.some((t) => t.startsWith('image/')));
  if (!item) return Promise.resolve(null);
  const type = item.types.find((t) => t.startsWith('image/'))!;
  return item
    .getType(type)
    .then(
      (blob) =>
        new Promise<string | null>((resolve) => {
          if (!blob) {
            resolve(null);
            return;
          }
          const reader = new FileReader();
          reader.onload = () => resolve(typeof reader.result === 'string' ? (reader.result.split(',')[1] ?? null) : null);
          reader.onerror = () => resolve(null);
          reader.readAsDataURL(blob);
        }),
    )
    .then((data) => (data ? { data, mimeType: type } : null));
}

export function ClipboardImageBody(props: { dialog: UiRequest; onAnswer: (value: unknown) => void }) {
  const [error, setError] = createSignal<string | undefined>();
  const read = () => {
    const nav = navigator as Navigator & { clipboard?: { read?: () => Promise<ClipboardItemLike[]> } };
    if (!nav.clipboard?.read) {
      setError('This build cannot read the clipboard — answer in the terminal.');
      return;
    }
    nav.clipboard
      .read()
      .then((items) => readClipboardImage(items as unknown as ClipboardItemLike[]))
      .then((image) => {
        if (image) props.onAnswer(image);
        else setError('No image on the clipboard.');
      })
      .catch(() => setError('This build cannot read the clipboard — answer in the terminal.'));
  };
  return (
    <div class="dlg" data-testid="dlg-clipboard">
      <div class="dlg-head">
        <ClipboardPaste size={16} class="dlg-head-icon" />
        <span class="dlg-eyebrow">Clipboard</span>
        <span class="flex-1" />
        <button type="button" class="dlg-x" aria-label="Cancel" onClick={() => props.onAnswer(null)}>
          <X size={18} />
        </button>
      </div>
      <p class="dlg-question">Paste the image from your clipboard</p>
      <div class="dlg-foot">
        <Show when={error()}>
          <span class="dlg-hint" role="alert">
            {error()}
          </span>
        </Show>
        <span class="flex-1" />
        <button type="button" class="dlg-btn primary" onClick={read}>
          Read clipboard
        </button>
      </div>
    </div>
  );
}

/** DialogPanel's body router: the widened kinds, or `undefined` when the pi card handles it. */
export function oinoDialogBody(props: { dialog: UiRequest; onAnswer: (value: unknown) => void; hostId?: string; sessionId?: string }) {
  const d = () => props.dialog;
  return (
    <Switch>
      <Match when={d().method === 'multiSelect' && !Array.isArray(d().questions)}>
        <MultiSelectBody dialog={d()} onAnswer={props.onAnswer} />
      </Match>
      <Match when={d().method === 'custom' && !!d().spec}>
        <CustomBody dialog={d()} onAnswer={props.onAnswer} hostId={props.hostId} sessionId={props.sessionId} />
      </Match>
      <Match when={d().method === 'editExternally'}>
        <EditExternallyBody dialog={d()} onAnswer={props.onAnswer} />
      </Match>
      <Match when={d().method === 'clipboardReadImage'}>
        <ClipboardImageBody dialog={d()} onAnswer={props.onAnswer} />
      </Match>
    </Switch>
  );
}
