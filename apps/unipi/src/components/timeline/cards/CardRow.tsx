/**
 * Card dispatcher (WP-A8, [chat-events]): one place for the entries that
 * render as cards. Native light rows: model / thinking-level changes
 * (`ModelChangeCard`) and the workspace-move marker. Every other event kind
 * `describeEvent` knows — extension messages and entries, user `!bash`
 * runs, branch summaries, mid-chat system messages — is an `EventCard`
 * (own icon + one-line summary, expands to the full body). Unknown `custom`
 * state entries render only through their extension's renderer
 * (`RendererCard`, G-21).
 */
import { Show } from 'solid-js';
import { CornerDownRight, FolderInput } from 'lucide-solid';
import type { SessionEntry } from '../../../lib/session/types';
import { cwdChangeTarget, isCwdChange } from '../../../lib/session/turns';
import { shortCwd } from '../../../lib/session/selectors';
import type { TimelineCtx } from '../context';
import { agentOfEntry } from '../../panes/tabs/agentEntry';
import { openSubagentTab } from '../../panes/tabs/sessionTabs';
import { ModelChangeCard } from './ModelChangeCard';
import { RendererCard } from './RendererCard';
import { EventCard } from './EventCard';
import { describeEvent } from '../../../lib/timeline/eventKinds';

export function CardRow(props: { entry: SessionEntry; ctx: TimelineCtx }) {
  const e = () => props.entry;
  if (e().type === 'model_change' || e().type === 'thinking_level_change') {
    return <ModelChangeCard entry={e()} />;
  }
  // "Move to a workspace…": a small marker row (the reminder text is for the model only).
  if (isCwdChange(e())) {
    const to = () => cwdChangeTarget(e());
    return (
      <div class="tl-card" data-kind="cwd-change" data-testid="tl-cwd-change" title={to()}>
        <span class="tool-icon">
          <FolderInput size={15} />
        </span>
        <span class="tl-card-text truncate">Moved to workspace {to() ? shortCwd(to()!) : ''}</span>
      </div>
    );
  }
  const view = () => describeEvent(e());
  // Z-42: a subagent / sidekick card has one way into the child transcript —
  // the `subagent` tab (a sheet on the phone). No second "Open session" path.
  const agent = () => agentOfEntry(e());
  const openAgent = () => {
    const a = agent();
    if (!a) return;
    openSubagentTab({ hostId: props.ctx.hostId, sessionId: props.ctx.sessionId(), cwd: props.ctx.cwd() }, a);
  };
  return (
    <Show when={view() !== null}>
      <div class="tl-cardblock">
        <Show when={view()} keyed fallback={<RendererCard entry={e()} ctx={props.ctx} />}>
          {(v) => <EventCard entry={e()} view={v} ctx={props.ctx} />}
        </Show>
        <Show when={agent()}>
          <button type="button" class="tl-card-transcript" onClick={openAgent} data-testid="tl-card-transcript">
            <CornerDownRight size={13} />
            <span>Transcript</span>
          </button>
        </Show>
      </div>
    </Show>
  );
}
