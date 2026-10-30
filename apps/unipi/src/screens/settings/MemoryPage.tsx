/**
 * Settings › Memory (WP-A25, G-9): the `memory` namespace's form (recall,
 * write pass, palace paths) plus the extension's **browse panel** — list,
 * search, forget and consolidate, global vs project — opened by `/memory` in
 * the active chat (the panel is built by the extension with the oino-ui
 * builder, `extensions/unipi/memory/src/panel.rs`).
 */
import { Brain } from 'lucide-solid';
import { CapabilityPage } from './CapabilityPage';

export function MemoryPage() {
  return (
    <CapabilityPage
      page="memory"
      title="Memory"
      icon={Brain}
      namespace="memory"
      sectionId="sec-memory"
      blurb="What the agent remembers between chats, and how it searches it."
    />
  );
}
