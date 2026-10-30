/**
 * A failed turn's error row (WP-A9, Z-48): unipi's red row with a "Details"
 * disclosure holding the **raw** error and a Copy action (the hybrid the
 * audit recommends — the row stays scannable, the provider text is one tap
 * away). The raw text is what the provider or extension said, never a
 * rephrasing.
 *
 * Retry is not here: Z-49 moved it to the turn tail (WP-A7), so there is one
 * retry surface per turn, not two.
 */
import { createSignal, Show } from 'solid-js';
import { Check, ChevronRight, CircleAlert, Copy } from 'lucide-solid';
import { copyText } from '../../lib/board/share';

/** The row's line: the first non-empty line of the raw error. */
export function errorHeadline(text: string): string {
  const line = text.split('\n').map((l) => l.trim()).find(Boolean);
  return line ?? 'The turn failed.';
}

export function ErrorRow(props: { text: string; testId?: string }) {
  const [open, setOpen] = createSignal(false);
  const [copied, setCopied] = createSignal(false);
  const copy = () => {
    void copyText(props.text).then((ok) => {
      if (!ok) return;
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    });
  };
  return (
    <div class="chat-error tl-error" role="alert" data-testid={props.testId ?? 'tl-error'}>
      <CircleAlert size={16} class="shrink-0" />
      <div class="tl-error-main">
        <span class="tl-error-text">{errorHeadline(props.text)}</span>
        <button
          type="button"
          class="tl-error-details"
          aria-expanded={open()}
          onClick={() => setOpen(!open())}
          data-testid="tl-error-details"
        >
          <ChevronRight size={13} class="tl-error-caret" data-open={open() ? '' : undefined} />
          Details
        </button>
        <Show when={open()}>
          <pre class="tl-error-raw" data-testid="tl-error-raw">
            {props.text}
          </pre>
          <button type="button" class="tl-error-copy" onClick={copy} aria-label="Copy the error">
            <Show when={copied()} fallback={<><Copy size={13} /> Copy</>}>
              <Check size={13} /> Copied
            </Show>
          </button>
        </Show>
      </div>
    </div>
  );
}
