/**
 * The phone ⋯ menu's "Panes" group (§3.9, WP-A29): on the phone there is no
 * side pane or bottom panel — what the desktop offers as pane tabs opens as
 * full-screen layers / bottom sheets here. This group is their launcher:
 *
 *   - the native `+`-launcher kinds, routed to their phone surfaces
 *     (Files → the `/files` layer, Git → the status-pane layer, Tree and
 *     Side chat → the session-tab sheets);
 *   - the session's U10 `sidePane` / `bottomPane` extension entries
 *     (`PhonePanesGroup`, full-screen `PhonePaneLayer`).
 *
 * A kind lists itself once it has a view (`launchableKinds`) *and* a phone
 * opener in `PHONE_OPENERS` — wave-3 kinds (terminal, browser) join by
 * registering one. `diff` / `subagent` / `preview` are not launchable: they
 * open from their context (a tool call, an agent, a file), same as desktop.
 */
import { For, Show } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { MenuLabel, MenuRow, MenuSeparator } from '../board/AnchoredMenu';
import { launchableKinds } from '../../lib/panes/kinds';
import { setFilesTarget } from '../../lib/filesTarget';
import { openStatusSection } from '../status/StatusPane';
import { openBtwTab, openTreeTab } from '../panes/tabs/sessionTabs';
import { PhonePanesGroup } from '../../ui/slots/paneTabs';
import type { UiRegistry } from '../../ui/registry';

type OpenCtx = { hostId: string; sessionId?: string; cwd?: string; navigate: (to: string) => void };

/** kind → the §3.9 surface its tab opens as on the phone. */
const PHONE_OPENERS: Record<string, (ctx: OpenCtx) => void> = {
  files: (c) => {
    if (c.cwd) setFilesTarget({ hostId: c.hostId, path: c.cwd });
    c.navigate('/files');
  },
  git: (c) => openStatusSection({ hostId: c.hostId, sessionId: c.sessionId, path: c.cwd || '~' }),
  tree: (c) => {
    if (c.sessionId) openTreeTab({ hostId: c.hostId, sessionId: c.sessionId, cwd: c.cwd });
  },
  btw: (c) => {
    if (c.sessionId) openBtwTab({ hostId: c.hostId, sessionId: c.sessionId, cwd: c.cwd });
  },
};

/** Registers a phone surface for a launchable kind (wave-3: terminal, browser). */
export function registerPhonePaneOpener(kind: string, open: (ctx: OpenCtx) => void): void {
  PHONE_OPENERS[kind] = open;
}

export function PanesMenu(props: { hostId: string; sessionId?: string; cwd?: string; registry?: UiRegistry; onOpen: () => void }) {
  const navigate = useNavigate();
  const ctx = () => ({ hostId: props.hostId, sessionId: props.sessionId, cwd: props.cwd, navigate });
  const kinds = () =>
    [...launchableKinds('side', !!props.sessionId), ...launchableKinds('bottom', !!props.sessionId)].filter((k) => PHONE_OPENERS[k.kind]);
  return (
    <>
      <Show when={kinds().length > 0}>
        <MenuSeparator />
        <MenuLabel>Panes</MenuLabel>
        <For each={kinds()}>
          {(kind) => {
            const Icon = kind.icon;
            return (
              <MenuRow
                icon={<Icon size={17} />}
                label={kind.launcher!.label}
                onSelect={() => {
                  props.onOpen();
                  PHONE_OPENERS[kind.kind]?.(ctx());
                }}
              />
            );
          }}
        </For>
      </Show>
      <Show when={props.registry} keyed>
        {(registry) => <PhonePanesGroup registry={registry} onOpen={props.onOpen} />}
      </Show>
    </>
  );
}
