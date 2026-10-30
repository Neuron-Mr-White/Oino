/**
 * The collapsed-pane capsule (WP-A16, §3.10): top-right of the timeline —
 * branch ±, LH mode + progress, running count — shown while the status pane
 * is closed or overlaying (push mode has no capsule). Click opens/focuses
 * the status pane (`openStatusSection`). The branch stays fresh through the
 * host's git watcher (Z-98, refcounted like the pane's); LH and running come
 * from the session's sticky topics — no polling.
 */
import { createEffect, createMemo, createSignal, on, onCleanup, onMount, Show } from 'solid-js';
import { git } from '../../lib/git';
import { hubs } from '../../lib/session/hub';
import { sessions } from '../../lib/session/store';
import { openStatusSection } from './StatusPane';
import { ModeIcon } from '../chat/ModeChip';
import { compactCount } from '../../lib/git';
import { lhSummary, LH_PROGRESS_TOPIC, LH_STATE_TOPIC, parseLhProgress, parseLhState } from '../../lib/status/lh';
import { BG_TASKS_TOPIC, SUBAGENTS_LIST_TOPIC, topicRunningCount } from '../../lib/status/running';
import './status.css';

type Branch = { name: string; add?: number; del?: number };

export function Capsule(props: { hostId: string; sessionId?: string; cwd?: string }) {
  const [branch, setBranch] = createSignal<Branch | undefined>(undefined);
  const [watchedRoot, setWatchedRoot] = createSignal<string | undefined>(undefined);
  const hub = createMemo(() => hubs().ensure(props.hostId));

  async function refresh() {
    const path = props.cwd || '~';
    const repo = await git.repo(props.hostId, path);
    if (!repo.ok || !repo.value.repo) {
      setBranch(undefined);
      return;
    }
    const stat = await git.shortstat(props.hostId, repo.value.root ?? path);
    const s = stat.ok && stat.value.repo ? stat.value : undefined;
    setBranch({ name: repo.value.branch || repo.value.head || 'no branch', ...(s ? { add: s.add, del: s.del } : {}) });
    // Z-98: keep the watcher only while the capsule is mounted (refcounted).
    const root = repo.value.root ?? path;
    const prev = watchedRoot();
    if (root !== prev) {
      setWatchedRoot(root);
      void git.watch(props.hostId, root, true);
      if (prev) void git.watch(props.hostId, prev, false);
    }
  }
  onMount(() => void refresh());
  onCleanup(() => {
    const root = watchedRoot();
    if (root) void git.watch(props.hostId, root, false);
  });
  createEffect(
    on(
      () => (watchedRoot() ? hub().state.git[watchedRoot()!] : undefined),
      (tick) => {
        if (tick === undefined) return;
        void refresh();
      },
      { defer: true },
    ),
  );

  const session = createMemo(() => (props.sessionId ? sessions().peek(props.hostId, props.sessionId) : undefined));
  createEffect(
    on(session, (s) => {
      if (!s) return;
      s.subscribeTopics([LH_STATE_TOPIC, LH_PROGRESS_TOPIC, BG_TASKS_TOPIC, SUBAGENTS_LIST_TOPIC]);
    }),
  );
  const lh = createMemo(() => {
    const s = session();
    if (!s) return undefined;
    return lhSummary(parseLhState(s.state.topics[LH_STATE_TOPIC]?.data), parseLhProgress(s.state.topics[LH_PROGRESS_TOPIC]?.data));
  });
  const running = createMemo(() => {
    const s = session();
    const n = s ? topicRunningCount(s.state.topics) : 0;
    return n > 0 ? n : undefined;
  });
  const open = () => openStatusSection({ hostId: props.hostId, sessionId: props.sessionId, path: props.cwd || '~' });

  return (
    <Show when={branch() || lh() || running()}>
      <div class="status-capsule" data-testid="status-capsule">
        <button type="button" class="status-capsule-btn" aria-label="Open the status pane" title="Git, long-horizon, todos, running" onClick={open}>
          <Show when={branch()}>
            {(b) => (
              <span class="cap-seg cap-branch" data-testid="cap-branch">
                {b().name}
                <Show when={(b().add ?? 0) + (b().del ?? 0) > 0}>
                  {' '}
                  <span class="cap-add">+{compactCount(b().add ?? 0)}</span> <span class="cap-del">−{compactCount(b().del ?? 0)}</span>
                </Show>
              </span>
            )}
          </Show>
          <Show when={lh()}>
            {(m) => (
              <span class="cap-seg cap-lh" data-mode={m().mode} data-phase={m().phase} data-testid="cap-lh">
                <ModeIcon mode={m().mode} size={12} />
                <span class="cap-lh-label">{m().label}</span>
                <Show when={m().countText}>
                  {(c) => <span class="cap-lh-count">{c()}</span>}
                </Show>
                <Show when={m().phase === 'active'}>
                  <span class="cap-pulse" aria-hidden="true" />
                </Show>
              </span>
            )}
          </Show>
          <Show when={running()}>
            {(n) => (
              <span class="cap-seg cap-running" data-testid="cap-running">
                {n()} running
              </span>
            )}
          </Show>
        </button>
      </div>
    </Show>
  );
}
