/**
 * Session side tabs (WP-A10, app spec §3.8): the four session-scoped pane
 * kinds — `diff`, `tree`, `btw`, `subagent` — and the one entry module every
 * opener goes through, so desktop (a side-pane tab) and phone (a §3.9 sheet
 * or full-screen layer) stay one UI each.
 *
 *   openDiffTab(base, {toolId, path}, line?)          ← "View diff" (Z-40)
 *   openTreeTab(base)                                 ← `+` launcher (Z-18)
 *   openBtwTab(base, question?)                       ← `/btw`, `+` Side question (Z-31)
 *   openSubagentTab(base, {agentId, name})            ← step rows / completion cards (Z-42)
 *
 * Tabs are opened in the shown workspace's scope so they persist per project
 * and are tagged with the session (hidden while another session shows, dropped
 * with the session — `lib/panes.ts` `dropSessionTabs`).
 */
import { createSignal } from 'solid-js';
import { desktopShell, layout } from '../../../lib/layout';
import { panes, type PaneScope, type PaneView } from '../../../lib/panes';
import { openSideTab } from '../SidePane';
import type { LaunchSpec } from '../PaneLauncher';
import { currentWorkspace } from '../../shell/Workspace';

/** The session a tab belongs to (`cwd` seeds the project scope when no workspace is shown). */
export type SessionTabBase = { hostId: string; sessionId: string; cwd?: string };

/** Which of the phone's session-tab surfaces is open (§3.9: sheets for tree /
 * side chat / subagent; the diff viewer is a full-screen layer). */
export type PhoneSessionTab = {
  kind: 'diff' | 'tree' | 'btw' | 'subagent';
  base: SessionTabBase;
  params: Record<string, unknown>;
  title: string;
};

/**
 * The `hostId`/`sessionId` a persisted tab carries (its params are the truth).
 */
export function tabBaseOf(params: Record<string, unknown> | undefined, fallback: { hostId: string; sessionId?: string }): SessionTabBase {
  return {
    hostId: typeof params?.hostId === 'string' ? params.hostId : fallback.hostId,
    sessionId: typeof params?.sessionId === 'string' ? params.sessionId : (fallback.sessionId ?? ''),
  };
}

const [phoneTab, setPhoneTab] = createSignal<PhoneSessionTab | undefined>();
export const phoneSessionTab = phoneTab;
export function closePhoneSessionTab(): void {
  setPhoneTab(undefined);
}

/** The phone's surface for a session tab (also used by a scenario to seed one). */
export function seedPhoneSessionTab(tab: PhoneSessionTab | undefined): void {
  setPhoneTab(tab);
}

/**
 * Opens a session tab: the side pane on the desktop, the phone surface on the
 * phone.
 */
export function openSessionTab(base: SessionTabBase, spec: LaunchSpec): void {
  if (desktopShell()) {
    const ws = currentWorkspace();
    const scope: PaneScope = ws?.scope ?? { hostId: base.hostId, projectKey: base.cwd || '~' };
    const view: PaneView = ws?.view ?? { sessionId: base.sessionId };
    openSideTab(scope, view, spec);
    return;
  }
  // Phone: no side pane — the tab's view opens as its §3.9 surface.
  layout().toggleSide(false);
  setPhoneTab({ kind: spec.kind as PhoneSessionTab['kind'], base, params: spec.params ?? {}, title: spec.title ?? spec.kind });
}

/** Z-40 "View diff": the full diff of one edit/write tool call, at `line`. */
export function openDiffTab(
  base: SessionTabBase,
  source: { toolId?: string; entryId?: string; part?: number; path: string },
  line?: number,
  opts?: { split?: boolean },
): void {
  const path = source.path;
  openSessionTab(base, {
    kind: 'diff',
    params: {
      hostId: base.hostId,
      sessionId: base.sessionId,
      ...(source.toolId ? { toolId: source.toolId } : {}),
      ...(source.entryId ? { entryId: source.entryId } : {}),
      ...(source.part !== undefined ? { part: source.part } : {}),
      path,
      ...(line ? { line } : {}),
      ...(opts?.split ? { split: true } : {}),
    },
    title: `Diff · ${path.split('/').filter(Boolean).pop() ?? path}`,
  });
}

/** Z-18: the session tree of this conversation. */
export function openTreeTab(base: SessionTabBase): void {
  openSessionTab(base, { kind: 'tree', params: { hostId: base.hostId, sessionId: base.sessionId }, title: 'Tree' });
}

/** Z-31: a side chat that never touches the main session (`question` seeds the first ask). */
export function openBtwTab(base: SessionTabBase, question?: string): void {
  openSessionTab(base, {
    kind: 'btw',
    params: { hostId: base.hostId, sessionId: base.sessionId, ...(question ? { question } : {}) },
    title: 'Side chat',
  });
}

/** Z-42: one nested agent's transcript. */
export function openSubagentTab(base: SessionTabBase, agent: { agentId: string; name?: string }): void {
  openSessionTab(base, {
    kind: 'subagent',
    params: { hostId: base.hostId, sessionId: base.sessionId, agentId: agent.agentId, ...(agent.name ? { name: agent.name } : {}) },
    title: agent.name ?? 'Subagent',
  });
}
