/**
 * The ONE model picker (owner: "two different model pickers is weird"): the
 * New-chat draft chip, the in-chat model chip (and Ctrl/Cmd+M, `/model`,
 * `/scoped-models`) and the settings model fields all open this popover —
 * the same look, the same list (`ModelList` over `modelView`: readiness,
 * Suggested / curated ranking, display names, the collapsed Unavailable
 * group with its reasons), the same search and keyboard (search field,
 * ↓ into the rows, ↑ back, Enter picks the first hit, Esc / Tab close).
 *
 * Anchoring: a popover under / above the chip or field on the desktop, a
 * bottom sheet on the phone shell. What differs per context is only the
 * header line and the compact footer rows (`PickerFootRow`): in a chat
 * "Use at startup", "Fusion…" and "Cycle set…"; in the draft "Use at
 * startup"; in settings "Use default". A footer row may swap the panel's
 * body for a sub-view (Fusion columns, the cycle set) inside the same
 * popover — never a second dialog.
 */
import { onCleanup, Show, type JSX } from 'solid-js';
import { AnchoredMenu } from '../board/AnchoredMenu';
import { BoardSheet } from '../board/BoardSheet';
import { menuKeys } from '../composer/menuKeys';
import { layout } from '../../lib/layout';
import type { SessionHandle } from '../../lib/session/store';
import { ChevronLeft } from 'lucide-solid';
import { modelKey, type ModelInfo } from '../../lib/model/picker';
import { ModelList, type ModelView } from './ModelList';
import './model.css';

/** Popover width on the desktop (the list's name + badges fit; Fusion's columns stack). */
export const PICKER_WIDTH = 380;

/** Where a picker opens when nothing on screen is its chip (a shortcut with no composer shown). */
export function centredAnchor(): DOMRect {
  const x = window.innerWidth / 2 + PICKER_WIDTH / 2;
  const y = Math.max(8, window.innerHeight / 2 - 240);
  return { left: x, right: x, top: y, bottom: y, width: 0, height: 0, x, y, toJSON: () => ({}) } as DOMRect;
}

/**
 * The shell: an `AnchoredMenu` anchored to `anchor` (desktop) or a bottom
 * sheet (phone). `children` is the panel (`ModelPickerPanel` or a sub-view).
 */
export function ModelPopover(props: {
  anchor: DOMRect;
  label: string;
  testId: string;
  onClose: () => void;
  /** The chip / field that opened it: focus returns there. */
  trigger?: () => HTMLElement | undefined;
  children: JSX.Element;
}) {
  const desktop = () => layout().desktop();
  const keys = (el: HTMLElement) => onCleanup(menuKeys(el, { close: props.onClose, trigger: props.trigger, autoFocus: false }));
  return (
    <Show
      when={desktop()}
      fallback={
        <BoardSheet open onClose={props.onClose} label={props.label} testId={`${props.testId}-sheet`} class="model-picker-sheet">
          <div class="mpop mpop-sheet" data-testid={props.testId} ref={keys}>
            {props.children}
          </div>
        </BoardSheet>
      }
    >
      <AnchoredMenu anchor={props.anchor} alignEnd width={PICKER_WIDTH} label={props.label} class="mpop-menu" onDismiss={props.onClose}>
        <div class="mpop" data-testid={props.testId} ref={keys}>
          {props.children}
        </div>
      </AnchoredMenu>
    </Show>
  );
}

/** The search field every picker view starts with (desktop: focused at once). */
export function PickerSearch(props: { value: string; placeholder?: string; onInput: (q: string) => void; onEnter?: () => void }) {
  return (
    <input
      class="mpop-search"
      type="search"
      placeholder={props.placeholder ?? 'Search models'}
      aria-label="Search models"
      value={props.value}
      onInput={(e) => props.onInput(e.currentTarget.value)}
      onKeyDown={(e) => {
        if (e.key === 'Enter' && props.onEnter) {
          e.preventDefault();
          props.onEnter();
        }
      }}
      ref={(el) => {
        // The phone keeps the keyboard down until the user taps the field.
        if (layout().desktop()) queueMicrotask(() => el.isConnected && el.focus({ preventScroll: true }));
      }}
      data-testid="model-search"
    />
  );
}

