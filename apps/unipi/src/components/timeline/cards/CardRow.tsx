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
import { describeEvent, eventAudience, type EventAudience } from '../../../lib/timeline/eventKinds';
import { useSettingsOptional } from '../../../settings/context';

/**
 * [owner-fix-8] The chat filter (Settings › chat, the chat ⋯ menu): which
 * audiences' event rows the timeline shows. Agent-only rows are hidden by
 * default; user/assistant messages are never filtered — only the audience
 * tagged event rows.
 */
export function audienceShown(audience: EventAudience, chat: { showAudienceUser: boolean; showAudienceBoth: boolean; showAudienceAgent: boolean } | undefined): boolean {
  if (!chat) return true;
  if (audience === 'user') return chat.showAudienceUser;
  if (audience === 'agent') return chat.showAudienceAgent;
  return chat.showAudienceBoth;
}

export function CardRow(props: { entry: SessionEntry; ctx: TimelineCtx }) {
  const e = () => props.entry;
  const settings = useSettingsOptional();
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
  // [owner-fix-8] the chat filter: an event row of a hidden audience is not
  // drawn. Rows without an audience badge (extension renderers) are kept.
  const hidden = () => {
    const v = view();
    if (!v) return false;
    return !audienceShown(v.audience ?? eventAudience(e()), settings?.settings.chat);
  };
  // Z-42: a subagent / sidekick card has one way into the child transcript —
  // the `subagent` tab (a sheet on the phone). No second "Open session" path.
  const agent = () => agentOfEntry(e());
  const openAgent = () => {
    const a = agent();
    if (!a) return;
    openSubagentTab({ hostId: props.ctx.hostId, sessionId: props.ctx.sessionId(), cwd: props.ctx.cwd() }, a);
  };
  return (
    <Show when={view() !== null && !hidden()}>
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
