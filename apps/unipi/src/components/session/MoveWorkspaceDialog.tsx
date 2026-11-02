/**
 * "Move to a workspace…" (UniPi Oino): a No-workspace chat runs in its own private
 * scratch folder; this dialog moves it to a real folder — one way only. The
 * picker is the draft's (recent workspaces + Open folder…); the text says the
 * move cannot be undone; "Copy the scratch files" is off by default and never
 * overwrites. The host appends one reminder to the conversation (the earlier
 * messages and the system prompt stay as they were) and the sidebar regroups
 * the chat under the new workspace.
 *
 * Desktop: centred modal; phone: the standard bottom sheet.
 */
import { createMemo, createResource, createSignal, onCleanup, onMount, Show } from 'solid-js';
import { X } from 'lucide-solid';
import { BoardSheet } from '../board/BoardSheet';
import { WorkspacePicker, homeOf } from '../draft/WorkspacePicker';
import { layout } from '../../lib/layout';
import { wrapTab } from '../../lib/focusTrap';
import { pushBack } from '../../lib/back';
import { fsRoots } from '../../lib/fs';
import { hubs } from '../../lib/session/hub';
import { folderChoices, folderName } from '../../lib/session/newSession';
import { moveToWorkspace } from '../../lib/session/actions';
import { showActionNotice } from '../../lib/timeline/actions';
import '../draft/draft.css';
import './session-modal.css';

export type MoveTarget = { hostId: string; sessionId: string; cwd?: string };

const [target, setTarget] = createSignal<MoveTarget>();

/** What the dialog is moving (`undefined` = closed). */
export const moveWorkspaceDialog = target;
export function openMoveWorkspaceDialog(next: MoveTarget): void {
  setTarget(next);
}
export function closeMoveWorkspaceDialog(): void {
  setTarget(undefined);
}

export function MoveWorkspaceDialog() {
  return (
    <Show when={target()} keyed>
      {(t) => <MoveWorkspace target={t} />}
    </Show>
  );
}

function MoveWorkspace(props: { target: MoveTarget }) {
  const [folder, setFolder] = createSignal<string | undefined>();
  const [copy, setCopy] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | undefined>();
  const close = () => !busy() && closeMoveWorkspaceDialog();
  const [roots] = createResource(
    () => props.target.hostId,
    async (id) => {
      const res = await fsRoots(id);
      return res.ok ? res.value.roots : [];
    },
  );
  const recent = createMemo(() => {
    const hub = hubs().peek(props.target.hostId);
    const cards = hub?.state.loaded ? Object.values(hub.state.cards) : [];
    return folderChoices([...cards].sort((a, b) => (b.updated_at ?? 0) - (a.updated_at ?? 0)), [], undefined);
  });
  const move = async () => {
    const cwd = folder();
    if (!cwd || busy()) return;
    setBusy(true);
    setError(undefined);
    try {
      const done = await moveToWorkspace(props.target.hostId, props.target.sessionId, cwd, { copyFiles: copy() });
      setBusy(false);
      closeMoveWorkspaceDialog();
      const copied = done.copied ? ` · ${done.copied} file${done.copied === 1 ? '' : 's'} copied` : '';
      showActionNotice(`Moved to workspace “${folderName(done.cwd)}”${copied}`);
    } catch (e) {
      setBusy(false);
      setError(String((e as { message?: unknown } | null)?.message ?? e));
    }
  };
  const body = () => (
    <div class="move-ws-form" data-testid="move-ws-form">
      <p class="move-ws-text">
        This chat has no workspace — it runs in a private scratch folder. Pick a folder to work in from the next message on.
      </p>
      <div class="move-ws-picker">
        <WorkspacePicker
          hostId={props.target.hostId}
          value={folder()}
          onChange={(p) => {
            setFolder(p);
            setError(undefined);
          }}
          recent={recent()}
          roots={roots.latest ?? []}
          home={homeOf(roots.latest ?? [])}
          allowNone={false}
          placeholder="Choose a folder…"
          disabled={busy()}
        />
      </div>
      <p class="move-ws-warn" data-testid="move-ws-warning">
        This is one way: the chat can’t go back to its scratch folder or move again. Earlier messages stay as they are; the agent is
        told the folder changed.
      </p>
      <label class="move-ws-copy">
        <input type="checkbox" checked={copy()} disabled={busy()} onChange={(e) => setCopy(e.currentTarget.checked)} data-testid="move-ws-copy" />
        <span>Copy the scratch folder’s files into the workspace (existing files are kept)</span>
      </label>
      <Show when={error()}>
        <p class="move-ws-error" role="alert" data-testid="move-ws-error">
          {error()}
        </p>
      </Show>
      <div class="rename-actions">
        <button type="button" class="btn ghost" onClick={close} data-testid="move-ws-cancel">
          Cancel
        </button>
        <button type="button" class="btn" disabled={!folder() || busy()} onClick={() => void move()} data-testid="move-ws-confirm">
          {busy() ? 'Moving…' : 'Move'}
        </button>
      </div>
    </div>
  );
  return (
    <Show
      when={layout().desktop()}
      fallback={
        <BoardSheet open onClose={close} label="Move to a workspace" title="Move to a workspace" testId="move-ws-sheet">
          {body()}
        </BoardSheet>
      }
    >
      <BackClose onClose={close} />
      <div class="sm-overlay" role="presentation" onClick={close} data-testid="move-ws-overlay">
        <div
          class="sm-dialog move-ws-dialog"
          role="dialog"
          aria-modal="true"
          aria-label="Move to a workspace"
          tabindex="-1"
          ref={(el) => queueMicrotask(() => el.focus({ preventScroll: true }))}
          onKeyDown={(e: KeyboardEvent) => {
            if (wrapTab(e, e.currentTarget as HTMLElement)) e.stopPropagation();
            if (e.key === 'Escape') close();
          }}
          onClick={(e) => e.stopPropagation()}
          data-testid="move-ws-dialog"
        >
          <div class="sm-head">
            <span class="sm-title">
              <h2>Move to a workspace</h2>
            </span>
            <button type="button" class="ghost-icon-button" aria-label="Close" onClick={close} data-testid="move-ws-close">
              <X size={20} />
            </button>
          </div>
          {body()}
        </div>
      </div>
    </Show>
  );
}

/** Android Back closes the dialog while it is open (not the app under it). */
function BackClose(props: { onClose: () => void }) {
  onMount(() => onCleanup(pushBack(props.onClose)));
  return null;
}
