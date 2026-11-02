/**
 * Settings › Subagents (WP-A25, G-11): the `subagents` namespace (concurrency,
 * default model / thinking) plus Agent profiles — a Settings sub-page with a
 * typed profile editor (owner5 #17), not the `/agents` chat panel.
 * "All running to background" stays where the running work is — the Session
 * modal's Running summary and Alt+B (A16/A18).
 */
import { Show, createSignal, onCleanup } from 'solid-js';
import { Bot } from 'lucide-solid';
import { CapabilityPage } from './CapabilityPage';
import { AgentProfilesPage } from './AgentProfilesPage';
import { registerCommandOverride } from '../../lib/settings/run';
import { settingsScroller } from '../../lib/settings/memory';
import { useNavigate } from '@solidjs/router';
import { desktopShell } from '../../lib/layout';

// The open sub-page survives leaving Settings for the app session (owner5 #16).
const [view, setView] = createSignal<'main' | 'profiles'>('main');

export function resetSubagentsView(): void {
  setView('main');
}

export function SubagentsPage() {
  let anchor: HTMLDivElement | undefined;
  let listTop = 0;
  const navigate = useNavigate();
  const openProfiles = () => {
    // Phone (§3.9, PH-09): Agent profiles is its own pushed page.
    if (!desktopShell()) return navigate('/settings/subagents/profiles');
    listTop = settingsScroller(anchor)?.scrollTop ?? 0;
    setView('profiles');
    requestAnimationFrame(() => {
      const sc = settingsScroller(anchor);
      if (sc) sc.scrollTop = 0;
    });
  };
  const back = () => {
    setView('main');
    requestAnimationFrame(() => {
      const sc = settingsScroller(anchor);
      if (sc) sc.scrollTop = listTop;
    });
  };
  // The form's own "Manage agents…" action opens the same page.
  onCleanup(registerCommandOverride('agents', openProfiles));
  return (
    <>
      <div ref={anchor} class="contents" />
      <Show
        when={view() === 'profiles'}
        fallback={
          <CapabilityPage
            page="subagents"
            title="Subagents"
            icon={Bot}
            namespace="subagents"
            sectionId="sec-subagents"
            blurb="Agent profiles the main chat can delegate to, and how many run at once."
            openers={{ 'agents.profiles': openProfiles }}
          />
        }
      >
        <AgentProfilesPage onBack={back} />
      </Show>
    </>
  );
}
