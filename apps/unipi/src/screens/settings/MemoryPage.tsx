/**
 * Settings › Memory (WP-A25, G-9): the `memory` namespace's form (recall,
 * write pass, palace paths) plus Browse memories / All projects — the app's
 * own paged modal over the host's `memory_browse` (components/memory). The
 * extension's `/memory` panel (forget, consolidate) stays a chat command.
 */
import { Brain } from 'lucide-solid';
import { CapabilityPage } from './CapabilityPage';
import { openMemoryBrowserFor } from '../../components/memory/MemoryBrowser';
import { DreamPanel } from '../../components/memory/DreamPanel';
import '../../components/memory/memory.css';
import { useHostsOptional } from '../../hosts/context';
import { settingsHostId } from '../../settings/target';

export function MemoryPage() {
  const hosts = useHostsOptional();
  // The same host the page's form and rows act on (This computer first).
  const hostId = () => hosts?.hosts().find((h) => h.transport === 'local')?.id ?? settingsHostId();
  return (
    <CapabilityPage
      page="memory"
      title="Memory"
      icon={Brain}
      namespace="memory"
      sectionId="sec-memory"
      blurb="What the agent remembers between chats, and how it searches it."
      // Owner fix 2026-11-03: the app's own paged browser, no chat needed.
      extraManage={
        <>
          <div class="settings-in-card-label">Dream</div>
          <DreamPanel hostId={hostId()} />
        </>
      }
      openers={{
        'memory.browse': () => openMemoryBrowserFor('project', hostId()),
        'memory.global': () => openMemoryBrowserFor('all', hostId()),
      }}
    />
  );
}
