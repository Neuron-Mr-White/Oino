/**
 * The status pane's Running section: the Session modal's Running view itself
 * ({@link RunningTab} — one shared row component: status icon, name, chips,
 * metrics, Stop, open) over the same snapshot hook, so the two cannot differ.
 * A subagent row opens its side-pane subagent tab, a background task row its
 * live terminal tab (the modal's handlers).
 */
import { createEffect, createMemo, on, onCleanup } from 'solid-js';
import { sessions } from '../../lib/session/store';
import { RunningTab } from '../session/RunningTab';
import '../session/session-modal.css';
import './status.css';

export function RunningSummary(props: { hostId: string; sessionId: string }) {
  const s = createMemo(() => sessions().ensure(props.hostId, props.sessionId));
  createEffect(
    on(s, (handle) => {
      const release = handle.acquire();
      onCleanup(release);
    }),
  );
  return (
    <section class="status-section status-running" data-testid="running-summary" aria-label="Running">
      <RunningTab s={s()} />
    </section>
  );
}
