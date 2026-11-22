/**
 * [zvec] Semantic search index per project, from the sidebar: whether a folder has an index
 * (the orange folder icon) and "Build search index…" (asks for consent first; never silent).
 */
import { createSignal } from 'solid-js';
import { z } from 'zod';
import { hostCall } from '../host/factory';
import { showActionNotice } from '../timeline/actions';
import { confirmSheet } from '../../components/phone/Sheet';

const [indexed, setIndexed] = createSignal<Readonly<Record<string, boolean>>>({});
const requested = new Set<string>();
const key = (hostId: string, cwd: string) => `${hostId}|${cwd}`;
/** Index states that mean "no usable index on disk". */
const NO_INDEX = new Set(['none', 'missing', 'uninitialized', 'disabled', 'building']);

function mark(hostId: string, cwd: string, on: boolean): void {
  const k = key(hostId, cwd);
  setIndexed((prev) => (prev[k] === on ? prev : { ...prev, [k]: on }));
}

async function refresh(hostId: string, cwd: string): Promise<void> {
  const reply = await hostCall(hostId, 'host_cmd', { cmd: { kind: 'search_index_status', cwd } }, z.object({ state: z.string() }));
  mark(hostId, cwd, reply.ok && !NO_INDEX.has(reply.value.state));
}

/** True once the project is known to have a search index (asks the host once per folder). */
export function searchIndexed(hostId: string, cwd: string): boolean {
  const k = key(hostId, cwd);
  if (!requested.has(k)) {
    requested.add(k);
    void refresh(hostId, cwd);
  }
  return indexed()[k] === true;
}

/** Sidebar "Build search index…": explicit consent, then the host builds it. */
export async function buildSearchIndex(hostId: string, cwd: string, label: string): Promise<void> {
  const again = indexed()[key(hostId, cwd)] === true;
  const ok = await confirmSheet({
    title: `${again ? 'Rebuild' : 'Build'} search index for ${label}?`,
    body: 'Downloads the local CPU model (potion-code-16m-v2, about 31 MB) the first time, and creates .zvec-grep/ in this folder. The index stays on this PC. New chats here can then search by meaning.',
    confirm: again ? 'Rebuild index' : 'Build index',
  });
  if (!ok) return;
  showActionNotice(`Building search index for ${label}…`);
  const reply = await hostCall(hostId, 'host_cmd', { cmd: { kind: 'build_search_index', cwd, consent: true } }, z.object({ success: z.boolean() }));
  if (reply.ok) {
    mark(hostId, cwd, true);
    showActionNotice(`Search index ready for ${label}. New chats there can use semantic search.`);
  } else {
    showActionNotice(`Couldn't build the search index: ${reply.error.message}`);
  }
}
