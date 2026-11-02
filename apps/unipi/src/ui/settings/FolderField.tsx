/**
 * A `string` field with `format: "path"`: a folder button (the same folder
 * browser as the new-chat workspace picker) instead of a bare text box, the
 * chosen path under it, and "Use default" to clear it.
 */
import { Show, createMemo, createResource } from 'solid-js';
import { WorkspacePicker, homeOf } from '../../components/draft/WorkspacePicker';
import { fsRoots } from '../../lib/fs';
import { LOCAL_HOST } from '../../lib/host';
import type { Field, Json } from './types';

export function FolderField(props: {
  field: Field;
  hostId?: string;
  value: string;
  disabled: boolean;
  set: (value: Json | undefined) => void;
}) {
  const hostId = () => props.hostId ?? LOCAL_HOST;
  const [roots] = createResource(hostId, async (id) => {
    const res = await fsRoots(id).catch(() => undefined);
    return res?.ok ? res.value.roots : [];
  });
  const home = createMemo(() => homeOf(roots.latest ?? []));
  return (
    <div class="settings-folder" data-testid={`set-${props.field.key}`}>
      <WorkspacePicker
        hostId={hostId()}
        value={props.value || undefined}
        onChange={(path) => path && props.set(path)}
        recent={[]}
        roots={roots.latest ?? []}
        home={home()}
        allowNone={false}
        placeholder="Choose folder…"
        disabled={props.disabled}
      />
      <Show when={props.value}>
        <button type="button" class="settings-link" disabled={props.disabled} onClick={() => props.set(undefined)} data-testid={`folder-clear-${props.field.key}`}>
          Use default
        </button>
      </Show>
    </div>
  );
}
