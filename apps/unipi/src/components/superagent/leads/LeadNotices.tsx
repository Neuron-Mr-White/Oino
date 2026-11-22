import { createSignal, For } from 'solid-js';
import { X } from 'lucide-solid';
import type { SuperagentState } from '../state';
import { dismissNotice, noticeKey, readDismissed, visibleNotices } from './notices';

/** One dismissible line per host notice (OIN-82: "Board created for Orchard"). Dismissal is remembered on this device per project and moment. */
export function LeadNotices(props: { sa: SuperagentState }) {
  const [dismissed, setDismissed] = createSignal(readDismissed());
  const items = () => visibleNotices(props.sa.status()?.notices, dismissed());
  return <For each={items()}>{(n) => <p class="sa-notice" role="status" data-testid="sa-notice" data-kind={n.kind}>
    <span class="sa-notice-text">Board created for {n.name || props.sa.projectName(n.projectId!)}</span>
    <button type="button" class="sa-icon-btn" aria-label={`Dismiss: board created for ${n.name || props.sa.projectName(n.projectId!)}`} data-key={noticeKey(n)} onClick={() => setDismissed(dismissNotice(n))}><X size={14} aria-hidden="true" /></button>
  </p>}</For>;
}
