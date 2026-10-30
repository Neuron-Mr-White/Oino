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

/** The chip's label: the running model's name, else its id, else a neutral word. */
export function modelChipLabel(model: SessionHandle['state']['model']): string {
  if (!model) return 'Model';
  const name = typeof model.name === 'string' && model.name.trim() ? model.name : undefined;
  return name ?? model.id ?? 'Model';
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
      title={`Model: ${label()} — change it (${modKey()} M)`}
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