/** One compact footer row (icon · label · detail), keyboard-reachable like the list rows. */
export function PickerFootRow(props: {
  icon: JSX.Element;
  label: string;
  detail?: string;
  checked?: boolean;
  expanded?: boolean;
  disabled?: boolean;
  title?: string;
  testId: string;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      role={props.checked === undefined ? 'menuitem' : 'menuitemradio'}
      aria-checked={props.checked === undefined ? undefined : props.checked}
      aria-expanded={props.expanded}
      class="mpop-foot-row"
      disabled={props.disabled}
      title={props.title}
      onClick={() => props.onSelect()}
      data-testid={props.testId}
    >
      <span class="mpop-foot-ic" aria-hidden="true">
        {props.icon}
      </span>
      <span class="mpop-foot-label">{props.label}</span>
      <Show when={props.detail}>
        <span class="mpop-foot-detail">{props.detail}</span>
      </Show>
    </button>
  );
}

/**
 * The list view every context shares: search, a header line, the list (or a
 * loading / empty state), then the context's footer rows.
 */
export function ModelPickerPanel(props: {
  view: ModelView;
  query: string;
  onQuery: (q: string) => void;
  /** The header line under the search ("Now …", "Model for this chat", a field's label). */
  header: JSX.Element;
  loading: boolean;
  /** Shown instead of the list when `showEmpty` (the context has nothing to list: no provider…). */
  empty?: JSX.Element;
  showEmpty?: boolean;
  currentKey?: string;
  pickedKey?: string;
  startupKey?: string;
  onSelect: (m: ModelInfo, isCurrent: boolean) => void;
  /** Between the list and the footer (an error, a confirm step). */
  notice?: JSX.Element;
  footer?: JSX.Element;
}) {
  const first = () => props.view.suggested[0]?.model ?? props.view.rest[0];
  return (
    <>
      <PickerSearch
        value={props.query}
        onInput={props.onQuery}
        placeholder={props.view.usable.length > 6 ? `Search all ${props.view.usable.length} models` : 'Search models'}
        onEnter={() => {
          const m = first();
          if (m) props.onSelect(m, props.currentKey === modelKey(m));
        }}
      />
      <div class="mpop-head">{props.header}</div>
      <div class="mpop-scroll">
        <Show
          when={!props.loading}
          fallback={
            <div class="mp-skeleton" data-testid="model-picker-loading">
              <span />
              <span />
              <span />
            </div>
          }
        >
          <Show when={!props.showEmpty} fallback={props.empty}>
            <ModelList
              view={props.view}
              query={props.query}
              currentKey={props.currentKey}
              pickedKey={props.pickedKey}
              startupKey={props.startupKey}
              onSelect={props.onSelect}
            />
          </Show>
        </Show>
      </div>
      {props.notice}
      <Show when={props.footer}>
        <div class="mpop-foot" data-testid="model-picker-foot">
          {props.footer}
        </div>
      </Show>
    </>
  );
}

/** A sub-view's title row with its way back to the list. */
export function PickerSubHead(props: { title: string; onBack: () => void; testId: string }) {
  return (
    <div class="mpop-subhead">
      <button type="button" role="menuitem" class="mpop-back" onClick={() => props.onBack()} data-testid={props.testId} aria-label={`Back to the model list (${props.title})`}>
        <ChevronLeft size={15} aria-hidden="true" />
        <span>{props.title}</span>
      </button>
    </div>
  );
}

/**
 * "Use at startup": the model a fresh session opens with — pi's
 * `defaultProvider` / `defaultModel` (+ the curated thinking level) and, with
 * Fusion on, its `startup.model` (it wins on a fresh session). `target` is a
 * live chat or the draft (whose `_oino/set_setting` goes to the host).
 */
export async function writeStartupModel(
  target: Pick<SessionHandle, 'cmd'>,
  key: string,
  opts: { effort?: string; fusionOn?: boolean } = {},
): Promise<void> {
  const slash = key.indexOf('/');
  await target.cmd({ type: '_oino/set_setting', key: 'defaultProvider', value: key.slice(0, slash), scope: 'global' });
  await target.cmd({ type: '_oino/set_setting', key: 'defaultModel', value: key.slice(slash + 1), scope: 'global' });
  if (opts.effort) await target.cmd({ type: '_oino/set_setting', key: 'defaultThinkingLevel', value: opts.effort, scope: 'global' });
  if (opts.fusionOn) await target.cmd({ type: '_oino/set_extension_setting', namespace: 'fusion', key: 'startup.model', value: key, scope: 'global' });
}

/** Fusion's `startup.model` from `_oino/get_extension_settings` (nested or dotted values). */
export function fusionStartup(reply: unknown): string | undefined {
  const values = (reply as { values?: Record<string, unknown> } | undefined)?.values;
  const nested = (values?.startup as { model?: unknown } | undefined)?.model;
  const v = nested ?? values?.['startup.model'];
  return typeof v === 'string' && v ? v : undefined;
}
