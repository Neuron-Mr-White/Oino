/**
 * Model chip (WP-A14, Z-69/Z-70): the composer's one model entry point —
 * "the model that runs" with a rolling name, opening the picker sheet.
 * The chat title and the (deleted) footer tray are not entry points (Z-70);
 * Ctrl/Cmd+M and `/model` reach the same sheet.
 */
import { Show } from 'solid-js';
import { ChevronUp, Cpu } from 'lucide-solid';
import type { SessionHandle } from '../../lib/session/store';
import { modKey } from '../../lib/shell';
import { RollingLabel } from '../RollingLabel';
import { openModelPicker } from './target';
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

export function ModelChip(props: { session: SessionHandle; disabled?: boolean }) {
  const label = () => modelChipLabel(props.session.state.model);
  const open = () => openModelPicker({ hostId: props.session.hostId, sessionId: props.session.sessionId() });
  return (
    <button
      type="button"
      class="tb-chip tb-chip-model"
      aria-label={`Model: ${label()}. Change model`}
      aria-haspopup="dialog"
      title={modelChipTitle(props.session.state.model)}
      disabled={props.disabled}
      onClick={open}
      data-testid="model-chip"
    >
      <Cpu size={15} class="tb-ic" />
      <RollingLabel class="tb-label" text={label()} testId="model-chip-label" />
      <Show when={!props.disabled}>
        <ChevronUp size={12} class="tb-chev" />
      </Show>
    </button>
  );
}
