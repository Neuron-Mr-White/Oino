/**
 * Settings › Subagents (WP-A25, G-11): the `subagents` namespace (concurrency,
 * default model / thinking) plus the profile manager panel (`/agents`).
 * "All running to background" stays where the running work is — the Session
 * modal's Running summary and Alt+B (A16/A18).
 */
import { Bot } from 'lucide-solid';
import { CapabilityPage } from './CapabilityPage';

export function SubagentsPage() {
  return (
    <CapabilityPage
      page="subagents"
      title="Subagents"
      icon={Bot}
      namespace="subagents"
      sectionId="sec-subagents"
      blurb="Agent profiles the main chat can delegate to, and how many run at once."
    />
  );
}
