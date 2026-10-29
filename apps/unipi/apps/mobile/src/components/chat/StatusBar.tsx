import { For } from 'solid-js';
import { runSegments, type LhRun } from '../../lib/chat/progress';

/**
 * Graph / swarm item bar: done (green) · failed (red) · running (animated)
 * on a grey track for what hasn't started. The value is settled items
 * (done + failed); running ones aren't counted yet.
 */
export function StatusBar(props: { counts: LhRun['counts']; live: boolean; label: string; class?: string; testId?: string }) {
  const settled = () => props.counts.done + props.counts.failed;
  return (
    <div
      class={`status-bar ${props.class ?? ''}`}
      role="progressbar"
      aria-label={props.label}
      aria-valuemin={0}
      aria-valuemax={props.counts.total}
      aria-valuenow={settled()}
      aria-valuetext={`${settled()} of ${props.counts.total} settled`}
      data-live={props.live ? '' : undefined}
      data-testid={props.testId}
    >
      <For each={runSegments(props.counts)}>{(seg) => <span class="status-bar-seg" data-tone={seg.tone} style={{ width: `${seg.pct}%` }} />}</For>
    </div>
  );
}
