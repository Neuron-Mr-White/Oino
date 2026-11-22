/**
 * The `settings` slot's app column (ui-protocol §6.2, WP-U11): one Settings
 * section per OSS-2 namespace — the core/pi namespaces (`oino.*`, group
 * "Agent") and each extension's own namespaces (group "Extensions", ordered
 * `order` desc then title A→Z, §6.4). Every section is the form engine
 * (`ui/settings/form.tsx`), so defaults, scope layering, `visibleIf` and
 * validation behave the same for core, host and extensions.
 */
import { For, Show } from 'solid-js';
import type { Namespace } from '../settings/types';
import { isCoreNamespace, sortNamespaces } from '../settings/types';
import { NamespaceForm } from '../settings/form';

/** Core namespaces (`oino.*`): pi settings, one form each. */
export function coreNamespaces(namespaces: readonly Namespace[]): Namespace[] {
  return sortNamespaces(namespaces.filter(isCoreNamespace));
}

/** Extension-owned namespaces (everything that is not core). */
export function extensionNamespaces(namespaces: readonly Namespace[]): Namespace[] {
  return sortNamespaces(namespaces.filter((ns) => !isCoreNamespace(ns)));
}

/**
 * Renders a namespace list as Settings sections. The caller decides which
 * namespaces exist (core, one extension's, or every enabled extension's).
 */
export function SettingsSections(props: {
  namespaces: readonly Namespace[];
  hostId?: string;
  onChanged?: () => void;
  /** Rows without their own card (inside an existing Settings card). */
  flat?: boolean;
  /** Shown when there is nothing to render (a host without namespaces). */
  empty?: string;
  emptyTestId?: string;
}) {
  return (
    <Show
      when={props.namespaces.length > 0}
      fallback={
        <Show when={props.empty}>
          <p class="px-4 py-3 text-xs text-text-dim" data-testid={props.emptyTestId}>{props.empty}</p>
        </Show>
      }
    >
      <For each={sortNamespaces([...props.namespaces])}>
        {(namespace) => (
          <NamespaceForm
            namespace={namespace}
            hostId={props.hostId}
            onChanged={props.onChanged}
            flat={props.flat}
          />
        )}
      </For>
    </Show>
  );
}
