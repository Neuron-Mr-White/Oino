/**
 * Reference-only entries the host did not send in full (`{id, truncated: true}`) have no row type, so
 * nothing in the list can show them. While the app fetches them this says so; when that failed (after its
 * own retries) it says so too and offers a retry, instead of leaving the chat silently incomplete (G3-1).
 */
import { Loader2 } from 'lucide-solid';
import { Show } from 'solid-js';
import { isStub } from '../../lib/session/fullMessages';
import type { TimelineCtx } from './context';

export function StubNotice(props: { ctx: TimelineCtx }) {
  const ids = () => props.ctx.session.state.entries.filter(isStub).map((e) => e.id);
  const statuses = () => ids().map((id) => props.ctx.session.fullMessages?.status(id));
  const failed = () => statuses().some((s) => s === 'error');
  const reason = () => {
    const full = props.ctx.session.fullMessages;
    return ids().map((id) => full?.error(id)).find((r) => r !== undefined);
  };
  return (
    <Show when={ids().length > 0}>
      <Show
        when={failed()}
        fallback={
          <div class="tl-fullmsg tl-stubs" role="status" data-testid="stubs-loading">
            <Loader2 size={12} class="tool-more-spin" />
            Loading the rest of this chat…
          </div>
        }
      >
        <div class="tl-fullmsg tl-stubs" role="alert" data-testid="stubs-failed">
          <span>Some of this chat could not be loaded{reason() ? ` (${reason()})` : ''}.</span>
          <button type="button" class="tool-more tl-fullmsg-retry" data-testid="stubs-retry" onClick={() => props.ctx.session.fullMessages?.retry(ids())}>
            Retry
          </button>
        </div>
      </Show>
    </Show>
  );
}
