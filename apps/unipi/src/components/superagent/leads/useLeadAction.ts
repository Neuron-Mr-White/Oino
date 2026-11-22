import { createSignal } from 'solid-js';
import { toAppError } from '@unipi/protocol';
import { confirmSheet } from '../../phone';
import { superagentApi, type LeadAction } from '../api';
import type { SuperagentState } from '../state';
import { LEAD_ACTION_DONE, leadActionError } from './lead';

/** One lead's health actions (`superagent.lead_action`): one request at a time, the host's refusal kept as text, never a silent success. */
export function useLeadAction(props: { hostId: () => string; sa: Pick<SuperagentState, 'refresh' | 'flash'>; sessionId: () => string; name: () => string }) {
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal('');
  async function run(action: LeadAction, model?: string): Promise<boolean> {
    if (busy()) return false;
    if (action === 'stop' && !(await confirmSheet({ title: `Stop the ${props.name()} lead?`, body: 'It stops working on its tickets. They move to Backlog so Auto-start does not relaunch them.', confirm: 'Stop lead', tone: 'danger' }))) return false;
    setBusy(true); setError('');
    try {
      const result = await superagentApi.leadAction(props.hostId(), props.sessionId(), action, model);
      if (!result.ok) { setError(leadActionError(result.error)); return false; }
      props.sa.flash(LEAD_ACTION_DONE[action](props.name()));
      await props.sa.refresh();
      return true;
    } catch (e) { setError(leadActionError(toAppError(e))); return false; }
    finally { setBusy(false); }
  }
  return { busy, error, run, clear: () => setError('') };
}
