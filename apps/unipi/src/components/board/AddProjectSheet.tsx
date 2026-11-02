/**
 * "Add project" (D-27): register a folder on the PC as a Board project from
 * the UI — a small folder browser over `fs_list` (the app has no native folder
 * dialog) and "Add this folder". The host re-checks the folder is one it lets
 * the app see.
 */
import { createEffect, createResource, createSignal, For, Show } from 'solid-js';
import { ArrowLeft, Folder, FolderPlus, Home } from 'lucide-solid';
import { fsList, fsRoots } from '../../lib/fs';
import { api, describe } from '../../lib/board/api';
import { parentOf, homeOf } from '../draft/WorkspacePicker';
import { shortPath } from '../../lib/session/newSession';
import { BoardSheet } from './BoardSheet';

export function AddProjectSheet(props: { open: boolean; onClose: () => void; hostId?: string; onAdded: (hostId: string) => void }) {
  const [dir, setDir] = createSignal<string | undefined>();
  const [home, setHome] = createSignal<string | undefined>();
  const [roots, setRoots] = createSignal<string[]>([]);
  const [typed, setTyped] = createSignal('');
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
      setRoots(roots);
      setDir(homeOf(roots) ?? roots[0] ?? '/');
    });
  });

  const [listing] = createResource(
    () => (props.open && props.hostId && dir() ? { hostId: props.hostId, path: dir()! } : undefined),
    async ({ hostId, path }) => {
      const res = await fsList(hostId, path);
      if (!res.ok) {
        const scope = /outside the allowed|allowed folders|E_SCOPE|scope/i.test(String(res.error.message));
        return { error: scope ? 'Unipi can’t open this folder on the PC. Go back to a folder it lists.' : String(res.error.message), dirs: [] as string[] };
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
  const norm = (p: string) => (p.length > 1 ? p.replace(/\/+$/, '') : p);
  /** E-12: the folders the app may open — a root and anything under it. */
  const allowed = (p: string) => roots().length === 0 || roots().some((r) => norm(p) === norm(r) || norm(p).startsWith(`${norm(r)}/`));
  const canUp = () => !!dir() && dir() !== '/' && allowed(parentOf(dir()!));
  const go = (p: string) => {
    const v = p.trim();
    if (v) setDir(norm(v));
  };
  createEffect(() => setTyped(dir() ?? ''));
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
        <button type="button" class="ghost-icon-button" aria-label="Up one folder" disabled={!canUp()} onClick={() => setDir(parentOf(dir()!))}>
          <ArrowLeft size={18} />
        </button>
        <input
          class="add-project-field"
          type="text"
          spellcheck={false}
          aria-label="Folder path"
          placeholder="/path/to/repo"
          value={typed()}
          title={dir()}
          onInput={(e) => setTyped(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              go(typed());
            }
          }}
          data-testid="add-project-path"
        />
      </div>
      <div class="add-project-quick" role="group" aria-label="Shortcuts">
        <Show when={home()}>
          <button type="button" class="add-project-chip" onClick={() => setDir(home())}>
            <Home size={14} /> Home
          </button>
        </Show>
        <For each={roots().filter((r) => r !== home()).slice(0, 4)}>
          {(r) => (
            <button type="button" class="add-project-chip" title={r} onClick={() => setDir(r)}>
              <Folder size={14} /> {shortPath(r, home())}
            </button>
          )}
        </For>
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
