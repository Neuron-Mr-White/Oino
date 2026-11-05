/**
 * The Fusion tab's lead / sidekick columns and effort pills (Z-76), shared by
 * the chat's model dialog (`ChatModelPicker`, applies with `fusion.set-pair`)
 * and the new-chat draft's (`DraftModelChip`, keeps the pair as a draft
 * choice applied right after the chat starts). One search filters both
 * columns; a column's curated entries come first.
 *
 * `createFusionPair` holds only the pending picks; what a seat shows falls
 * back to the running / configured pair (`base`), then Fusion's curated
 * defaults, then the catalogue's first rows.
 */
import { createEffect, createMemo, createSignal, For, on, Show, type JSX } from 'solid-js';
import { Check } from 'lucide-solid';
import type { FusionCurated } from '../../lib/model/fusion';
import { columnModels, FUSION_EFFORTS, modelKey, modelName, shortModelName, type ModelInfo } from '../../lib/model/picker';
import { curatedFirst, modelPathLabel, rowTitle } from './suggested';
import { LIST_CAP } from './ModelList';
import { PickerSkeleton } from './ModelPopover';

export type FusionSide = 'lead' | 'sidekick';

/** Scrolls the chosen row of each Fusion column into view (often below its fold). */
export function revealPicked(root: HTMLElement): void {
  for (const list of root.querySelectorAll<HTMLElement>('.mp-col-list')) {
    const row = list.querySelector<HTMLElement>('[aria-current="true"]');
    if (!row) continue;
    const lr = list.getBoundingClientRect();
    const rr = row.getBoundingClientRect();
    if (rr.top < lr.top || rr.bottom > lr.bottom) list.scrollTop += rr.top - lr.top - (lr.height - rr.height) / 2;
  }
}

export type FusionPairOptions = {
  /** The usable catalogue rows (the Single list's `usable`). */
  models: () => readonly ModelInfo[];
  /** The whole catalogue a curated default is checked against (default: `models`). */
  catalogue?: () => readonly ModelInfo[];
  /** The one search shared with the Single list. */
  query: () => string;
  curated: () => FusionCurated | undefined;
  /** The pair a seat falls back to: the running pair, else the extension's configured one. */
  base: () => { lead?: string; sidekick?: string };
  /** Called on every column / pill pick (e.g. to clear an error). */
  onPick?: () => void;
};

