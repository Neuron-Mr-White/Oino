import { isUnsupportedOp, type LeadAction, type LeadState } from '../api';

/** The owner's words for a lead's state (OIN-80). Colours and icons are the canvas's: green moves, amber Hand, red TriangleAlert, Moon. */
export const LEAD_STATE_LABEL: Record<LeadState, string> = {
  working: 'Working', waiting: 'Waiting for you', idle: 'Idle', unloaded: 'Unloaded (wakeable)', stalled: 'Stalled',
};

/** Most urgent first: a stalled lead needs a decision, a waiting one an answer. Equal states keep the host's order. */
export const LEAD_STATE_RANK: Record<LeadState, number> = { stalled: 0, waiting: 1, working: 2, idle: 3, unloaded: 4 };

/** "<1 min", "12 min", "2 h 05 min": the minute precision the question timers and the idle line use. */
export function minutesText(ms: number): string {
  const minutes = Math.floor(Math.max(0, ms) / 60_000);
  if (minutes < 1) return '<1 min';
  if (minutes < 60) return `${minutes} min`;
  const rest = minutes % 60;
  return `${Math.floor(minutes / 60)} h${rest ? ` ${String(rest).padStart(2, '0')} min` : ''}`;
}

/** What a lead action's refusal reads like. An older host answers "unknown method": say so, never pretend. The host's own words win for its refusals; the fallbacks cover an empty message. */
export function leadActionError(error: { code?: unknown; message?: string }): string {
  const text = (error.message ?? '').trim();
  if (isUnsupportedOp(error.code) || /unknown (method|op)/i.test(text)) return `This host can't do that yet${text ? ` (${text})` : ''}. Update Unipi on the PC.`;
  if (text) return text;
  switch (error.code) {
    case 'E_LEAD_STATE': return 'The lead is not in a state that allows that. Refresh and try again.';
    case 'E_LEAD_UNKNOWN': return 'That lead no longer exists.';
    case 'E_UNKNOWN_MODEL': return 'The host does not know that model.';
    default: return 'The host refused.';
  }
}

export const LEAD_ACTION_DONE: Record<LeadAction, (name: string) => string> = {
  retry: (name) => `Retrying the ${name} lead.`,
  wake: (name) => `Waking the ${name} lead.`,
  switch_model: (name) => `Switched the ${name} lead to the new model.`,
  stop: (name) => `The ${name} lead stopped.`,
};
