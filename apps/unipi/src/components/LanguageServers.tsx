/**
 * Settings › Editor › Language servers (WP-A21): the `lsp_servers`
 * roster of the selected host — which servers are configured, whether
 * their binary is on the host's PATH, and the install hint when it
 * isn't. Servers come from the host's `config.json` (`lsp.*`), so this
 * list is read-only here; the note says where to edit.
 */
import { createResource, For, Show } from 'solid-js';
import { CircleAlert, CircleCheck, Languages } from 'lucide-solid';
import { LspServersDataSchema, type LspServersData } from '@unipi/protocol';
import { call } from '../lib/ipc';
import { useHosts } from '../hosts/context';
import { LOCAL_HOST } from '../lib/host/client';

export function LanguageServers(props: { hostId?: () => string | undefined }) {
  const hosts = useHosts();
  const hostId = () => props.hostId?.() ?? hosts.selectedId() ?? hosts.hosts()[0]?.id ?? LOCAL_HOST;
  const [data] = createResource(hostId, async (id): Promise<LspServersData | undefined> => {
    const result = await call('lsp_list', { hostId: id, path: '~' }, LspServersDataSchema);
    return result.ok ? result.value : undefined;
  });

  return (
    <div class="lsp-settings" data-testid="lsp-settings">
      <Show
        when={data() !== undefined}
        fallback={
          <p class="text-sm text-text-muted" data-testid="lsp-settings-empty">
            {data.loading ? 'Checking the host…' : 'No language servers on this host (older host, or `lsp.enabled` off).'}
          </p>
        }
      >
        <ul class="lsp-list">
          <For each={data()!.servers}>
            {(server) => (
              <li class="lsp-row" data-testid={`lsp-server-${server.language}`}>
                <span class="lsp-lang" title={server.language}>{server.language}</span>
                <span class="lsp-name" title={`${server.name} — ${server.command}`}>
                  {server.name}
                </span>
                <Show
                  when={server.available}
                  fallback={
                    <span class="lsp-state missing" title={server.install_hint ?? `${server.command} isn't on the host's PATH`}>
                      <CircleAlert size={13} /> {server.install_hint ?? 'not installed'}
                    </span>
                  }
                >
                  <span class="lsp-state ok">
                    <CircleCheck size={13} /> installed
                  </span>
                </Show>
              </li>
            )}
          </For>
        </ul>
        <p class="lsp-note">
          <Languages size={12} /> Servers live in the host's <code>config.json</code> — <code>lsp.servers.&lt;language&gt;.command</code>,{' '}
          <code>lsp.idle_minutes</code>, <code>lsp.enabled</code>.
        </p>
      </Show>
    </div>
  );
}
