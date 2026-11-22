/**
 * Under a message the host cut (hotfix 1.0.9): "Loading the rest…" while its full entry is
 * fetched (`lib/session/fullMessages.ts`), a "Show full message" retry button when that failed.
 * Nothing once the full text is in.
 */
import { Loader2 } from 'lucide-solid';
import { Show } from 'solid-js';
import type { TimelineCtx } from './context';

export function FullMessageTail(props: { ctx: TimelineCtx; entryId?: string }) {
  const status = () => (props.entryId ? props.ctx.session.fullMessages?.status(props.entryId) : undefined);
  const reason = () => (props.entryId ? props.ctx.session.fullMessages?.error(props.entryId) : undefined);
  return (
    <>
      <Show when={status() === 'loading'}>
        <div class="tl-fullmsg" role="status" data-testid="full-message-loading">
          <Loader2 size={12} class="tool-more-spin" />
          Loading the rest…
        </div>
      </Show>
      <Show when={status() === 'error'}>
        <div class="tl-fullmsg" role="alert" data-testid="full-message-error">
          <span>Couldn't load the rest{reason() ? ` (${reason()})` : ''}.</span>
          <button type="button" class="tool-more tl-fullmsg-retry" data-testid="full-message-retry" onClick={() => props.ctx.session.fullMessages?.retry([props.entryId!])}>
            Show full message
          </button>
        </div>
      </Show>
    </>
  );
}
