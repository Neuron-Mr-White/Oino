/**
 * The ONE model picker (owner: "two different model pickers is weird" — and
 * "it must be the full one"): the New-chat draft chip, the in-chat model chip
 * (and Ctrl/Cmd+M, `/model`, `/scoped-models`), the settings model fields and
 * the Cron job model field all open this dialog — the old in-chat "Model"
 * dialog layout everywhere:
 *
 *   title (+ ✕) · "Now …" · Single / Fusion tabs (when the context has
 *   Fusion) · full-width search "Search all N models" · the list (Suggested,
 *   provider rows, the collapsed Unavailable group with its reasons, the
 *   Startup badge, modality icons, context chip, a ⋯ menu per row in chat /
 *   draft: Fusion roles, cycle set, startup) · the "Cycle set" filter chip
 *   (chat / draft; on at open for `/scoped-models`) · the footer: the
 *   explanatory line and the context's action ("Use at startup", or a
 *   field's "Use default").
 *
 * Shell (`ModelDialog`): a centred modal on the desktop (`model-picker-overlay`,
 * portalled so it sits above sheets and settings), a full-screen sheet with
 * the same content on the phone. Keyboard: the search has focus, ↓ walks the
 * rows, ↑ goes back, Enter in the search picks the first hit, Esc closes,
 * Tab wraps inside; focus returns to the opener on close.
 */
import { createEffect, createSignal, onCleanup, Show, type JSX } from 'solid-js';
import { Portal } from 'solid-js/web';
import { Repeat, X } from 'lucide-solid';
import { BoardSheet } from '../board/BoardSheet';
import { menuKeys } from '../composer/menuKeys';
import { layout } from '../../lib/layout';
import { pushBack } from '../../lib/back';
import { wrapTab } from '../../lib/focusTrap';
import type { SessionHandle } from '../../lib/session/store';
import { modelKey, type ModelInfo } from '../../lib/model/picker';
import { cycleKeysOf, toggleCycleKey } from '../../lib/model/cycle';
import { ModelList, type ModelView, type RowActions } from './ModelList';
import '../session/session-modal.css';
import './model.css';

/**
 * The dialog shell: centred modal (desktop) or full-screen sheet (phone).
 * `testId` names the dialog (desktop) / the sheet's content (phone, the sheet
 * itself is `${testId}-sheet`).
 */
export function ModelDialog(props: {
  title: string;
  testId: string;
  onClose: () => void;
  /** The chip / field that opened it: focus returns there (default: whatever had focus at open). */
  trigger?: () => HTMLElement | undefined;
  children: JSX.Element;
}) {
  const desktop = () => layout().desktop();
  const openedFrom = typeof document !== 'undefined' ? (document.activeElement as HTMLElement | null) : null;
  const trigger = () => props.trigger?.() ?? (openedFrom && openedFrom !== document.body ? openedFrom : undefined);
  const keys = (el: HTMLElement) => onCleanup(menuKeys(el, { close: props.onClose, trigger, autoFocus: false, tabCloses: false }));
  // Back / Esc closes the top overlay — this dialog, not the sheet or page under it.
  createEffect(() => {
    if (desktop()) onCleanup(pushBack(() => props.onClose()));
  });
  return (
    <Show
      when={desktop()}
      fallback={
        <BoardSheet open onClose={props.onClose} label={props.title} title={props.title} testId={`${props.testId}-sheet`} class="model-picker-sheet">
          <div class="mp-sheet-body" data-testid={props.testId} ref={keys}>
            {props.children}
          </div>
        </BoardSheet>
      }
    >
      <Portal>
        <div class="sm-overlay model-picker-overlay" onClick={(e) => e.target === e.currentTarget && props.onClose()} data-testid="model-picker-overlay">
          <div
            role="dialog"
            aria-modal="true"
            aria-label={props.title}
            class="sm-dialog model-picker-dialog"
            tabindex="-1"
            data-testid={props.testId}
            ref={(el) => {
              keys(el);
              queueMicrotask(() => el.isConnected && !el.contains(document.activeElement) && el.focus({ preventScroll: true }));
            }}
            onKeyDown={(e: KeyboardEvent) => {
              if (e.key === 'Escape' && !e.defaultPrevented) {
                e.preventDefault();
                e.stopPropagation();
                props.onClose();
                return;
              }
              if (wrapTab(e, e.currentTarget as HTMLElement)) e.stopPropagation();
            }}
          >
            <div class="sm-head">
              <span class="sm-title">
                <h2>{props.title}</h2>
              </span>
              <button type="button" class="ghost-icon-button" aria-label="Close" onClick={() => props.onClose()} data-testid="model-picker-close">
                <X size={20} />
              </button>
            </div>
            <div class="sm-body mp-body">{props.children}</div>
          </div>
        </div>
      </Portal>
    </Show>
  );
}

