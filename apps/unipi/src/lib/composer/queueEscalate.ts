/**
 * Escalating send for one queued item (owner-fix-3): the row's send button
 * walks the item up the urgency ladder one press at a time —
 *
 *   follow-up (After turn) --press--> steer (Next step: after the current step)
 *   steer             --press--> send now (abort the run, send it as a prompt)
 *
 * While no run is going (a held queue) there is nothing to interrupt, so a
 * press just delivers the item (the host prompts it; the run drains the rest).
 */
export type QueueKind = 'steering' | 'followUp';

export type EscalateStep = 'steer' | 'interrupt' | 'deliver';

export type EscalateView = { step: EscalateStep; label: string; tooltip: string };

/** The row chip, in plain words: when the queued message reaches the agent. */
export function kindView(queue: QueueKind): { label: string; tooltip: string } {
  return queue === 'steering'
    ? { label: 'Next step', tooltip: 'Sent to the agent after its current step' }
    : { label: 'After turn', tooltip: 'Sent when the agent finishes this turn' };
}

/** What the next press of an item's send button does. */
export function nextEscalation(queue: QueueKind, running: boolean): EscalateStep {
  if (!running) return 'deliver';
  return queue === 'followUp' ? 'steer' : 'interrupt';
}

/** Copy for the button (aria-label / tooltip) given the next step. */
export function escalationView(queue: QueueKind, running: boolean): EscalateView {
  const step = nextEscalation(queue, running);
  switch (step) {
    case 'steer':
      return { step, label: 'Send at next step', tooltip: 'Send at next step — the agent gets it after its current step' };
    case 'interrupt':
      return { step, label: 'Stop the current step and send now', tooltip: 'Stop the current step and send now — press twice to confirm' };
    default:
      return { step, label: 'Send now', tooltip: 'Send now' };
  }
}

/** The queue the item lives in after the press (`interrupt` leaves the queue). */
export function queueAfter(queue: QueueKind, step: EscalateStep): QueueKind | undefined {
  return step === 'steer' ? 'steering' : step === 'deliver' ? queue : undefined;
}
