/**
 * "Add project" (D-27): register a folder on the PC as a Board project from the UI. It is the
 * same folder picker as the workspace chip (FolderPicker: search by name with zoxide, type a
 * path, browse), with "Add this folder" as its action. The host re-checks the folder is one it
 * lets the app see, and a refusal shows in the picker.
 */
import { createEffect, createSignal, Show } from 'solid-js';
import { api, describe } from '../../lib/board/api';
import { isOwnPcFile } from '../../lib/openLocal';
import { FolderPicker } from '../draft/FolderPicker';

export function AddProjectSheet(props: { open: boolean; onClose: () => void; hostId?: string; onAdded: (hostId: string) => void }) {
  const [busy, setBusy] = createSignal(false);
  const [problem, setProblem] = createSignal<string | undefined>();
  createEffect(() => props.open && setProblem(undefined));

  async function add(path: string) {
    const hostId = props.hostId;
    if (!hostId || busy()) return;
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
    <Show when={props.open && props.hostId}>
      {(hostId) => (
        <FolderPicker
          hostId={hostId()}
          recentItems={[]}
          allowNone={false}
          ownPc={isOwnPcFile(hostId())}
          title="Add a project"
          actionLabel="Add"
          error={problem()}
          busy={busy()}
          onClose={props.onClose}
          onUse={(path) => void add(path)}
        />
      )}
    </Show>
  );
}
