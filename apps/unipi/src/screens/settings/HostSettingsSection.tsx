/**
 * Host config section (WP-A24): This computer's host settings (`config.json`)
 * over `host_settings_get/set` — relay, port forwarding, push, file-access
 * scope, terminals, updates. Access-sensitive writes are local-only; values
 * apply at once (read fresh per call).
 */
import { Show, createResource, createSignal } from 'solid-js';
import { Cpu, RefreshCw } from 'lucide-solid';
import type { AppError } from '@unipi/protocol';
import type { HostSettings } from '@unipi/protocol';
import { hostSettingsGet, hostSettingsSet } from '../../settings/hostCmd';
import { Row, Section, Segmented, Stepper } from './rows';
import { SwitchField } from '../../components/Switch';
import { ErrorBanner } from '../../components/ErrorBanner';

export function HostSettingsSection() {
  const [error, setError] = createSignal<AppError | undefined>();
  const [busy, setBusy] = createSignal<string | undefined>();
  const [relayUrl, setRelayUrl] = createSignal('');
  const [ports, setPorts] = createSignal('');
  const [fsAllow, setFsAllow] = createSignal('');
  const [fsDeny, setFsDeny] = createSignal('');

  const [settings, { refetch }] = createResource(async (): Promise<HostSettings | undefined> => {
    const result = await hostSettingsGet();
    if (!result.ok) {
      setError(result.error);
      return undefined;
    }
    setError(undefined);
    const value = result.value;
    setRelayUrl(typeof value.relay === 'string' && value.relay !== 'n0' ? value.relay : '');
    setPorts(Array.isArray(value.forwarding?.ports) ? value.forwarding!.ports!.join(', ') : '');
    setFsAllow((value.fs?.allow ?? []).join('\n'));
    setFsDeny((value.fs?.deny ?? []).join('\n'));
    return value;
  });

  const patch = async (key: string, value: unknown) => {
    setBusy(key);
    const result = await hostSettingsSet(key, value);
    setBusy(undefined);
    if (!result.ok) {
      setError(result.error);
      return;
    }
    setError(undefined);
    await refetch();
  };

  const relayMode = () => (settings.latest?.relay && settings.latest.relay !== 'n0' ? 'custom' : 'n0');

  return (
    <Section title="Host config" id="sec-host" icon={<Cpu size={14} />}>
      <Show when={error()}>
        <div class="p-3">
          <Show when={error()} keyed>
            {(err) => <ErrorBanner error={err} dismiss={() => setError(undefined)} />}
          </Show>
        </div>
      </Show>
      <Show when={settings.latest} keyed>
        {(host) => (
          <>
            <Row label="Host relay" description="The relay this PC's host advertises for pairing (its own setting; the app's relay is under Connection)">
              <Segmented
                label="Host relay"
                value={relayMode()}
                options={['n0', 'custom'] as const}
                labels={{ n0: 'n0 (default)', custom: 'Custom' }}
                onChange={(mode) => void patch('relay', mode === 'n0' ? 'n0' : relayUrl() || 'https://')}
              />
            </Row>
            <Show when={relayMode() === 'custom'}>
              <div class="px-4 py-3">
                <label class="text-xs text-text-dim block mb-1" for="host-relay-url">Relay URL</label>
                <input
                  id="host-relay-url"
                  type="url"
                  class="w-full rounded-input bg-surface-2 border border-line px-3 py-2 text-sm text-text"
                  value={relayUrl()}
                  onInput={(e) => setRelayUrl(e.currentTarget.value)}
                  onBlur={() => relayUrl().startsWith('https://') && void patch('relay', relayUrl())}
                  placeholder="https://relay.example.com"
                />
              </div>
            </Show>
            <Row label="Relay only" description="Refuse direct connections; everything goes through the relay">
              <SwitchField label="Relay only" checked={host.relay_only === true} disabled={busy() === 'relay_only'} onChange={(v) => void patch('relay_only', v)} />
            </Row>
            <Row label="Port forwarding" description="Let paired devices reach listening TCP ports on this PC">
              <SwitchField
                label="Port forwarding"
                checked={host.forwarding?.enabled === true}
                disabled={busy() === 'forwarding.enabled'}
                onChange={(v) => void patch('forwarding.enabled', v)}
              />
            </Row>
            <Show when={host.forwarding?.enabled === true}>
              <div class="px-4 py-3">
                <label class="text-xs text-text-dim block mb-1" for="host-forward-ports">Forwarded ports</label>
                <input
                  id="host-forward-ports"
                  type="text"
                  class="w-full rounded-input bg-surface-2 border border-line px-3 py-2 text-sm text-text"
                  value={ports()}
                  onInput={(e) => setPorts(e.currentTarget.value)}
                  onBlur={() => {
                    const list = ports().split(',').map((p) => Number(p.trim())).filter((p) => Number.isInteger(p) && p > 0 && p < 65536);
                    void patch('forwarding.ports', list.length ? list : 'any');
                  }}
                  placeholder="any, or 5173, 3000"
                />
              </div>
            </Show>
            <Row label="Push to phones" description="Offer ntfy push delivery to paired devices">
              <SwitchField
                label="Push to phones"
                checked={host.push?.enabled === true}
                disabled={busy() === 'push.enabled'}
                onChange={(v) => void patch('push.enabled', v)}
              />
            </Row>
            <Row label="File access allow list" description="Folders outside $HOME the host may touch (one per line; empty = home only)">
              <textarea
                class="w-full min-h-20 rounded-input bg-surface-2 border border-line px-3 py-2 text-sm text-text font-mono"
                aria-label="File access allow list"
                value={fsAllow()}
                onInput={(e) => setFsAllow(e.currentTarget.value)}
                onBlur={() => void patch('fs.allow', fsAllow().split('\n').map((l) => l.trim()).filter(Boolean))}
              />
            </Row>
            <Row label="File access deny list" description="Always refused, even inside $HOME (one per line)">
              <textarea
                class="w-full min-h-20 rounded-input bg-surface-2 border border-line px-3 py-2 text-sm text-text font-mono"
                aria-label="File access deny list"
                value={fsDeny()}
                onInput={(e) => setFsDeny(e.currentTarget.value)}
                onBlur={() => void patch('fs.deny', fsDeny().split('\n').map((l) => l.trim()).filter(Boolean))}
              />
            </Row>
            <Row label="Max terminals" description="Host-owned PTYs at once">
              <Stepper
                label="Max terminals"
                value={host.term?.max ?? 16}
                min={1}
                max={128}
                step={1}
                onChange={(v) => void patch('term.max', v)}
              />
            </Row>
            <Row label="Host updates" description="Prompt before installing a host release, or install and restart automatically">
              <Segmented
                label="Host updates"
                value={host.update?.mode ?? 'prompt'}
                options={['prompt', 'auto'] as const}
                labels={{ prompt: 'Prompt', auto: 'Automatic' }}
                onChange={(mode) => void patch('update.mode', mode)}
              />
            </Row>
          </>
        )}
      </Show>
      <div class="px-4 py-3 flex justify-end">
        <button type="button" class="rounded-pill bg-surface-2 text-text-muted px-3 py-2 text-xs flex items-center gap-1.5" onClick={() => void refetch()}>
          <RefreshCw size={13} /> Refresh
        </button>
      </div>
    </Section>
  );
}