/** The "Now …" line under the title. */
export function NowLine(props: { value: string }) {
  return (
    <p class="mp-now" data-testid="model-picker-now">
      <span class="mp-now-word">Now</span>
      <span class="mp-now-value">{props.value}</span>
    </p>
  );
}

/** Single / Fusion tabs (shown only where Fusion is on for the context). */
export function ModeTabs(props: { mode: 'single' | 'fusion'; onMode: (m: 'single' | 'fusion') => void }) {
  return (
    <div class="sm-tabs mp-tabs" role="tablist" aria-label="Model mode">
      <button type="button" role="tab" aria-selected={props.mode === 'single'} class="sm-tab" onClick={() => props.onMode('single')} data-testid="model-tab-single">
        Single
      </button>
      <button type="button" role="tab" aria-selected={props.mode === 'fusion'} class="sm-tab" onClick={() => props.onMode('fusion')} data-testid="model-tab-fusion">
        Fusion
      </button>
    </div>
  );
}

/** The full-width search (desktop: focused at once; the phone keeps the keyboard down). */
export function PickerSearch(props: { value: string; placeholder?: string; onInput: (q: string) => void; onEnter?: () => void }) {
  return (
    <input
      class="mp-search"
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
        if (layout().desktop()) queueMicrotask(() => el.isConnected && el.focus({ preventScroll: true }));
      }}
      data-testid="model-search"
    />
  );
}

/** The loading rows. */
export const PickerSkeleton = () => (
  <div class="mp-skeleton" data-testid="model-picker-loading">
    <span />
    <span />
    <span />
  </div>
);

/**
 * The dialog's content every context shares, in the old dialog's order:
 * Now · tabs · search · filter chip · list (or `content`, e.g. Fusion's
 * columns) · section · notice (an error) · footer (scope line + actions).
 */
export function ModelPickerPanel(props: {
  view: ModelView;
  query: string;
  onQuery: (q: string) => void;
  /** "Now …" — the model this context runs / will start with / is set to. */
  now: string;
  tabs?: JSX.Element;
  searchPlaceholder?: string;
  loading: boolean;
  /** Shown instead of the list when `showEmpty` (no provider, no list yet…). */
  empty?: JSX.Element;
  showEmpty?: boolean;
  /** Replaces the list (Fusion's columns). */
  content?: JSX.Element;
  currentKey?: string;
  pickedKey?: string;
  startupKey?: string;
  /** The rows' ⋯ menu (Fusion roles, cycle set, startup); none → no ⋯. */
  rowActions?: RowActions;
  /**
   * The "Cycle set" filter chip (chat / draft): narrows the list to the cycle
   * set's members; `initial` = on at open (`/scoped-models`).
   */
  cycleFilter?: { set: CycleSet; initial?: boolean };
  onSelect: (m: ModelInfo, isCurrent: boolean) => void;
  /** Under the list. */
  section?: JSX.Element;
  /** An error / a confirm step. */
  notice?: JSX.Element;
  /** The footer's explanatory line. */
  scope?: JSX.Element;
  /** The footer's buttons, right-aligned. */
  actions?: JSX.Element;
  testId?: string;
}) {
  const [onlyCycle, setOnlyCycle] = createSignal(!!props.cycleFilter?.initial);
  const filtering = () => !!props.cycleFilter && onlyCycle();
  /** The list as shown: the whole view, or only the cycle set's members (no Unavailable group). */
  const view = (): ModelView => {
    const set = props.cycleFilter?.set;
    if (!set || !onlyCycle()) return props.view;
    const on = (m: ModelInfo) => set.has(modelKey(m));
    return { ...props.view, suggested: props.view.suggested.filter((r) => on(r.model)), rest: props.view.rest.filter(on), unavailable: [] };
  };
  const filteredEmpty = () => filtering() && view().suggested.length === 0 && view().rest.length === 0;
  const first = () => view().suggested[0]?.model ?? view().rest[0];
  return (
    <div class="mp" data-testid={props.testId ?? 'model-picker'}>
      <NowLine value={props.now} />
      {props.tabs}
      <PickerSearch
        value={props.query}
        onInput={props.onQuery}
        placeholder={props.searchPlaceholder ?? (props.view.usable.length > 0 ? `Search all ${props.view.usable.length} models` : 'Search models')}
        onEnter={
          props.content
            ? undefined
            : () => {
                const m = first();
                if (m) props.onSelect(m, props.currentKey === modelKey(m));
              }
        }
      />
      <Show when={props.cycleFilter && !props.content && !props.showEmpty}>
        <div class="mp-filters" role="group" aria-label="Filter models">
          <button
            type="button"
            class="mp-filter-chip"
            aria-pressed={onlyCycle() ? 'true' : 'false'}
            title="Show only the models Ctrl+Alt+P steps through"
            onClick={() => setOnlyCycle((v) => !v)}
            data-testid="model-filter-cycle"
          >
            <Repeat size={13} aria-hidden="true" />
            <span>Cycle set</span>
            <span class="mp-filter-count">{props.cycleFilter!.set.exact().length}</span>
          </button>
          <Show when={onlyCycle()}>
            <span class="mp-hint mp-filter-hint" data-testid="model-filter-cycle-hint">
              Use ⋯ to add or remove
            </span>
          </Show>
        </div>
      </Show>
      <Show
        when={!props.content}
        fallback={props.content}
      >
        <Show when={!props.loading} fallback={<PickerSkeleton />}>
          <Show when={!props.showEmpty} fallback={props.empty}>
            <Show when={filtering() && props.cycleFilter!.set.custom().length > 0}>
              <p class="mp-hint" data-testid="model-filter-cycle-custom">
                Also cycles (patterns kept as written): {props.cycleFilter!.set.custom().join(', ')}
              </p>
            </Show>
            <Show
              when={!filteredEmpty()}
              fallback={
                <div class="mp-list" data-testid="model-picker-list">
                  <p class="mp-empty" data-testid="model-filter-cycle-empty">
                    {props.query.trim()
                      ? `No model in the cycle set matches “${props.query.trim()}”.`
                      : 'No models in the cycle set yet. Use ⋯ on a model to add it.'}
                  </p>
                </div>
              }
            >
              <ModelList
                view={view()}
                query={props.query}
                currentKey={props.currentKey}
                pickedKey={props.pickedKey}
                startupKey={props.startupKey}
                actions={props.rowActions}
                onSelect={props.onSelect}
              />
            </Show>
          </Show>
        </Show>
      </Show>
      {props.section}
      {props.notice}
      <Show when={props.scope || props.actions}>
        <div class="mp-foot" data-testid="model-picker-foot">
          <Show when={props.scope}>
            <p class="mp-hint mp-scope-note" data-testid="model-picker-scope">
              {props.scope}
            </p>
          </Show>
          {props.actions}
        </div>
      </Show>
    </div>
  );
}

