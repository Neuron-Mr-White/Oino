/**
 * MCP servers (owner 2026-11-05: "MCP settings have issues, check one by one"): a native
 * table of every configured server — status, source, transport, tool count and the FULL
 * error (wrapping, never cut off at an ellipsis) — with per-server Enable / Disable and
 * Reconnect, plus Reload all and Sync catalog. Data: `unipi.mcp`'s `mcp.servers` /
 * `mcp.server-action` services in Settings' own command session.
 *
 * The two per-server controls match the extension's API (`mcp.server-action`):
 * the switch is enable / disable (it persists the server's `enabled` and starts or
 * stops it) and Reconnect is a button that restarts a running server. Both say what
 * they did — a toast with the server's new status, or its error.
 */
import { createResource, createSignal, For, Show } from 'solid-js';
import { CircleAlert, Plug, RefreshCw } from 'lucide-solid';
import { settingsCommandChat } from '../../lib/settings/run';
import { toasts } from '../../lib/toasts';
import { SwitchField } from '../Switch';
import './mcp.css';

export type McpServer = { name: string; source: string; enabled: boolean; transport: string; status: string; error?: string; toolCount: number; tools: string[] };
export type McpServersData = { servers: McpServer[]; errors: string[]; totalTools: number };

const obj = (v: unknown): Record<string, unknown> => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : {});

/** `mcp.servers`, leniently. Pure. */
export function parseMcpServers(raw: unknown): McpServersData {
  const o = obj(raw);
  const servers = (Array.isArray(o.servers) ? o.servers : []).map(obj).filter((s) => typeof s.name === 'string').map((s) => ({
    name: String(s.name),
    source: String(s.source ?? 'global'),
    enabled: s.enabled !== false,
    transport: String(s.transport ?? 'stdio'),
    status: String(s.status ?? 'stopped'),
    ...(typeof s.error === 'string' && s.error ? { error: s.error } : {}),
    toolCount: typeof s.toolCount === 'number' ? s.toolCount : 0,
    tools: Array.isArray(s.tools) ? s.tools.filter((t): t is string => typeof t === 'string') : [],
  }));
  return { servers, errors: Array.isArray(o.errors) ? o.errors.filter((e): e is string => typeof e === 'string') : [], totalTools: typeof o.totalTools === 'number' ? o.totalTools : 0 };
}

export function McpServers(props: { hostId: string }) {
  const call = async (name: string, payload?: Record<string, unknown>) => {
    const handle = await settingsCommandChat(props.hostId);
    if (!handle) throw new Error('Couldn’t reach the agent on this PC — try again in a moment.');
    return handle.cmd<unknown>({ type: '_oino/service_call', name, ...(payload ? { payload } : {}) });
  };
  const [data, { refetch, mutate }] = createResource(async () => parseMcpServers(await call('mcp.servers')));
  const [busy, setBusy] = createSignal<string | undefined>();
  const [open, setOpen] = createSignal<string | undefined>();
  /** Runs one server action and reports the outcome (the row's new status). */
  const act = async (key: string, action: string, name?: string) => {
    setBusy(key);
    try {
      const r = obj(await call('mcp.server-action', { action, ...(name ? { name } : {}) }));
      if (r.ok === false) toasts().push(String(r.message ?? 'That did not work'), 'error');
      else {
        const next = r.servers ? parseMcpServers(r.servers) : undefined;
        if (next) mutate(next);
        else await refetch();
        const after = next?.servers.find((s) => s.name === name);
        if (after) toasts().push(after.error ? `${after.name}: ${after.error}` : `${after.name} — ${after.status}`, after.error ? 'error' : 'success');
        else if (!name) toasts().push(action === 'reload' ? 'Reloaded the servers' : 'Synced the catalog', 'success');
      }
    } catch (e) {
      toasts().push(String((e as { message?: unknown })?.message ?? e), 'error');
    } finally {
      setBusy(undefined);
    }
  };
  return (
    <div class="mcps" data-testid="mcp-servers">
      <div class="mcps-bar">
        <span class="mcps-title">Servers</span>
        <Show when={data.latest}>{(d) => <span class="mcps-dim">{d().servers.length} configured · {d().totalTools} tools</span>}</Show>
        <span class="flex-1" />
        <button type="button" class="btn-secondary settings-btn" disabled={!!busy()} onClick={() => void act('sync', 'sync')} data-testid="mcp-sync">Sync catalog</button>
        <button type="button" class="btn-secondary settings-btn" disabled={!!busy()} onClick={() => void act('reload', 'reload')} data-testid="mcp-reload">
          <RefreshCw size={13} class={busy() === 'reload' ? 'refresh-spinning' : undefined} /> Reload all
        </button>
      </div>
      <Show when={!data.error} fallback={<p class="mcps-note" role="alert">{String((data.error as { message?: unknown })?.message ?? 'MCP is not available on this PC.')}</p>}>
        <Show when={data.latest} fallback={<p class="mcps-note">Loading…</p>}>
          {(d) => (
            <>
              <Show when={d().errors.length}>
                <div class="mcps-config-err" role="alert">
                  <CircleAlert size={14} />
                  <div>
                    <For each={d().errors}>{(e) => <p>{e}</p>}</For>
                  </div>
                </div>
              </Show>
              <Show when={d().servers.length} fallback={<p class="mcps-note">No MCP servers yet. Add one from the catalog below.</p>}>
                <ul class="mcps-list">
                  <For each={d().servers}>
                    {(s) => (
                      <li class="mcps-row" data-status={s.status} data-testid={`mcp-row-${s.name}`}>
                        <div class="mcps-main">
                          <button type="button" class="mcps-name" aria-expanded={open() === s.name} onClick={() => setOpen(open() === s.name ? undefined : s.name)}>
                            <Plug size={14} />
                            <span>{s.name}</span>
                          </button>
                          <span class="mcps-status" data-status={s.status}>{s.status}</span>
                          <span class="mcps-dim">{s.source} · {s.transport} · {s.toolCount} tools</span>
                        </div>
                        <div class="mcps-actions">
                          <button
                            type="button"
                            class="btn-secondary settings-btn mcps-reconnect"
                            disabled={!!busy() || !s.enabled}
                            title={s.enabled ? `Restart ${s.name}` : `${s.name} is off — turn it on first`}
                            data-testid={`mcp-reconnect-${s.name}`}
                            onClick={() => void act(`rc-${s.name}`, 'reconnect', s.name)}
                          >
                            <RefreshCw size={13} class={busy() === `rc-${s.name}` ? 'refresh-spinning' : undefined} />
                            Reconnect
                          </button>
                          <SwitchField
                            label={`${s.name} ${s.enabled ? 'enabled' : 'disabled'}`}
                            checked={s.enabled}
                            disabled={!!busy() || s.source.startsWith('project') || s.source === 'acp'}
                            onChange={(v) => void act(`en-${s.name}`, v ? 'enable' : 'disable', s.name)}
                            testId={`mcp-en-${s.name}`}
                          />
                        </div>
                        <Show when={s.error}>
                          <p class="mcps-err" data-testid={`mcp-err-${s.name}`}><CircleAlert size={13} /> <span>{s.error}</span></p>
                        </Show>
                        <Show when={open() === s.name && s.tools.length}>
                          <div class="mcps-tools"><For each={s.tools}>{(t) => <code>{t}</code>}</For></div>
                        </Show>
                      </li>
                    )}
                  </For>
                </ul>
              </Show>
            </>
          )}
        </Show>
      </Show>
    </div>
  );
}
