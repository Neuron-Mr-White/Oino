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
import { createMemo, createSignal, Show } from 'solid-js';
import { expansionOpen, setExpansion } from '../../lib/timeline/expansion';
import { Check, ChevronRight, CircleAlert, Copy } from 'lucide-solid';
import type { TimelineCtx } from './context';
import { newestErrorKey } from '../../lib/session/turns';
import { copyText } from '../../lib/board/share';

/** The row's line: the first non-empty line of the raw error. */
export function errorHeadline(text: string): string {
  const line = text.split('\n').map((l) => l.trim()).find(Boolean);
  if (!line) return 'The turn failed.';
  // F-02: a provider's raw JSON body is never the headline — it stays under Details.
  if (/^(\d{3}\s+)?[{[]/.test(line) || /"type"\s*:\s*"error"/.test(line)) {
    if (/\b(401|403)\b|authentication|unauthori[sz]ed|invalid.{0,12}key/i.test(line)) return 'This model’s provider rejected the key.';
    if (/\b429\b|rate.?limit|quota|overloaded/i.test(line)) return 'This model’s provider is limiting requests.';
    if (/\b404\b|not.found/i.test(line)) return 'This provider has no such model.';
    return 'This model’s provider returned an error.';
  }
  return line;
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
  /** Open state by item (session + key) in the expansion store — survives virtualizer slot reuse. */
  foldKey?: { sessionId: string; key: string };
  /** Consecutive identical errors folded into this one (shown as a muted "×N"). */
  count?: number;
  /** Set on an expanded older error: clicking the message folds it back to its one-line row. */
  onCollapse?: () => void;
}) {
  const [localOpen, setLocalOpen] = createSignal(false);
  const open = () => (props.foldKey ? expansionOpen(props.foldKey.sessionId, props.foldKey.key, false) : localOpen());
  const setOpen = (v: boolean) => (props.foldKey ? setExpansion(props.foldKey.sessionId, props.foldKey.key, v) : setLocalOpen(v));
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
        <span class="tl-error-text" data-collapsible={props.onCollapse ? '' : undefined} title={props.onCollapse ? 'Collapse' : undefined} onClick={props.onCollapse}>
          {errorHeadline(props.text)}
          <Show when={(props.count ?? 1) > 1}>
            {' '}
            <span class="tl-error-count" data-testid="tl-error-count">×{props.count}</span>
          </Show>
        </span>
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

/**
 * A failed turn's error in the timeline: only the newest error of the whole chat is a full card;
 * older ones (or folded runs) are one muted line that opens in place to the card and folds back.
 */
export function TimelineError(props: { text: string; ctx: TimelineCtx; itemKey: string; count?: number; model?: string }) {
  const newest = createMemo(() => {
    const key = newestErrorKey(props.ctx.session.turns());
    return key === undefined || key === props.itemKey;
  });
  const [expanded, setExpanded] = createSignal(false);
  const card = () => (
    <ErrorRow
      text={props.text}
      count={props.count}
      model={props.model}
      foldKey={{ sessionId: props.ctx.sessionId(), key: `error:${props.itemKey}` }}
      onCollapse={newest() ? undefined : () => setExpanded(false)}
    />
  );
  return (
    <Show when={newest() || expanded()} fallback={<ErrorLine text={props.text} count={props.count} onOpen={() => setExpanded(true)} />}>
      {card()}
    </Show>
  );
}

function ErrorLine(props: { text: string; count?: number; onOpen: () => void }) {
  const headline = () => errorHeadline(props.text);
  return (
    <button type="button" class="tl-error-line" aria-expanded="false" onClick={props.onOpen} title={headline()} data-testid="tl-error-line">
      <CircleAlert size={15} class="shrink-0" />
      <span class="tl-error-line-text">{headline()}</span>
      <Show when={(props.count ?? 1) > 1}>
        <span class="tl-error-count">×{props.count}</span>
      </Show>
      <ChevronRight size={15} class="tl-error-line-chev" />
    </button>
  );
}
