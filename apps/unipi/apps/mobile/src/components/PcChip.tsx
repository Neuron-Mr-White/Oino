/**
 * The one "which PC" chip (UNI-218 F4): 34px pill, status dot, name,
 * chevron. Used by Chat, Board, Files and Preview headers.
 */
import { ChevronDown } from 'lucide-solid';

export function PcChip(props: {
  name?: string;
  /** pc-dot state: ok | checking | error | offline | not_installed */
  state?: string;
  /** Board live LED (data-live on the dot). */
  live?: string;
  label?: string;
  onClick: () => void;
}) {
  return (
    <button type="button" class="pc-chip" aria-label={props.label ?? `PC: ${props.name ?? 'none'}`} onClick={props.onClick}>
      <span class="pc-dot" data-state={props.state ?? 'checking'} data-live={props.live} />
      <span class="truncate">{props.name ?? 'Choose a PC'}</span>
      <ChevronDown size={14} />
    </button>
  );
}
