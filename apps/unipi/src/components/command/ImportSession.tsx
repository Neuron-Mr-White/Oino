/**
 * "Import session…" (WP-A25, G-15): pi's `/import` — resume a session from a
 * JSONL file. The app asks for the path and sends `_oino/import_jsonl` on the
 * shown chat (core switches that session's log to the imported file and
 * answers `{cancelled}`), then the chat keeps going from there.
 *
 * The dialog is mounted once from the shell (`BuiltinActions`) and opened by
 * the composer's `/import` route or the command center, so there is exactly
 * one import surface (DIRECTION §4.2.2).
 */
import { createResource, createSignal, Show } from 'solid-js';
import { z } from 'zod';
import { BoardSheet } from '../board/BoardSheet';
import { fsRoots, fsList } from '../../lib/fs';
import { sessions } from '../../lib/session/store';
import { showActionNotice } from '../../lib/timeline/actions';
import { manageChat } from '../../lib/settings/manage';
import { LOCAL_HOST } from '../../lib/host/client';
import { resolveHostId } from '../../lib/host/localId';
import { refreshThisComputer, thisComputer } from '../../lib/thisComputer';

const [open, setOpen] = createSignal(false);

/** The open flag (`/import`, the palette) and its closer. */
export const importSessionOpen = open;
export function openImportSession(): boolean {
  setOpen(true);
  return true;
}
export function closeImportSession(): void {
  setOpen(false);
}

/** `_oino/import_jsonl`'s reply: the import may be cancelled by a turn in flight. */
export const ImportReplySchema = z.object({ cancelled: z.boolean().optional() }).passthrough();

/** Absolute path or nothing: the host resolves `~`? No — the user picks a real path. */
export function importPathProblem(raw: string): string | undefined {
  const path = raw.trim();
  if (!path) return 'Give the session file’s path';
  if (!/\.jsonl?$/i.test(path)) return 'A session file ends in .jsonl';
  if (!path.startsWith('/') && !/^[A-Za-z]:[\\/]/.test(path)) return 'Use the full path on the host';
  return undefined;
}

export function ImportSessionDialog() {
  // The chat the import lands in: whichever the Manage/built-in route picked.
  const target = () => manageChat();
  const hostId = () => target()?.hostId ?? IMPORT_HOST;
  const [path, setPath] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | undefined>();
  const problem = () => importPathProblem(path());

  // The host's own sessions folder is where the file usually is: offer it (and the home folder) as chips.
  // Only while the dialog is open — it is mounted at startup and must not list folders then — and the
  // folder comes from the host (its data dir sits in its agent dir), never a guessed ~/.oino.
  const [roots] = createResource(
    () => (open() ? hostId() : undefined),
    (id) => fsRoots(id).catch(() => undefined),
  );
  const [agentDir] = createResource(
    () => (open() ? hostId() : undefined),
    async (id) => {
      if (id !== LOCAL_HOST && id !== resolveHostId(LOCAL_HOST)) return undefined;
      if (thisComputer().kind !== 'ready') await refreshThisComputer();
      const state = thisComputer();
      return state.kind === 'ready' ? state.info.data_dir.replace(/[\\/]+host[\\/]*$/, '') : undefined;
    },
  );
  const firstRoot = (): string | undefined => {
    const r = roots.latest;
    return r && r.ok ? r.value.roots[0] : undefined;
  };
  const [files] = createResource(
    () => (open() ? { hostId: hostId(), root: firstRoot(), agent: agentDir.latest } : undefined),
    async (src) => {
      if (!src.root) return [];
      const root = src.root;
      const dirs = src.agent ? [root, `${src.agent}/sessions`] : [root];
      const rows: string[] = [];
      for (const dir of dirs) {
        const listing = await fsList(src.hostId, dir).catch(() => undefined);
        if (!listing?.ok) continue;
        for (const entry of listing.value.entries) if (entry.kind === 'file' && entry.name.endsWith('.jsonl')) rows.push(`${dir}/${entry.name}`);
      }
      return rows.slice(0, 8);
    },
  );

  const submit = async () => {
    const chat = target();
    if (problem() || busy() || !chat) return;
    const handle = sessions().peek(chat.hostId, chat.sessionId);
    if (!handle) {
      setError('That chat is closed — open it again');
      return;
    }
    setBusy(true);
    setError(undefined);
    try {
      const value = await handle.cmd({ type: '_oino/import_jsonl', path: path().trim() });
      const parsed = ImportReplySchema.safeParse(value);
      if (parsed.success && parsed.data.cancelled) {
        setError('The running turn kept the session — stop it and import again');
        return;
      }
      closeImportSession();
      setPath('');
      showActionNotice('Session imported');
    } catch (e) {
      setError(String((e as { message?: unknown })?.message ?? e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <BoardSheet open={open()} onClose={closeImportSession} label="Import session" title="Import session" testId="import-session">
      <div class="import-session" data-testid="import-session-body">
        <div class="import-session-field">
          <label class="import-session-label" for="import-session-path">
            Session file
          </label>
          <input
            id="import-session-path"
            class="import-session-input"
            type="text"
            spellcheck={false}
            placeholder="/home/you/.oino/sessions/2026-10-31T12-00-00.jsonl"
            value={path()}
            onInput={(e) => setPath(e.currentTarget.value)}
            data-testid="import-session-path"
          />
          <Show when={path().trim() && problem()}>
            <span class="settings-field-hint" data-testid="import-session-hint">
              {problem()}
            </span>
          </Show>
        </div>
        <Show when={(files.latest ?? []).length > 0}>
          <div class="import-session-recent">
            <p class="import-session-label">Recent session files</p>
            <ul>
              {(files.latest ?? []).map((file) => (
                <li>
                  <button type="button" class="settings-link" onClick={() => setPath(file)} data-testid="import-session-recent">
                    {file}
                  </button>
                </li>
              ))}
            </ul>
          </div>
        </Show>
        <Show when={!target()}>
          <p class="settings-field-hint" data-testid="import-session-noch">
            Open a chat first — the import resumes into the session you have open.
          </p>
        </Show>
        <Show when={error()}>
          <p class="settings-field-hint settings-field-hint-bad" role="alert" data-testid="import-session-error">
            {error()}
          </p>
        </Show>
        <div class="import-session-actions">
          <button type="button" class="btn btn-secondary" onClick={closeImportSession}>
            Cancel
          </button>
          <button type="button" class="btn btn-primary" disabled={!!problem() || busy() || !target()} onClick={() => void submit()} data-testid="import-session-confirm">
            {busy() ? 'Importing…' : 'Import'}
          </button>
        </div>
      </div>
    </BoardSheet>
  );
}

/** The default host (`This computer`) when no chat is shown: import still needs a chat. */
export const IMPORT_HOST = LOCAL_HOST;
