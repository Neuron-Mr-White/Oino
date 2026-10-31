/**
 * The draft's model chip (Z-19): looks and sits exactly like the chat's
 * model chip; the chat's picker sheet is session-bound, so the draft opens a
 * compact list instead — this PC's model catalogue (borrowed from a live chat
 * on the same PC when one is open) or, before any chat ran, the default. The
 * pick is applied to the new session before its first prompt.
 */
import { createResource, createSignal, For, Show } from 'solid-js';
import { Check, ChevronUp, Cpu } from 'lucide-solid';
import { AnchoredMenu } from '../board/AnchoredMenu';
import { modelKey, modelName, parseModels, type ModelInfo } from '../../lib/model/picker';
import { modelChipLabel, modelChipTitle } from '../model/ModelChip';
import { RollingLabel } from '../RollingLabel';
import type { DraftSession } from './draftSession';

export function DraftModelChip(props: { session: DraftSession; disabled?: boolean }) {
  const [anchor, setAnchor] = createSignal<DOMRect | undefined>();
  const label = () => modelChipLabel(props.session.state.model);
  const current = () => modelKey(props.session.state.model ?? {});
  const [models] = createResource(
    () => (anchor() ? props.session.hostId : undefined),
    // The draft answers from a live chat on this PC, else the list it last saw (P0-09).
    async (): Promise<ModelInfo[]> => parseModels(await props.session.cmd<unknown>({ type: 'get_available_models' }).catch(() => undefined)),
  );
  const list = (): ModelInfo[] => {
    const all = models() ?? [];
    const m = props.session.state.model;
    if (all.length || !m?.id) return all;
    return [{ provider: m.provider ?? '', id: m.id, name: m.name ?? m.id } as ModelInfo];
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
        onClick={(e) => (anchor() ? setAnchor(undefined) : setAnchor((e.currentTarget as HTMLElement).getBoundingClientRect()))}
        data-testid="model-chip"
      >
        <Cpu size={15} class="tb-ic" />
        <RollingLabel class="tb-label" text={label()} testId="model-chip-label" />
        <ChevronUp size={12} class="tb-chev" />
      </button>
      <Show when={anchor()}>
        {(rect) => (
          <AnchoredMenu anchor={rect()} alignEnd width={300} label="Model" onDismiss={() => setAnchor(undefined)}>
            <div class="ws-menu dm-menu" data-testid="draft-model-menu">
              <div class="ws-label">Model for this chat</div>
              <Show when={!models.loading} fallback={<div class="ws-browse-note">Loading…</div>}>
                <For each={list()}>
                  {(m) => (
                    <button
                      type="button"
                      role="menuitemradio"
                      aria-checked={modelKey(m) === current()}
                      class="ws-row"
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
