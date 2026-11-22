/**
 * [ui-chat-bits] The chat header's cron marker: a chat the cron scheduler started
 * (card `source: "cron"`) shows a small "Cron · <job>" chip; it opens that job on the
 * Cron page (`/cron?job=<id>&host=<hostId>`). The job id/name ride the card
 * (`cron_job_id` / `cron_job_name`, host-filled from the cron runs store); an older
 * host without them falls back to the run label lookup (`cronRunLabel`).
 */
import { Show } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { Clock } from 'lucide-solid';
import { hubs } from '../../lib/session/hub';
import { cronRunLabel } from '../../lib/cronRunLabels';
import './cronChip.css';

/** The job behind a cron-run chat, or `undefined` for any other chat. Pure over the card. */
export function cronJobOf(card: { source?: string; cron_job_id?: string; cron_job_name?: string } | undefined, fallbackName?: string): { id?: string; name?: string } | undefined {
  if (card?.source !== 'cron') return undefined;
  return { id: card.cron_job_id, name: card.cron_job_name || fallbackName || card.cron_job_id };
}

/** `/cron` deep link opening one job's detail. */
export const cronJobHref = (hostId: string, jobId?: string): string =>
  jobId ? `/cron?job=${encodeURIComponent(jobId)}&host=${encodeURIComponent(hostId)}` : `/cron?host=${encodeURIComponent(hostId)}`;

export function CronChip(props: { hostId: string; sessionId: string }) {
  const navigate = useNavigate();
  const card = () => hubs().ensure(props.hostId).state.cards[props.sessionId];
  const job = () => {
    const c = card();
    if (c?.source !== 'cron') return undefined;
    // Older hosts don't name the job on the card: ask the runs store (lazy, cached).
    const fallback = c.cron_job_name ? undefined : cronRunLabel(props.hostId, props.sessionId)?.jobName;
    return cronJobOf(c, fallback);
  };
  return (
    <Show when={job()}>
      {(j) => (
        <button
          type="button"
          class="cron-head-chip"
          data-testid="conv-cron-chip"
          title={j().name ? `Started by the automation “${j().name}” — open it in Automations` : 'Started by an automation — open Automations'}
          aria-label={j().name ? `Automation ${j().name}` : 'Automation'}
          onClick={() => navigate(cronJobHref(props.hostId, j().id))}
        >
          <Clock size={13} aria-hidden="true" />
          <span class="cron-head-chip-label">
            Cron
            <Show when={j().name}>
              <span class="cron-head-chip-job"> · {j().name}</span>
            </Show>
          </span>
        </button>
      )}
    </Show>
  );
}