export function createFusionPair(opts: FusionPairOptions) {
  const byKey = createMemo(() => new Set((opts.catalogue ?? opts.models)().map((m) => modelKey(m))));
  const [pendingLead, setPendingLead] = createSignal<string | undefined>();
  const [pendingSidekick, setPendingSidekick] = createSignal<string | undefined>();
  const [leadEffort, setLeadEffort] = createSignal('medium');
  const [sidekickEffort, setSidekickEffort] = createSignal('medium');
  /** A key the catalogue knows (a curated default can name a provider this PC lacks). */
  const known = (key: string | undefined): string | undefined => (key && byKey().has(key) ? key : undefined);
  const firstKnown = (l: ReadonlyArray<{ key: string }> | undefined) => l?.find((e) => byKey().has(e.key))?.key;
  const nth = (i: number) => {
    const m = opts.models()[i];
    return m ? modelKey(m) : undefined;
  };
  const leadKey = () =>
    pendingLead() ?? opts.base().lead ?? known(opts.curated()?.default.lead) ?? firstKnown(opts.curated()?.lead) ?? nth(0);
  const sidekickKey = () =>
    pendingSidekick() ??
    opts.base().sidekick ??
    known(opts.curated()?.default.sidekick) ??
    firstKnown(opts.curated()?.sidekick) ??
    nth(1) ??
    nth(0);
  /** A curated effort the pills can show (they offer low / medium / high). */
  const pillEffort = (key: string): string | undefined => {
    const e = opts.curated()?.effort[key];
    return e && (FUSION_EFFORTS as readonly string[]).includes(e) ? e : undefined;
  };
  const byKeyMap = createMemo(() => new Map((opts.catalogue ?? opts.models)().map((m) => [modelKey(m), m])));
  const colRows = createMemo(() => columnModels(opts.models(), opts.query()));
  const splitColumn = (curatedKeys: readonly string[], selectedKey: string | undefined) => {
    const base = curatedFirst(colRows(), curatedKeys);
    const selected = selectedKey ? colRows().find((m) => modelKey(m) === selectedKey) : undefined;
    if (!selected) return { selected: undefined as ModelInfo | undefined, curated: base.curated, rest: base.rest };
    return {
      selected,
      curated: base.curated.filter((m) => modelKey(m) !== selectedKey),
      rest: base.rest.filter((m) => modelKey(m) !== selectedKey),
    };
  };
  const leadRows = createMemo(() => splitColumn((opts.curated()?.lead ?? []).map((e) => e.key), leadKey()));
  const sidekickRows = createMemo(() => splitColumn((opts.curated()?.sidekick ?? []).map((e) => e.key), sidekickKey()));

  let leadListEl: HTMLDivElement | undefined;
  let sidekickListEl: HTMLDivElement | undefined;

  /** A seat pick: the key, plus its curated effort when it has one. */
  const choose = (side: FusionSide, key: string) => {
    opts.onPick?.();
    const effort = pillEffort(key);
    if (side === 'lead') {
      setPendingLead(key);
      if (effort) setLeadEffort(effort);
      queueMicrotask(() => { if (leadListEl) leadListEl.scrollTop = 0; });
    } else {
      setPendingSidekick(key);
      if (effort) setSidekickEffort(effort);
      queueMicrotask(() => { if (sidekickListEl) sidekickListEl.scrollTop = 0; });
    }
  };

  // When search clears, scroll columns back to top where Selected sits.
  createEffect(
    on(
      () => opts.query().trim(),
      (q, prev) => {
        if (!q && prev) {
          if (leadListEl) leadListEl.scrollTop = 0;
          if (sidekickListEl) sidekickListEl.scrollTop = 0;
        }
      },
    ),
  );

  /** One column row. */
  const ColRow = (p: { m: ModelInfo; side: FusionSide }) => {
    const key = modelKey(p.m);
    const chosen = () => (p.side === 'lead' ? leadKey() : sidekickKey()) === key;
    return (
      <button
        type="button"
        role="menuitemradio"
        aria-checked={chosen()}
        class="mp-row"
        title={rowTitle(modelName(p.m), p.m)}
        aria-current={chosen() ? 'true' : undefined}
        data-key={key}
        onClick={() => choose(p.side, key)}
      >
        <span class="mp-row-main">
          <span class="mp-row-name">{shortModelName(modelName(p.m))}</span>
          <span class="mp-row-sub">{modelPathLabel(p.m)}</span>
        </span>
        <Show when={chosen()}>
          <Check size={16} class="mp-check" />
        </Show>
      </button>
    );
  };
  const Column = (p: { side: FusionSide }) => {
    const rows = () => (p.side === 'lead' ? leadRows() : sidekickRows());
    const chosenKey = () => (p.side === 'lead' ? leadKey() : sidekickKey());
    const chosenLabel = () => {
      const k = chosenKey();
      if (!k) return undefined;
      const m = byKeyMap().get(k);
      return m ? shortModelName(modelName(m)) : k.slice(k.indexOf('/') + 1);
    };
    return (
      <div class="mp-col" data-testid={`fusion-col-${p.side}`}>
        <p class="mp-label">
          <span>{p.side === 'lead' ? 'Lead' : 'Sidekick'}</span>
          <Show when={chosenLabel()}>
            <span class="mp-col-picked" title={chosenKey()}> · {chosenLabel()}</span>
          </Show>
        </p>
        <div
          class="mp-list mp-col-list"
          ref={(el) => {
            if (p.side === 'lead') leadListEl = el;
            else sidekickListEl = el;
          }}
        >
          <Show when={rows().selected}>
            {(sel) => (
              <>
                <p class="mp-group">Selected</p>
                <ColRow m={sel()} side={p.side} />
              </>
            )}
          </Show>
          <Show when={rows().curated.length > 0}>
            <p class="mp-group">Curated</p>
            <For each={rows().curated}>{(m) => <ColRow m={m} side={p.side} />}</For>
            <Show when={rows().rest.length > 0}>
              <p class="mp-group">All models</p>
            </Show>
          </Show>
          <Show when={rows().curated.length === 0 && rows().selected && rows().rest.length > 0}>
            <p class="mp-group">All models</p>
          </Show>
          <For
            each={rows().rest.slice(0, LIST_CAP)}
            fallback={
              <Show when={!rows().selected && rows().curated.length === 0}>
                <p class="mp-empty">No model matches.</p>
              </Show>
            }
          >
            {(m) => <ColRow m={m} side={p.side} />}
          </For>
        </div>
      </div>
    );
  };
  const Effort = (p: { side: FusionSide }) => {
    const value = () => (p.side === 'lead' ? leadEffort() : sidekickEffort());
    const set = (level: string) => {
      opts.onPick?.();
      if (p.side === 'lead') setLeadEffort(level);
      else setSidekickEffort(level);
    };
    return (
      <div class="mp-effort" role="group" aria-label={p.side === 'lead' ? 'Lead effort' : 'Sidekick effort'}>
        <span class="mp-label">{p.side === 'lead' ? 'Lead effort' : 'Sidekick effort'}</span>
        <For each={FUSION_EFFORTS}>
          {(level) => (
            <button type="button" role="menuitemradio" aria-checked={value() === level} class="mp-pill" onClick={() => set(level)}>
              {level}
            </button>
          )}
        </For>
      </div>
    );
  };
  /** The tab's content: both columns and both effort rows (a skeleton while loading). */
  const content = (loading: boolean): JSX.Element => (
    <Show when={!loading} fallback={<PickerSkeleton />}>
      <div class="mp-fusion" ref={(el) => queueMicrotask(() => revealPicked(el))} data-testid="model-picker-fusion">
        <Column side="lead" />
        <Column side="sidekick" />
      </div>
      <Effort side="lead" />
      <Effort side="sidekick" />
    </Show>
  );

  return {
    leadKey,
    sidekickKey,
    leadEffort,
    sidekickEffort,
    setLeadEffort,
    setSidekickEffort,
    setPendingLead,
    setPendingSidekick,
    pillEffort,
    content,
  };
}
