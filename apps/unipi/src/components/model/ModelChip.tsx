/**
 * Model chip (WP-A14, Z-69/Z-70): the composer's one model entry point —
 * "the model that runs" with a rolling name, opening the one model dialog
 * (`ChatModelPicker`; `data-session` names the chat it belongs to).
 * The chat title and the (deleted) footer tray are not entry points (Z-70);
 * Ctrl/Cmd+M and `/model` reach the same dialog.
 */
import { Show } from 'solid-js';
import { Brain, ChevronUp, Cpu } from 'lucide-solid';
import { layout } from '../../lib/layout';
import type { SessionHandle } from '../../lib/session/store';
import { modKey } from '../../lib/shell';
import { RollingLabel } from '../RollingLabel';
import { modelPickerTarget, openModelPicker } from './target';
import { modelDisplayName, modelOrigin } from '../../lib/model/displayName';

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

export function ModelChip(props: { session: SessionHandle; disabled?: boolean }) {
  const label = () => modelChipLabel(props.session.state.model);
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
      <Cpu size={15} class="tb-ic" />
      <RollingLabel class="tb-label" text={label()} testId="model-chip-label" />
      {/* Phone (§3.5): no Thinking chip — a glyph says the model thinks above Off. */}
      <Show when={!layout().desktop() && thinkingOn(props.session.state.thinkingLevel)}>
        <Brain size={14} class="tb-think-glyph" aria-hidden="true" />
      </Show>
      <Show when={!props.disabled}>
        <ChevronUp size={12} class="tb-chev" />
      </Show>
    </button>
  );
}
