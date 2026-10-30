/**
 * Agent section (WP-A24): the pi agent's own settings (`~/.oino/settings.json`
 * + the project file), read over `host_cmd get_settings` without a session.
 * The schema-driven editing form is U11's (it lands inside this section);
 * here the keys are listed and searchable, with values shown read-only.
 */
import { For, Show, createResource, createSignal } from 'solid-js';
import type { AppError } from '@unipi/protocol';
import { Waypoints } from 'lucide-solid';
import { piSettings } from '../../settings/hostCmd';
import { LOCAL_HOST } from '../../lib/host/client';
import { Row, Section } from './rows';
import { ErrorBanner } from '../../components/ErrorBanner';

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
  const [keys] = createResource(async (): Promise<Array<[string, unknown]>> => {
    const result = await piSettings(hostId());
    if (!result.ok) {
      setError(result.error);
      return [];
    }
    setError(undefined);
    return Object.entries(result.value).filter(([key]) => key !== 'extensionSettings');
  });

  return (
    <Section title="Agent" id="sec-agent" icon={<Waypoints size={14} />}>
      <Show when={error()}>
        <div class="p-3">
          <Show when={error()} keyed>
            {(err) => <ErrorBanner error={err} dismiss={() => setError(undefined)} />}
          </Show>
        </div>
      </Show>
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
        Values from settings.json on this PC, merged with the project file. Editing arrives with the settings form.
      </p>
    </Section>
  );
}
