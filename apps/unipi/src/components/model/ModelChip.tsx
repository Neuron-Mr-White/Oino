/**
 * Model chip (WP-A14, Z-69/Z-70): the composer's one model entry point —
 * "the model that runs" with a rolling name, opening the one model dialog
 * (`ChatModelPicker`; `data-session` names the chat it belongs to).
 * The chat title and the (deleted) footer tray are not entry points (Z-70);
 * Ctrl/Cmd+M and `/model` reach the same dialog.
 *
 * Owner 2026-11-04: in a chat the chip lives in the **model capsule** — the
 * context ring first, then the name, in one pill (`ModelCapsule`). The
 * capsule is the toolbar's flexible slot: it takes the row's free width up to
 * a cap and shrinks to a floor before the other chips fold (`Toolbar`).
 */
import { createEffect, Show } from 'solid-js';
import { Brain, ChevronUp, Cpu } from 'lucide-solid';
import { layout } from '../../lib/layout';
import { canThink, ensureThinkingLevels, thinkingLevels } from '../../lib/model/thinking';
import type { SessionHandle } from '../../lib/session/store';
import { modKey } from '../../lib/shell';
import { RollingLabel } from '../RollingLabel';
import { modelPickerTarget, openModelPicker } from './target';
import { modelDisplayName, modelOrigin } from '../../lib/model/displayName';
import { ContextRing } from './ContextRing';

/** The chip's label: the running model's display name (never a raw `router/id` path), else a neutral word. */
export function modelChipLabel(model: SessionHandle['state']['model']): string {
  return modelDisplayName(model) || 'Model';
}

/** The chip's tooltip: the full name, where it runs (provider · id) and the shortcut. */
export function modelChipTitle(model: SessionHandle['state']['model']): string {
  const origin = modelOrigin(model);
  return `Model: ${modelChipLabel(model)}${origin ? ` (${origin})` : ''} — change it (${modKey()} M)`;
}

/** A thinking level above Off (the phone model chip's glyph). */
export const thinkingOn = (level: string | undefined): boolean => !!level && level !== 'off';

/** `icon={false}`: inside the capsule the context ring stands where the chip icon was. */
export function ModelChip(props: { session: SessionHandle; disabled?: boolean; icon?: boolean }) {
  const label = () => modelChipLabel(props.session.state.model);
  // Phone: no Thinking chip fetches the levels — the chip's glyph needs them (§3.5).
  createEffect(() => {
    void props.session.state.model?.id;
    void props.session.state.conn;
    if (!layout().desktop()) ensureThinkingLevels(props.session as unknown as SessionHandle);
  });
  const open = () => openModelPicker({ hostId: props.session.hostId, sessionId: props.session.sessionId() });
  return (
    <button
      type="button"
      class="tb-chip tb-chip-model"
      aria-label={`Model: ${label()}. Change model`}
      aria-haspopup="dialog"
      aria-expanded={(() => {
        const t = modelPickerTarget();
        return !!t && t.hostId === props.session.hostId && t.sessionId === props.session.sessionId();
      })()}
      title={modelChipTitle(props.session.state.model)}
      disabled={props.disabled}
      onClick={open}
      data-testid="model-chip"
      data-session={`${props.session.hostId}/${props.session.sessionId()}`}
    >
      <Show when={props.icon !== false}>
        <Cpu size={15} class="tb-ic" />
      </Show>
      <RollingLabel class="tb-label" text={label()} testId="model-chip-label" />
      {/* Phone (§3.5): no Thinking chip — a glyph says the model thinks above Off. */}
      <Show when={!layout().desktop() && thinkingOn(props.session.state.thinkingLevel) && canThink(thinkingLevels(props.session as unknown as SessionHandle))}>
        <Brain size={14} class="tb-think-glyph" aria-hidden="true" />
      </Show>
      <Show when={!props.disabled}>
        <ChevronUp size={12} class="tb-chev" />
      </Show>
    </button>
  );
}

/** The capsule's floor (px): ring + chevron + about twelve characters of the
 *  name on the desktop; the phone row is tighter (ring + the thinking glyph + about four;
 *  a short name such as "GLM 4.7" fits whole). Below the
 *  floor the other chips fold first (`ToolbarItem.min`). */
export const CAPSULE_MIN = 148;
export const CAPSULE_MIN_PHONE = 100;

/** The chat's model capsule: context ring · model name · chevron, one pill. */
export function ModelCapsule(props: { session: SessionHandle; disabled?: boolean }) {
  return (
    <div class="tb-capsule" role="group" aria-label="Model and context" data-testid="model-capsule">
      <ContextRing session={props.session} />
      <ModelChip session={props.session} disabled={props.disabled} icon={false} />
    </div>
  );
}
