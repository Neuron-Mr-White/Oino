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

/** A short next step for the errors a user can act on (D-24); undefined for the rest. */
export function errorHint(text: string): string | undefined {
  if (/\b(401|403)\b|unauthori[sz]ed|invalid api key|user not found|authentication/i.test(text)) return 'The provider rejected the credential. Check the key or sign in again in Settings › Providers, or pick another model.';
  if (/\b429\b|rate.?limit|quota|overloaded|\b529\b/i.test(text)) return 'The provider is limiting or overloaded. Wait a moment and retry, or pick another model.';
  if (/\b(404)\b|model.*not found|unknown model/i.test(text)) return 'This provider has no such model. Pick another model.';
  if (/network|fetch failed|ECONN|ENOTFOUND|timed? ?out/i.test(text)) return 'The provider could not be reached. Check the connection and retry.';
  return undefined;
}

export function ErrorRow(props: {
  text: string;
  testId?: string;
  /** The model that failed (D-24): "Provider · model-id", shown with the raw error. */
  model?: string;
}) {
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
          <Show when={props.model || errorHint(props.text)}>
            <dl class="tl-error-facts" data-testid="tl-error-facts">
              <Show when={props.model}>
                <dt>Model</dt>
                <dd>{props.model}</dd>
              </Show>
              <Show when={errorHint(props.text)}>
                <dt>What to do</dt>
                <dd>{errorHint(props.text)}</dd>
              </Show>
            </dl>
          </Show>
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
