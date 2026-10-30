/**
 * Agent section (WP-U11, G-5): the pi agent's own settings, rendered by the
 * OSS-2 form engine from the core `oino.*` namespaces (`oino.model`,
 * `oino.compaction`, `oino.retry`, …) over `host_cmd get_settings_schema` /
 * `get_extension_settings` without a session. Core maps each namespace back to
 * its `settings.json` key, so the form edits `~/.oino/settings.json` (global)
 * and `<project>/.oino/settings.json` (project) through one write path.
 *
 * A host without core namespaces (v1) falls back to the read-only key list.
 */
import { For, Show, createResource, createSignal } from 'solid-js';
import type { AppError } from '@unipi/protocol';
import { Waypoints } from 'lucide-solid';
import { piSettings } from '../../settings/hostCmd';
import { settingsSchema } from '../../ui/settings/data';
import { LOCAL_HOST } from '../../lib/host/client';
import { Row, Section } from './rows';
import { ErrorBanner } from '../../components/ErrorBanner';
import { SettingsSections, coreNamespaces } from '../../ui/slots/SettingsSections';
import type { Namespace } from '../../ui/settings/types';

function describe(value: unknown): string {
  if (value === null) return 'null';
  if (Array.isArray(value)) return `list (${value.length})`;
  if (typeof value === 'object') {
    const keys = Object.keys(value as Record<string, unknown>);
    return keys.length ? `${keys.slice(0, 4).join(', ')}${keys.length > 4 ? '…' : ''}` : 'empty';
  }
  return String(value);
}

export function AgentSection() {
  const hostId = () => LOCAL_HOST;
  const [error, setError] = createSignal<AppError | undefined>();
  const [schema] = createResource(async (): Promise<Namespace[]> => {
    const result = await settingsSchema(hostId());
    if (!result.ok) return [];
    setError(undefined);
    return result.value;
  });
  const core = () => coreNamespaces(schema.latest ?? []);

  return (
    <Section title="Agent" id="sec-agent" icon={<Waypoints size={14} />}>
      <Show when={error()} keyed>
        {(err) => (
          <div class="p-3">
            <ErrorBanner error={err} dismiss={() => setError(undefined)} />
          </div>
        )}
      </Show>
      <Show
        when={core().length > 0}
        fallback={<PiKeyList hostId={hostId()} />}
      >
        <div class="px-4 pt-4">
          <p class="text-xs text-text-dim" data-testid="agent-form-note">
            The pi agent's settings on this PC, edited per scope: Global writes ~/.oino/settings.json, Project writes the folder's .oino/settings.json.
          </p>
        </div>
        <div class="settings-form-list">
          <SettingsSections namespaces={core()} flat empty="No agent settings on this PC yet" />
        </div>
      </Show>
    </Section>
  );
}

/** v1 hosts: the merged pi settings object, read-only (A24 behaviour). */
function PiKeyList(props: { hostId: string }) {
  const [keys] = createResource(async (): Promise<Array<[string, unknown]>> => {
    const result = await piSettings(props.hostId);
    if (!result.ok) return [];
    return Object.entries(result.value).filter(([key]) => key !== 'extensionSettings');
  });
  return (
    <>
      <Show when={!keys.loading && keys.latest?.length === 0}>
        <Row label="No agent settings" description="This PC has no ~/.oino/settings.json yet — a session creates the defaults" />
      </Show>
      <For each={keys.latest ?? []}>
        {([key, value]) => (
          <Row label={key}>
            <span class="text-xs text-text-dim font-mono" id={`pi-${key}`}>{describe(value)}</span>
          </Row>
        )}
      </For>
      <p class="px-4 py-3 text-xs text-text-dim" data-testid="agent-form-note">
        Values from settings.json on this PC, merged with the project file. This host reports no settings schema, so the keys stay read-only.
      </p>
    </>
  );
}
