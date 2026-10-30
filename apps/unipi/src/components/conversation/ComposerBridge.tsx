/**
 * The A7 → composer handoff (Z-28 edit, Z-30 quote): the interim send box
 * that reads `lib/timeline/actions`' per-session draft — text, quote chips and
 * restored images — and hands them back on send.
 *
 * WP-A9's `BottomDock` takes a `composer` element until WP-A12's real composer
 * lands; this is that element. A12 consumes the same handoff store
 * (`composerDraft`, `composerSendText`, `registerComposerFocus`), so the turn
 * actions and selection quote keep working unchanged.
 */
import { createEffect, For, onCleanup, Show } from 'solid-js';
import { Image as ImageIcon, Quote, SendHorizonal, Square } from 'lucide-solid';
import type { SessionHandle } from '../../lib/session/store';
import {
  clearComposer,
  composerDraft,
  composerSendText,
  excerpt,
  registerComposerFocus,
  removeComposerImage,
  removeComposerQuote,
  setComposerText,
} from '../../lib/timeline/actions';

export function ComposerBridge(props: { session: SessionHandle; readOnly?: boolean }) {
  const key = () => props.session.key;
  const draft = () => composerDraft(key());
  let input: HTMLInputElement | undefined;
  createEffect(() => {
    const k = key();
    const cleanup = registerComposerFocus(k, () => input?.focus());
    onCleanup(cleanup);
  });
  const running = () => !!props.session.state.run.running;
  const send = () => {
    const current = draft();
    const text = composerSendText(current);
    if (props.readOnly || (!text.trim() && !current.images.length)) return;
    props.session.send(text, current.images.length ? { images: current.images } : undefined);
    clearComposer(key());
    if (input) input.value = '';
  };
  const stop = () => {
    if (!props.readOnly) void props.session.abort();
  };
  return (
    <div class="tl-composer-wrap" data-testid="tl-composer-wrap">
      <Show when={draft().quotes.length > 0 || draft().images.length > 0}>
        <div class="tl-composer-chips">
          <For each={draft().quotes}>
            {(quote) => (
              <span class="tl-chip tl-quote-chip" data-testid="tl-quote-chip" title={quote.text}>
                <Quote size={12} />
                <span class="tl-chip-label">{excerpt(quote.text, 72)}</span>
                <button type="button" class="tl-chip-x" aria-label="Remove quote" onClick={() => removeComposerQuote(key(), quote.id)}>
                  ✕
                </button>
              </span>
            )}
          </For>
          <For each={draft().images}>
            {(image, i) => (
              <span class="tl-chip" data-testid="tl-draft-image">
                <ImageIcon size={12} />
                <span class="tl-chip-label">image {i() + 1}</span>
                <button type="button" class="tl-chip-x" aria-label={`Remove image ${i() + 1}`} onClick={() => removeComposerImage(key(), i())}>
                  ✕
                </button>
              </span>
            )}
          </For>
        </div>
      </Show>
      <div class="tl-composer" data-testid="tl-composer" data-readonly={props.readOnly ? '' : undefined}>
        <input
          ref={input}
          class="tl-composer-input"
          value={draft().text}
          disabled={props.readOnly}
          placeholder={
            props.readOnly
              ? 'Read-only — this session is open elsewhere'
              : running()
                ? 'Keep typing to steer…'
                : 'Message…'
          }
          onInput={(e) => setComposerText(key(), e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === 'Escape' && running()) {
              e.preventDefault();
              stop();
              return;
            }
            if (e.key === 'Enter' && !e.shiftKey) {
              e.preventDefault();
              send();
            }
          }}
        />
        <Show
          when={running()}
          fallback={
            <button type="button" class="tl-composer-send" aria-label="Send" onClick={send} disabled={props.readOnly}>
              <SendHorizonal size={17} />
            </button>
          }
        >
          <button type="button" class="tl-composer-send tl-composer-stop" aria-label="Stop" onClick={stop} disabled={props.readOnly}>
            <Square size={15} fill="currentColor" />
          </button>
        </Show>
      </div>
    </div>
  );
}