/** The error line between the list and the footer. */
export function PickerError(props: { error: string | null | undefined }) {
  return (
    <Show when={props.error}>
      <p class="mp-error" role="alert" data-testid="model-picker-error">
        {props.error}
      </p>
    </Show>
  );
}

/** The footer's secondary button ("Use at startup", "Use default"). */
export function PickerSecondary(props: {
  label: string;
  disabled?: boolean;
  pressed?: boolean;
  title?: string;
  testId: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      class="pair-secondary mp-apply mp-secondary-btn"
      disabled={props.disabled}
      aria-pressed={props.pressed === undefined ? undefined : props.pressed ? 'true' : 'false'}
      title={props.title}
      onClick={() => props.onClick()}
      data-testid={props.testId}
    >
      {props.label}
    </button>
  );
}

/**
 * The cycle set (WP-A25, `/scoped-models`): pi's `enabledModels` — what
 * Ctrl+Alt+P steps through. Seeded once from the loaded setting; `toggle`
 * writes it back (`_oino/set_setting`, global) through `target` (a live chat
 * or the draft, whose settings go to the host). Exact `provider/model`
 * entries are the toggles; anything else (a glob, a `:level`) is kept as
 * written.
 */
export type CycleSet = {
  exact: () => string[];
  custom: () => string[];
  has: (key: string) => boolean;
  /** Adds / removes one model; resolves false when the write failed (`onError` got the message). */
  toggle: (key: string, on: boolean) => Promise<boolean>;
};

export function createCycleSet(opts: {
  target: () => Pick<SessionHandle, 'cmd'>;
  stored: () => unknown;
  loaded: () => boolean;
  onError: (message: string | null) => void;
}): CycleSet {
  const [cycle, setCycle] = createSignal<{ exact: string[]; custom: string[] }>({ exact: [], custom: [] });
  let seeded = false;
  createEffect(() => {
    if (seeded || !opts.loaded()) return;
    seeded = true;
    setCycle(cycleKeysOf(opts.stored()));
  });
  return {
    exact: () => cycle().exact,
    custom: () => cycle().custom,
    has: (key) => cycle().exact.includes(key),
    toggle: async (key, on) => {
      const cur = cycle();
      const nextExact = on ? [...new Set([...cur.exact, key])] : cur.exact.filter((k) => k !== key);
      const value = toggleCycleKey(cur.exact, cur.custom, key, on);
      opts.onError(null);
      try {
        await opts.target().cmd({ type: '_oino/set_setting', key: 'enabledModels', value: value ?? null, scope: 'global' });
        setCycle({ exact: nextExact, custom: cur.custom });
        return true;
      } catch (e) {
        opts.onError(String((e as { message?: unknown })?.message ?? e));
        return false;
      }
    },
  };
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
