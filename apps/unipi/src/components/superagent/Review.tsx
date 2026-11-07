import { createEffect, createSignal, Show } from 'solid-js';
import { ChevronRight } from 'lucide-solid';
import type { SuperagentState } from './state';
import { Limited } from './parts';

/** Tickets a lead finished: collapsed by default. The protocol has no accept op, so review happens in the ticket. */
export function ReviewSection(props: { sa: SuperagentState; onTicket: (project: string, task: string) => void }) {
  const [open, setOpen] = createSignal(false);
  const items = () => props.sa.status()?.review ?? [];
  createEffect(() => { if (open()) void props.sa.ensureTitles(items()); });
  return <section class="sa-sec sa-review" aria-label="Review" data-testid="sa-review">
    <button type="button" class="sa-disclosure" aria-expanded={open()} onClick={() => setOpen(!open())}>
      <ChevronRight size={16} class="sa-chev" /><h2>Review</h2><span class="sa-badge" data-zero={items().length === 0 ? '' : undefined}>{items().length}</span>
    </button>
    <Show when={open()}>
      <Show when={items().length} fallback={<p class="sa-empty">Nothing to review.</p>}>
        <ul class="sa-review-list">
          <Limited items={items()} limit={4}>{(r) => <li><button type="button" class="sa-review-row" aria-label={`Open ${r.taskId}`} onClick={() => props.onTicket(r.projectId, r.taskId)}>
            <span class="sa-project">{props.sa.projectName(r.projectId)}</span>
            <strong>{r.taskId}{props.sa.meta(r.projectId, r.taskId)?.title ? ` · ${props.sa.meta(r.projectId, r.taskId)!.title}` : ''}</strong>
          </button></li>}</Limited>
        </ul>
      </Show>
    </Show>
  </section>;
}
