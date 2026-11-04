/**
 * Escalating send for one queued item (owner-fix-3): the row's send button
 * walks the item up the urgency ladder one press at a time —
 *
 *   follow-up (QUEUE) --press--> steer (delivered at the next tool boundary)
 *   steer             --press--> send now (abort the run, send it as a prompt)
 *
 * While no run is going (a held queue) there is nothing to interrupt, so a
 * press just delivers the item (the host prompts it; the run drains the rest).
 */
export type QueueKind = 'steering' | 'followUp';

export type EscalateStep = 'steer' | 'interrupt' | 'deliver';

export type EscalateView = { step: EscalateStep; label: string; tooltip: string };

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
      return { step, label: 'Steer now', tooltip: 'Steer now — deliver at the next tool boundary' };
    case 'interrupt':
      return { step, label: 'Send now (interrupts)', tooltip: 'Send now (interrupts) — stops the current turn and sends this' };
    default:
      return { step, label: 'Send now', tooltip: 'Send now' };
  }
}

/** The queue the item lives in after the press (`interrupt` leaves the queue). */
export function queueAfter(queue: QueueKind, step: EscalateStep): QueueKind | undefined {
  return step === 'steer' ? 'steering' : step === 'deliver' ? queue : undefined;
}
