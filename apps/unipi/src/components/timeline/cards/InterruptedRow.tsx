/**
 * Perf T13: the turn was cut off by a host kill or restart. Core's resume repair appends an
 * `oino.turn-interrupted` custom entry (`{reason, at}`, never sent to the model); this is its
 * one-line row: "Interrupted — Continue". Continue sends the same `/continue` the turn tail's
 * Continue sends (unipi.utility: a hidden turn with no text). The button shows only while the
 * marker is still the chat's latest word — once a message follows (the owner typed, or a
 * long-horizon chat auto-continued) and while a run is going, the row stays as history.
 */
import { Show, createEffect, createMemo, createSignal } from 'solid-js';
import { CirclePause } from 'lucide-solid';
import type { SessionEntry } from '../../../lib/session/types';
import { actionError, showActionNotice } from '../../../lib/timeline/actions';
import type { TimelineCtx } from '../context';

/** Whether no message follows the marker on the loaded branch (the turn is still cut off). */
export function interruptedOpen(entries: readonly SessionEntry[], id: string): boolean {
  const at = entries.findIndex((e) => e.id === id);
  if (at < 0) return false;
  for (let i = at + 1; i < entries.length; i++) if (entries[i]!.type === 'message') return false;
  return true;
}

/**
 * r7 P2-2: how long after the host's ACK a Continue that never starts a run is given up on
 * (the button comes back). Not a timer on the pending command: before the ACK the command's own
 * 30 s budget (`CMD_TIMEOUT_MS`) rejects it; after it, the run normally starts within seconds —
 * 22 s was measured with a woken host under load — and `agent_start` clears it first.
 */
export const CONTINUE_ACKED_FALLBACK_MS = 45_000;

/**
 * Chats whose Continue is on its way (session key → the marker it was sent from), kept outside the
 * row: a remount (virtual list, layout change) must not give the button back, or a second click
 * would send a second `/continue`.
 */
const [continuing, setContinuing] = createSignal<ReadonlyMap<string, string>>(new Map());
const markContinuing = (key: string, id: string | undefined) =>
  setContinuing((m) => {
    if (m.get(key) === id) return m;
    const next = new Map(m);
    if (id === undefined) next.delete(key);
    else next.set(key, id);
    return next;
  });

export function InterruptedRow(props: { entry: SessionEntry; ctx: TimelineCtx }) {
  const state = () => props.ctx.session.state;
  const key = () => props.ctx.session.key;
  const open = createMemo(() => !props.ctx.readOnly && !state().run?.running && interruptedOpen(state().entries, props.entry.id));
  const sending = () => continuing().get(key()) === props.entry.id;
  // D22: Continue is pending from the click until the run starts — not just until the host's ACK,
  // which can come well before the woken runtime reports `agent_start`. Once the run starts, or
  // a message follows the marker, `open()` is false and the pending state goes with it.
  createEffect(() => {
    if (!open() && sending()) markContinuing(key(), undefined);
  });
  const resume = async () => {
    if (sending()) return;
    const k = key();
    const id = props.entry.id;
    markContinuing(k, id);
    try {
      await props.ctx.session.cmd({ type: 'prompt', message: '/continue' });
      // Not tied to the row: it may unmount (virtual list) before the fallback is due.
      setTimeout(() => continuing().get(k) === id && markContinuing(k, undefined), CONTINUE_ACKED_FALLBACK_MS);
    } catch (error) {
      markContinuing(k, undefined);
      showActionNotice(`Couldn't continue: ${actionError(error)}`);
    }
  };
  return (
    <div class="tl-card tl-interrupted" data-kind="interrupted" data-testid="tl-interrupted" title="The host stopped while this turn was running">
      <span class="tool-icon">
        <CirclePause size={15} />
      </span>
      <span class="tl-card-text truncate">Interrupted</span>
      <Show when={open()}>
        <span class="tl-interrupted-sep" aria-hidden="true">—</span>
        <button type="button" class="tl-interrupted-continue" data-testid="tl-interrupted-continue" aria-busy={sending()} data-pending={sending() ? '' : undefined} disabled={sending()} onClick={() => void resume()}>
          {sending() ? 'Continuing…' : 'Continue'}
        </button>
      </Show>
    </div>
  );
}
