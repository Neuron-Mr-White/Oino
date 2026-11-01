/**
 * "Add project" (D-27): register a folder on the PC as a Board project from
 * the UI — a small folder browser over `fs_list` (the app has no native folder
 * dialog) and "Add this folder". The host re-checks the folder is one it lets
 * the app see.
 */
import { createEffect, createResource, createSignal, For, Show } from 'solid-js';
import { ArrowLeft, Folder, FolderPlus } from 'lucide-solid';
import { fsList, fsRoots } from '../../lib/fs';
import { api, describe } from '../../lib/board/api';
import { parentOf, homeOf } from '../draft/WorkspacePicker';
import { shortPath } from '../../lib/session/newSession';
import { BoardSheet } from './BoardSheet';

export function AddProjectSheet(props: { open: boolean; onClose: () => void; hostId?: string; onAdded: (hostId: string) => void }) {
  const [dir, setDir] = createSignal<string | undefined>();
  const [home, setHome] = createSignal<string | undefined>();
  const [busy, setBusy] = createSignal(false);
  const [problem, setProblem] = createSignal<string | undefined>();

  // Start in the PC's home folder each time the sheet opens.
  createEffect(() => {
    if (!props.open || !props.hostId) return;
    const hostId = props.hostId;
    setProblem(undefined);
    setDir(undefined);
    void fsRoots(hostId).then((res) => {
      if (!props.open) return;
      const roots = res.ok ? res.value.roots : [];
      setHome(homeOf(roots));
      setDir(homeOf(roots) ?? roots[0] ?? '/');
    });
  });

  const [listing] = createResource(
    () => (props.open && props.hostId && dir() ? { hostId: props.hostId, path: dir()! } : undefined),
    async ({ hostId, path }) => {
      const res = await fsList(hostId, path);
      if (!res.ok) {
        const scope = /outside the allowed|allowed folders|E_SCOPE|scope/i.test(String(res.error.message));
        return { error: scope ? 'UniPi can’t open this folder on the PC. Go back to a folder it lists.' : String(res.error.message), dirs: [] as string[] };
      }
      return {
        error: undefined as string | undefined,
        dirs: res.value.entries
          .filter((e) => e.kind === 'dir' && !e.name.startsWith('.'))
          .map((e) => e.name)
          .sort((a, b) => a.localeCompare(b)),
      };
    },
  );
  const join = (base: string, name: string) => `${base.replace(/\/+$/, '')}/${name}`;

  async function add() {
    const hostId = props.hostId;
    const path = dir();
    if (!hostId || !path || busy()) return;
    setBusy(true);
    setProblem(undefined);
    try {
      await api.addProject(hostId, path);
      props.onAdded(hostId);
      props.onClose();
    } catch (error) {
      setProblem(describe(error) || 'Couldn’t add this folder.');
    } finally {
      setBusy(false);
    }
  }

  return (
    <BoardSheet
      open={props.open}
      onClose={props.onClose}
      label="Add a project"
      testId="add-project-sheet"
      class="board-sheet-tall"
      footer={
        <button type="button" class="btn btn-primary w-full" disabled={busy() || !dir() || listing.loading || !!listing()?.error} onClick={() => void add()} data-testid="add-project-use">
          <FolderPlus size={16} /> {busy() ? 'Adding…' : 'Add this folder'}
        </button>
      }
    >
      <p class="add-project-hint">Pick the folder of a repo on this PC. Tasks for it live on the Board.</p>
      <div class="add-project-head">
        <button type="button" class="ghost-icon-button" aria-label="Up one folder" disabled={!dir() || dir() === '/'} onClick={() => setDir(parentOf(dir()!))}>
          <ArrowLeft size={18} />
        </button>
        <span class="add-project-path" title={dir()}>
          {dir() ? shortPath(dir()!, home()) : '…'}
        </span>
      </div>
      <Show when={problem()}>{(text) => <p class="add-project-error" role="alert">{text()}</p>}</Show>
      <div class="add-project-list" role="list">
        <Show when={dir() && !listing.loading} fallback={<p class="add-project-note">Loading…</p>}>
          <Show when={!listing()?.error} fallback={<p class="add-project-note">{listing()?.error}</p>}>
            <Show when={listing()?.dirs.length} fallback={<p class="add-project-note">No folders inside</p>}>
              <For each={listing()?.dirs ?? []}>
                {(name) => (
                  <button type="button" role="listitem" class="picker-row" onClick={() => setDir(join(dir()!, name))}>
                    <Folder size={18} class="shrink-0 text-text-dim" />
                    <span class="min-w-0 flex-1 truncate text-left text-[15px]">{name}</span>
                  </button>
                )}
              </For>
            </Show>
          </Show>
        </Show>
      </div>
    </BoardSheet>
  );
}
