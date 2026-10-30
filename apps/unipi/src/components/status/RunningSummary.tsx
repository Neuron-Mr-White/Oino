/**
 * The status pane's compact Running summary (WP-A16, RULINGS Z-95): counts +
 * up to 3 rows over the same `fetchRunning` snapshot the Session modal's
 * Running tab reads (shared selectors in `lib/status/running.ts` — the
 * numbers cannot disagree). "Open in Session" goes to the modal's Running
 * tab — the only full Running view (D-7); Alt+B sends subagents to the
 * background; work-tray rows route there too (G-20).
 */
import { createEffect, createMemo, createResource, For, on, onCleanup, Show } from 'solid-js';
import { ArrowRight, SquareTerminal } from 'lucide-solid';
import { sessions } from '../../lib/session/store';
import { fetchRunning } from '../session/RunningTab';
import { liveSource } from '../session/modalData';
import { openSessionModal } from '../session/modalTarget';
import { runningCounts, runningRows, runningTotal, type RunningRow } from '../../lib/status/running';
import './status.css';

const ROW_ICON: Record<RunningRow['kind'], string> = { bash: 'bash', agent: 'agent', sidekick: 'fusion', tray: 'tray' };

export function RunningSummary(props: { hostId: string; sessionId: string }) {
  const s = createMemo(() => sessions().ensure(props.hostId, props.sessionId));
  createEffect(
    on(s, (handle) => {
      const release = handle.acquire();
      onCleanup(release);
    }),
  );
  const [data, { refetch }] = createResource(liveSource(s), async (state) => (state === 'live' ? fetchRunning(s()) : undefined));
  // Live sessions keep moving: refresh on run-state changes (seq), throttled like the modal's tab.
  let last = 0;
  createEffect(() => {
    void s().state.seq;
    if (Date.now() - last >= 2000) {
      last = Date.now();
      refetch();
    }
  });
  const counts = createMemo(() => runningCounts(data.latest));
  const rows = createMemo(() => runningRows(data.latest));
  const open = () => openSessionModal({ hostId: props.hostId, sessionId: props.sessionId, tab: 'running' });
  const toBackground = () => void s().cmd({ type: '_oino/invoke_shortcut', shortcutId: 'unipi.subagents.foreground' }).catch(() => {});

  return (
    <section class="status-section" data-testid="running-summary" aria-label="Running">
      <div class="status-section-head">
        <span class="status-section-title">
          <SquareTerminal size={13} /> Running
        </span>
        <span class="status-dim" data-testid="running-counts">
          <Show when={counts().bash > 0}>{counts().bash} shell{counts().bash === 1 ? '' : 's'} · </Show>
          <Show when={counts().agents > 0}>{counts().agents} agent{counts().agents === 1 ? '' : 's'} · </Show>
          <Show when={counts().tray > 0}>{counts().tray} tray · </Show>
          <Show when={runningTotal(counts()) === 0} fallback={<span data-testid="running-total">{runningTotal(counts())}</span>}>
            nothing
          </Show>
        </span>
        <Show when={counts().agents > 0}>
          <button type="button" class="status-action" data-testid="running-background" title="All to background (Alt+B)" onClick={toBackground}>
            Background
          </button>
        </Show>
        <button type="button" class="status-link" data-testid="running-open" onClick={open}>
          Open in Session <ArrowRight size={11} />
        </button>
      </div>
      <Show
        when={rows().length > 0}
        fallback={
          <button type="button" class="status-empty status-empty-link" onClick={open}>
            Nothing running. The Session modal has the full view.
          </button>
        }
      >
        <div class="status-rows">
          <For each={rows()}>
            {(row) => (
              <button type="button" class="status-row" data-kind={ROW_ICON[row.kind]} data-testid="running-row" title={row.title} onClick={open}>
                <span class="status-dot" data-state={row.state} />
                <span class="status-row-main">
                  <span class="status-row-title truncate">{row.title}</span>
                  <Show when={row.sub}>{(sub) => <span class="status-row-sub truncate">{sub()}</span>}</Show>
                </span>
              </button>
            )}
          </For>
          <Show when={runningTotal(counts()) > rows().length}>
            <button type="button" class="status-row status-row-more" onClick={open} data-testid="running-more">
              +{runningTotal(counts()) - rows().length} more in Session
            </button>
          </Show>
        </div>
      </Show>
    </section>
  );
}
