/**
 * The browser beside a chat shows the profile that chat's agent uses (not the PC's one
 * remembered profile): the agent's `browser_*` tools may run on `ace-jobs` while the PC's
 * default is `default`, and opening `default` would launch a browser the agent never touches.
 *
 * Where the chat's agent profile comes from:
 * - the PC: `browser_status{session_id}` → `session_profile` (read when the pane opens and
 *   again after each browser tool call; older PCs answer without it);
 * - the timeline: every `browser_*` tool result carries `details.browser.profile`, so a switch
 *   shows at once, before the PC answers.
 *
 * Without a chat (the Browser screen, a pane tab with no session) or when neither source
 * knows a profile, the PC's remembered choice (`browserPane().profileFor`) stays in charge.
 */
import { createEffect, createMemo, createSignal, onCleanup } from 'solid-js';
import { browserStatus } from './browser';
import { browserPane, effectiveProfile } from './browserPane';
import { sessions } from './session/store';
import type { LiveTool, SessionEntry } from './session/types';

/** How far back the timeline is scanned for a browser tool result. */
const SCAN = 300;
/** The PC gets this long to name the chat's profile before the pane opens on the PC's own choice. */
const ASK_MS = 4000;

type Seen = { id: string; profile: string };

function profileOf(details: unknown): string | undefined {
  const browser = (details as { browser?: { profile?: unknown } } | undefined)?.browser;
  return typeof browser?.profile === 'string' && browser.profile ? browser.profile : undefined;
}

/** The newest browser tool result of a chat (finished live tool first, else the timeline): its id + profile. */
export function latestBrowserResult(entries: readonly SessionEntry[], liveTools: Readonly<Record<string, LiveTool>>): Seen | undefined {
  let live: LiveTool | undefined;
  for (const tool of Object.values(liveTools)) {
    if (!tool.done || !/^browser_/.test(tool.name) || !profileOf((tool.result as { details?: unknown } | undefined)?.details)) continue;
    if (!live || tool.startedAt >= live.startedAt) live = tool;
  }
  if (live) return { id: live.id, profile: profileOf((live.result as { details?: unknown }).details)! };
  for (let i = entries.length - 1; i >= Math.max(0, entries.length - SCAN); i -= 1) {
    const m = entries[i]!.message;
    if (m?.role !== 'toolResult' || !/^browser_/.test(m.toolName ?? '')) continue;
    const profile = profileOf(m.details);
    if (profile) return { id: m.toolCallId ?? entries[i]!.id, profile };
  }
  return undefined;
}

export type ChatBrowserProfile = {
  /** The profile the pane shows. */
  profile: () => string;
  /** The profile this chat's agent uses (undefined = unknown: an older PC, or not asked yet). */
  agentProfile: () => string | undefined;
  /** Open the viewer? False until the PC has said which profile the chat uses (never launch `default` first). */
  ready: () => boolean;
  /** The profile menu picked another one (viewing only; the agent keeps its profile). */
  choose: (name: string) => void;
};

export function chatBrowserProfile(hostId: () => string, sessionId: () => string | undefined): ChatBrowserProfile {
  const pane = browserPane();
  const [agent, setAgent] = createSignal<string | undefined>();
  const [asked, setAsked] = createSignal(false);

  const seen = createMemo<Seen | undefined>(() => {
    const sid = sessionId();
    const state = sid ? sessions().peek(hostId(), sid)?.state : undefined;
    return state ? latestBrowserResult(state.entries, state.liveTools) : undefined;
  }, undefined, { equals: (a, b) => a?.id === b?.id && a?.profile === b?.profile });

  let last = '';
  let ticket = 0;
  createEffect(() => {
    const host = hostId();
    const sid = sessionId();
    const result = seen();
    const key = `${host}\u0000${sid ?? ''}`;
    if (key !== last) {
      last = key;
      setAgent(undefined);
      setAsked(false);
    }
    if (!sid) return;
    // A tool result shows its switch at once; the PC's answer (below) settles it.
    if (result) setAgent(result.profile);
    const mine = ++ticket;
    const timer = setTimeout(() => mine === ticket && setAsked(true), ASK_MS);
    onCleanup(() => clearTimeout(timer));
    void browserStatus(host, sid)
      .then((r) => {
        if (mine !== ticket) return;
        if (r.ok && r.value.session_profile) setAgent(r.value.session_profile);
      })
      .catch(() => {})
      .finally(() => {
        if (mine === ticket) setAsked(true);
      });
  });

  return {
    profile: () => {
      const sid = sessionId();
      const perHost = pane.profileFor(hostId());
      return sid ? effectiveProfile(pane.chatProfileFor(hostId(), sid), agent(), perHost) : perHost;
    },
    agentProfile: agent,
    ready: () => {
      const sid = sessionId();
      return !sid || asked() || agent() !== undefined || !!pane.chatProfileFor(hostId(), sid);
    },
    choose: (name) => {
      const sid = sessionId();
      const known = agent();
      // No agent profile known (an older PC): today's per-PC choice.
      if (sid && known !== undefined) pane.setChatProfile(hostId(), sid, { profile: name, seen: known });
      else pane.setProfileFor(hostId(), name);
    },
  };
}
