/**
 * The draft's model chip (Z-19): looks and sits exactly like the chat's
 * model chip; the chat's picker sheet is session-bound, so the draft opens a
 * compact list instead — this PC's model catalogue (borrowed from a live chat
 * on the same PC when one is open) or, before any chat ran, the default. The
 * pick is applied to the new session before its first prompt.
 */
import { createResource, createSignal, For, onCleanup, Show } from 'solid-js';
import { Check, ChevronUp, Cpu } from 'lucide-solid';
import { AnchoredMenu } from '../board/AnchoredMenu';
import { hits as modelHits, modelKey, modelName, parseModels, type ModelInfo } from '../../lib/model/picker';
import { modelChipLabel, modelChipTitle } from '../model/ModelChip';
import { RollingLabel } from '../RollingLabel';
import { menuKeys } from '../composer/menuKeys';
import type { DraftSession } from './draftSession';

export function DraftModelChip(props: { session: DraftSession; disabled?: boolean }) {
  const [anchor, setAnchor] = createSignal<DOMRect | undefined>();
  // D-01: no usable model on show → say so, never a name the new chat can't run.
  const label = () => (props.session.state.model?.id ? modelChipLabel(props.session.state.model) : 'Choose a model');
  const current = () => modelKey(props.session.state.model ?? {});
  const [models] = createResource(
    () => (anchor() ? props.session.hostId : undefined),
    // The draft answers from a live chat on this PC, else the list it last saw (P0-09).
    async (): Promise<ModelInfo[]> => parseModels(await props.session.cmd<unknown>({ type: 'get_available_models' }).catch(() => undefined)),
  );
  const [query, setQuery] = createSignal('');
  let chip: HTMLButtonElement | undefined;
  /** The current model first, then the catalogue filtered by the query (name, id, provider); ≤ 80 rows. */
  const list = (): ModelInfo[] => {
    const all = models() ?? [];
    const m = props.session.state.model;
    const rows = all.length || !m?.id ? all : [{ provider: m.provider ?? '', id: m.id, name: m.name ?? m.id } as ModelInfo];
    const q = query().trim().toLowerCase();
    const hit = q ? rows.filter((r) => modelHits(r, q)) : rows;
    const cur = hit.filter((r) => modelKey(r) === current());
    return [...cur, ...hit.filter((r) => modelKey(r) !== current())].slice(0, 80);
  };
  return (
    <>
      <button
        type="button"
        class="tb-chip tb-chip-model"
        aria-label={`Model: ${label()}. Change model`}
        aria-haspopup="menu"
        aria-expanded={!!anchor()}
        title={modelChipTitle(props.session.state.model)}
        disabled={props.disabled}
        ref={chip}
        onClick={(e) => {
          setQuery('');
          if (anchor()) setAnchor(undefined);
          else setAnchor((e.currentTarget as HTMLElement).getBoundingClientRect());
        }}
        data-testid="model-chip"
      >
        <Cpu size={15} class="tb-ic" />
        <RollingLabel class="tb-label" text={label()} testId="model-chip-label" />
        <ChevronUp size={12} class="tb-chev" />
      </button>
      <Show when={anchor()}>
        {(rect) => (
          <AnchoredMenu anchor={rect()} alignEnd width={300} label="Model" onDismiss={() => setAnchor(undefined)}>
            <div
              class="ws-menu dm-menu"
              data-testid="draft-model-menu"
              ref={(el) => onCleanup(menuKeys(el, { close: () => setAnchor(undefined), trigger: () => chip, autoFocus: false }))}
            >
              <input
                class="dm-search"
                type="search"
                placeholder="Search models"
                aria-label="Search models"
                value={query()}
                onInput={(e) => setQuery(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') {
                    const first = list()[0];
                    if (first) {
                      setAnchor(undefined);
                      props.session.pickModel({ provider: first.provider ?? '', modelId: first.id, name: modelName(first) });
                    }
                  }
                }}
                ref={(el) => queueMicrotask(() => el.focus())}
              />
              <div class="ws-label">Model for this chat</div>
              <Show when={!models.loading} fallback={<div class="ws-browse-note">Loading…</div>}>
                <For each={list()}>
                  {(m) => (
                    <button
                      type="button"
                      role="menuitemradio"
                      aria-checked={modelKey(m) === current()}
                      class="ws-row"
                      data-label={modelName(m)}
                      title={`${modelName(m)} — ${m.provider} · ${m.id}`}
                      onClick={() => {
                        setAnchor(undefined);
                        props.session.pickModel({ provider: m.provider ?? '', modelId: m.id, name: modelName(m) });
                      }}
                    >
                      <Cpu size={15} class="ws-row-ic" />
                      <span class="ws-row-text">
                        <span class="ws-row-label">{modelName(m)}</span>
                        <span class="ws-row-detail">
                          {m.provider} · {m.id}
                        </span>
                      </span>
                      <Show when={modelKey(m) === current()}>
                        <Check size={15} class="ws-row-check" />
                      </Show>
                    </button>
                  )}
                </For>
                <Show when={(models() ?? []).length > 0 && !list().length}>
                  <div class="ws-browse-note">No model matches “{query()}”</div>
                </Show>
                <Show when={!(models() ?? []).length}>
                  <div class="ws-browse-note">The other models show here once a chat on this PC has run.</div>
                </Show>
              </Show>
            </div>
          </AnchoredMenu>
        )}
      </Show>
    </>
  );
}
