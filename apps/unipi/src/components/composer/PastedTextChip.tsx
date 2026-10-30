/**
 * A "Pasted text · N lines" chip (WP-A12, Z-61): a paste of ≥ 5120 chars
 * stays out of the field as a chip; on send it expands inline (the model
 * receives the text). The chip's tooltip carries the paste's head; ✕ drops
 * it. The same row renders the public chip API's entries (WP-B8's
 * element/screenshot chips).
 */
import { ClipboardPaste } from 'lucide-solid';
import type { ComposerChip } from '../../lib/composer/drafts';

export function PastedTextChip(props: { chip: ComposerChip; onRemove: () => void }) {
  return (
    <span
      class="tl-chip cpaste-chip"
      data-testid="pasted-chip"
      data-kind={props.chip.kind}
      title={props.chip.detail ?? props.chip.markdown ?? props.chip.label}
    >
      <ClipboardPaste size={12} />
      <span class="tl-chip-label">{props.chip.label}</span>
      <button type="button" class="tl-chip-x" aria-label={`Remove ${props.chip.label}`} onClick={() => props.onRemove()}>
        ✕
      </button>
    </span>
  );
}
