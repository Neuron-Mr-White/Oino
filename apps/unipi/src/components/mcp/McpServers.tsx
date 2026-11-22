/**
 * Settings › MCP (OIN-101 WI-3, SPEC §2; earlier: owner 2026-11-05 full errors + per-server actions).
 *
 *  - Scope picker `Global | Project: <name> ▾` — the known projects of the PC Settings is on. Entering from the
 *    sidebar's "Project settings" or from `/mcp` in a chat preselects that chat's project (`projectScope`,
 *    `settings/target.ts`); picking one here writes the same scope, so the other forms follow.
 *  - Header: the resolved file paths of the scope and (project scope) the trust state with its one control.
 *  - Warning rows for parse / skip reasons (file + line), never a silent empty list.
 *  - The server list — rows on the desktop, cards on the phone (CSS only): name · scope badge · source file ·
 *    transport · status · actions (enable, Reconnect, Edit, Remove; a read-only row from `.pi` / `.unipi` /
 *    `.mcp.json` gets [Override in Oino]). Errors show in full.
 *  - [Add server] → `McpSheet`.
 *
 * An older host (no `files` / `trust` in the answer, or no `mcp.save`) keeps working read-only for what it
 * knows: the page says what is missing instead of breaking (`state.modern`).
 */
import { createEffect, createMemo, createResource, createSignal, For, onCleanup, Show } from 'solid-js';
import { CircleAlert, FileText, FolderOpen, Pencil, Plug, Plus, RefreshCw, ShieldAlert, ShieldCheck, Trash2 } from 'lucide-solid';
import { hubs } from '../../lib/session/hub';
import { projectsOf } from '../../lib/session/selectors';
import { resolveHostId } from '../../lib/host/localId';
import { projectScope, setProjectScope, settingsProjectCwd } from '../../settings/target';
import { toasts } from '../../lib/toasts';
import { desktopShell } from '../../lib/layout';
import { hostClient } from '../../lib/host/factory';
import { SwitchField } from '../Switch';
import { confirmSheet } from '../phone/Sheet';
import { mcpAutonomyContext, mcpCatalog, projectChat, getTrust, loadMcp, mcpAction, removeMcp, saveMcp, setMcpEnabled, setTrust, Unsupported, type SaveReply } from './api';
import { McpSheet, type SheetMode } from './McpSheet';
import {
  BADGE_LABEL, autonomyHint, parseMcpState, projectLabel, serverStatus, shortFile,
  type McpScope, type McpServer, type McpState, type ProjectTrust,
} from './model';
import './mcp.css';

/** Back-compat name: the data reader the first MCP page tests import. */
export const parseMcpServers = parseMcpState;
export type { McpServer };

/** `/p/x/.oino/mcp.json` read in `/p/x` → `.oino/mcp.json`; anything else in short form. */
const relFile = (path: string, root?: string): string => (root && path.startsWith(`${root.replace(/[\\/]+$/, '')}/`) ? path.slice(root.replace(/[\\/]+$/, '').length + 1) : shortFile(path));

const msg = (e: unknown): string => String((e as { message?: unknown })?.message ?? e);

export function McpServers(props: { hostId: string }) {
  const cwd = () => settingsProjectCwd(props.hostId);
  const scope = (): McpScope => (cwd() ? 'project' : 'global');
  const at = () => ({ hostId: props.hostId, ...(cwd() ? { cwd: cwd() } : {}) });

  const [defaultAutonomy] = createResource(() => props.hostId, async (host) => {
    const reply = await hostClient(host).request<{ value?: unknown }>('host_cmd', { cmd: { kind: 'get_settings', key: 'autonomy.defaultLevel' } });
    return typeof reply.value === 'string' ? reply.value : 'medium';
  });
  /** N2/N5/N6: the chat whose autonomy applies (by title), the Settings default with no chat, nothing on Global. */
  const autonomyContext = () => mcpAutonomyContext(props.hostId, cwd(), defaultAutonomy() ?? '…');
  const chat = () => projectChat(props.hostId, cwd());
  const [rev, setRev] = createSignal(0);
  const [data, { mutate }] = createResource(
    () => ({ host: props.hostId, cwd: cwd(), rev: rev() }),
    (src) => loadMcp({ hostId: src.host, ...(src.cwd ? { cwd: src.cwd } : {}) }),
  );
  const state = (): McpState | undefined => (data.error ? undefined : data.latest);
  const refresh = () => setRev((n) => n + 1);

  const [busy, setBusy] = createSignal<string | undefined>();
  const [open, setOpen] = createSignal<string | undefined>();
  const [sheet, setSheet] = createSignal<SheetMode | undefined>();
  const [flash, setFlash] = createSignal<string | undefined>();

  /* Servers that are still connecting: re-read the status (bounded, chained) so the row and the sheet settle on their own. */
  let polls = 0;
  createEffect(() => {
    const s = state();
    if (!s?.servers.some((x) => serverStatus(x).kind === 'connecting')) {
      polls = 0;
      return;
    }
    if (polls >= 20) return;
    const t = setTimeout(() => {
      polls += 1;
      refresh();
    }, 1500);
    onCleanup(() => clearTimeout(t));
  });

  const projects = createMemo(() => {
    const here = cwd();
    const list = projectsOf(hubs().ensure(resolveHostId(props.hostId)).state.cards).filter((p) => p.cwd);
    const known = list.map((p) => ({ cwd: p.cwd, label: p.label }));
    if (here && !known.some((p) => p.cwd === here)) known.unshift({ cwd: here, label: projectLabel(here) });
    return known;
  });
  const pickScope = (value: string) => setProjectScope(value === 'global' ? undefined : { hostId: projectScope()?.hostId ?? props.hostId, cwd: value });

  const trust = (): ProjectTrust | undefined => state()?.trust;
  const trusted = () => trust()?.state === 'trusted';
  /** Project files can be written: the host understands projects and the folder is trusted. */
  const projectWritable = () => !!cwd() && !!state()?.modern && trusted();
  const projectWhy = () => (!cwd() ? undefined : !state()?.modern ? 'This PC’s UniPi Oino is too old to edit project servers.' : !trusted() ? 'Trust this project first.' : undefined);

  const report = (reply: SaveReply | undefined) => {
    if (reply?.state) mutate(reply.state);
    else refresh();
    if (reply && !reply.ok) toasts().push(reply.message ?? 'That did not work', 'error');
  };

  /** Runs one host call with the row's busy state and an error toast. */
  const run = async (key: string, fn: () => Promise<SaveReply | undefined>, after?: (r: SaveReply | undefined) => void) => {
    setBusy(key);
    try {
      const r = await fn();
      report(r);
      if (!r || r.ok) after?.(r);
    } catch (e) {
      toasts().push(msg(e), 'error');
    } finally {
      setBusy(undefined);
    }
  };

  const say = (name: string, r: SaveReply | undefined) => {
    const row = r?.state?.servers.find((s) => s.name === name);
    if (!row) return;
    const st = serverStatus(row);
    toasts().push(row.error ? `${name}: ${row.error}` : `${name} — ${st.label}`, row.error ? 'error' : 'success');
  };

  /** On a global server in project scope this writes a partial override into the project's `.oino/mcp.json`. */
  const toggle = (s: McpServer, on: boolean) => run(`en-${s.name}`, () => setMcpEnabled(at(), scope(), s.name, on), (r) => say(s.name, r));
  const reconnect = (s: McpServer) => run(`rc-${s.name}`, () => mcpAction(at(), 'reconnect', s.name), (r) => say(s.name, r));
  const reloadAll = () => run('reload', () => mcpAction(at(), 'reload'), () => toasts().push(cwd() ? 'Reloaded this project’s servers' : 'Reloaded the servers in every open project', 'success'));
  const sync = () => run('sync', () => mcpAction(at(), 'sync'), () => toasts().push('Synced the catalog', 'success'));
  const remove = async (s: McpServer) => {
    const ok = await confirmSheet({
      title: `Remove ${s.name}?`,
      body: s.badge === 'override' ? `${s.name} stops overriding the global server of the same name in this project.` : `${s.name} is deleted from ${scope() === 'project' ? 'this project’s' : 'your'} .oino/mcp.json. Stored secrets stay on this PC until you replace them.`,
      confirm: 'Remove',
      tone: 'danger',
    });
    if (ok) await run(`rm-${s.name}`, () => removeMcp(at(), scope(), s.name), () => toasts().push(`Removed ${s.name}`, 'success'));
  };

  const saveFromSheet = async (sc: McpScope, entries: Record<string, Record<string, unknown>>, secrets: Record<string, string>): Promise<SaveReply> => {
    const reply = await saveMcp(at(), sc, entries, secrets);
    // A global save from a project sheet returns the global instance's view.
    // Re-read the selected scope so project rows and authoritative trust survive.
    if (reply.state && sc === scope()) mutate(reply.state);
    refresh();
    setFlash(Object.keys(entries)[0]);
    return reply;
  };

  /* Trust (project scope). */
  const decideTrust = (yes: boolean) =>
    run('trust', async () => {
      const c = cwd()!;
      // Both save on this folder itself: `false` also overrides a trusted parent (SPEC §3 check 8).
      await setTrust(props.hostId, c, c, yes);
      const next = await getTrust(props.hostId, c).catch(() => undefined);
      if (next && state()) mutate({ ...state()!, trust: next });
      refresh();
      return undefined;
    }, () => toasts().push(yes ? 'Trusted — loading this project’s servers' : 'Stopped trusting this project', yes ? 'success' : 'info'));

  const sheetProject = () => (cwd() ? { cwd: cwd()!, writable: projectWritable(), ...(projectWhy() ? { why: projectWhy() } : {}) } : undefined);
  const taken = (sc: McpScope) => (state()?.servers ?? []).filter((s) => (sc === 'global' ? s.badge === 'global' : s.badge !== 'global')).map((s) => s.name);
  const filesOfScope = () => (state()?.files ?? []).filter((f) => (cwd() ? f.scope === 'project' : f.scope === 'global'));
  const defaultFile = () => (cwd() ? `${cwd()!.replace(/[\\/]+$/, '')}/.oino/mcp.json` : '~/.oino/mcp.json');
  const loadError = () => {
    const e = data.error as unknown;
    if (!e) return undefined;
    return e instanceof Unsupported ? e.message : msg(e) || 'MCP is not available on this PC.';
  };
  const projectUnsupported = () => !!cwd() && !!state() && !state()!.modern;

  return (
    <div class="mcps" id="mcp-servers" data-testid="mcp-servers">
      <div class="mcps-bar">
        <label class="mcps-scope">
          <span class="mcps-title">Scope</span>
          <select
            class="settings-input settings-select mcps-scope-select"
            aria-label="Scope"
            value={cwd() ?? 'global'}
            onChange={(e) => pickScope(e.currentTarget.value)}
            data-testid="mcp-scope"
          >
            <option value="global" selected={!cwd()}>Global</option>
            <For each={projects()}>{(p) => <option value={p.cwd} selected={cwd() === p.cwd}>Project: {p.label}</option>}</For>
          </select>
        </label>
        <span class="flex-1" />
        <button type="button" class="btn-secondary settings-btn mcps-btn" disabled={!!busy() || !!loadError()} onClick={() => void reloadAll()} data-testid="mcp-reload" title={cwd() ? 'Re-read this project’s files and reconnect' : 'Re-read ~/.oino/mcp.json and reload every open project'}>
          <RefreshCw size={13} class={busy() === 'reload' ? 'refresh-spinning' : undefined} /> {cwd() ? 'Reload' : 'Reload all'}
        </button>
        <button type="button" class="btn-primary settings-btn mcps-btn" disabled={!!loadError() || projectUnsupported() || (!!cwd() && !projectWritable() && !!state())} onClick={() => setSheet({ kind: 'add' })} data-testid="mcp-add">
          <Plus size={14} /> Add server
        </button>
      </div>

      <div class="mcps-head" data-testid="mcp-head">
        <p class="mcps-files" data-testid="mcp-files" hidden={projectUnsupported() ? true : undefined}>
          <FileText size={13} aria-hidden="true" />
          <span>
            <Show when={cwd()}><span class="mcps-dim" title={cwd()}>{shortFile(cwd()!)}/ </span></Show>
            <Show when={filesOfScope().length} fallback={<code>{defaultFile()}</code>}>
              <For each={filesOfScope()}>
                {(f, i) => (
                  <>
                    <Show when={i() > 0}> · </Show>
                    <code title={f.path} data-exists={f.exists === false ? 'no' : undefined}>{relFile(f.path, cwd())}</code>
                    <Show when={f.exists === false}><span class="mcps-dim"> (none)</span></Show>
                  </>
                )}
              </For>
            </Show>
          </span>
        </p>
        <Show when={cwd() && state()}>
          <TrustControl
            trust={trust()}
            busy={busy() === 'trust'}
            onTrust={() => void decideTrust(true)}
            onRevoke={() => void decideTrust(false)}
          />
        </Show>
      </div>

      <Show when={!loadError()} fallback={<p class="mcps-note" role="alert" data-testid="mcp-error">{loadError()}</p>}>
        <Show when={state()} fallback={<p class="mcps-note">Loading…</p>}>
          {(d) => (
            <>
              <Show when={projectUnsupported()}>
                <p class="mcps-warn" role="status" data-testid="mcp-old-host"><CircleAlert size={14} /> <span>This PC’s UniPi Oino is older than project MCP files, so a project’s servers can’t be shown or edited here. Pick Global to manage the servers it knows, or update it on that PC.</span></p>
              </Show>
              <Show when={d().warnings.length}>
                <div class="mcps-warn" role="alert" data-testid="mcp-warnings">
                  <CircleAlert size={14} />
                  <div>
                    <For each={d().warnings}>
                      {(w) => (
                        <p>
                          <Show when={w.file}><code>{relFile(w.file!, cwd())}{w.line ? `:${w.line}` : ''}</code> </Show>
                          {w.text}
                        </p>
                      )}
                    </For>
                  </div>
                </div>
              </Show>
              <Show when={!projectUnsupported()}>
                <div class="mcps-count">
                  <span class="mcps-dim">{d().servers.length} {d().servers.length === 1 ? 'server' : 'servers'} · {d().totalTools} tools</span>
                  <span class="flex-1" />
                  <button type="button" class="mcps-link" disabled={!!busy()} onClick={() => void sync()} data-testid="mcp-sync" title="Refresh the catalog of installable servers">Sync catalog</button>
                </div>
                <Show when={d().servers.length} fallback={<p class="mcps-note" data-testid="mcp-empty">{cwd() && !trusted() && d().modern ? 'No project servers are loaded while this folder is not trusted.' : 'No MCP servers yet. Add one with Add server, or from the catalog below.'}</p>}>
                  <ul class="mcps-list">
                    <For each={[...d().servers].sort((a, b) => Number(a.badge === 'global') - Number(b.badge === 'global') || a.name.localeCompare(b.name))}>
                      {(s) => {
                        const st = () => serverStatus(s, { errorShownBelow: true });
                        return (
                          <li class="mcps-row" data-status={s.status} data-kind={st().kind} data-flash={flash() === s.name ? '' : undefined} data-testid={`mcp-row-${s.name}`}>
                            <div class="mcps-main">
                              <button type="button" class="mcps-name" aria-expanded={open() === s.name} onClick={() => setOpen(open() === s.name ? undefined : s.name)} data-testid={`mcp-name-${s.name}`}>
                                <Plug size={14} />
                                <span>{s.name}</span>
                              </button>
                              <span class="mcps-badge" data-badge={s.badge} data-testid={`mcp-badge-${s.name}`}>{BADGE_LABEL[s.badge]}</span>
                              <Show when={autonomyContext()}>{(t) => <span class="mcps-dim" data-testid={`mcp-autonomy-${s.name}`}>{t()}</span>}</Show>
                              <span class="mcps-transport">{s.transport === 'http' || s.transport === 'sse' || s.transport === 'streamable-http' ? 'HTTP' : 'Command'}</span>
                              <span class="mcps-status" data-kind={st().kind} data-status={s.status} data-testid={`mcp-status-${s.name}`}>{st().label}</span>
                              <Show when={s.sourceFile}>
                                <span class="mcps-source" title={s.sourceFile}><code>{shortFile(s.sourceFile!)}</code><Show when={s.readOnly}> · read-only</Show></span>
                              </Show>
                            </div>
                            <div class="mcps-actions">
                              <Show when={!s.readOnly}>
                                <SwitchField
                                  label={`${s.name} ${s.enabled ? 'enabled' : 'disabled'}`}
                                  checked={s.enabled}
                                  disabled={!!busy() || s.source === 'acp'}
                                  onChange={(v) => void toggle(s, v)}
                                  testId={`mcp-en-${s.name}`}
                                />
                              </Show>
                              <button
                                type="button"
                                class="btn-secondary settings-btn mcps-btn mcps-reconnect"
                                disabled={!!busy() || !s.enabled || st().kind === 'untrusted'}
                                title={s.enabled ? `Restart ${s.name}` : `${s.name} is off — turn it on first`}
                                data-testid={`mcp-reconnect-${s.name}`}
                                onClick={() => void reconnect(s)}
                              >
                                <RefreshCw size={13} class={busy() === `rc-${s.name}` ? 'refresh-spinning' : undefined} />
                                Reconnect
                              </button>
                              <Show
                                when={!s.readOnly}
                                fallback={
                                  <>
                                    <button type="button" class="btn-secondary settings-btn mcps-btn" disabled={!projectWritable() && !!cwd()} onClick={() => setSheet({ kind: 'override', server: s })} data-testid={`mcp-override-${s.name}`}>Override in Oino</button>
                                    {/* The Files pane belongs to an open chat's workspace: without one the app says so (CommandCenter's notice). */}
                                    <Show when={desktopShell() && s.sourceFile?.startsWith('/')}>
                                      <button type="button" class="btn-secondary settings-btn mcps-btn" onClick={() => void import('../command/CommandCenter').then((m) => m.openFileFromCenter(props.hostId, s.sourceFile!))} data-testid={`mcp-open-${s.name}`}><FolderOpen size={13} /> Open file</button>
                                  </Show>
                                </>
                              }
                            >
                              <button type="button" class="btn-secondary settings-btn mcps-btn" disabled={!!busy()} onClick={() => setSheet({ kind: 'edit', server: s })} data-testid={`mcp-edit-${s.name}`}><Pencil size={13} /> Edit</button>
                              <Show when={!(cwd() && s.badge === 'global')}>
                                <button type="button" class="btn-secondary settings-btn mcps-btn mcps-remove" disabled={!!busy()} onClick={() => void remove(s)} data-testid={`mcp-remove-${s.name}`}><Trash2 size={13} /> Remove</button>
                              </Show>
                            </Show>
                          </div>
                          <Show
                            when={autonomyHint(s, chat(), !!cwd())}
                            fallback={<Show when={s.warning}><p class="mcps-warn" data-testid={`mcp-warning-${s.name}`}>{s.warning}</p></Show>}
                          >
                            {(h) => (
                              <p class="mcps-warn" data-testid={`mcp-warning-${s.name}`}>
                                <span><Show when={h().note}>{h().note} </Show><strong data-testid={`mcp-autonomy-hint-${s.name}`}>{h().hint}</strong></span>
                              </p>
                            )}
                          </Show>
                          <Show when={s.error}>
                            <p class="mcps-err" data-testid={`mcp-err-${s.name}`}><CircleAlert size={13} /> <span>{s.error}</span></p>
                          </Show>
                          <Show when={open() === s.name}>
                            <div class="mcps-tools" data-testid={`mcp-tools-${s.name}`}>
                              <Show when={s.tools.length} fallback={<span class="mcps-dim">No tools{st().kind === 'connected' ? '' : ' while it is not connected'}.</span>}>
                                <For each={s.tools}>{(t) => <code>{t}</code>}</For>
                              </Show>
                            </div>
                          </Show>
                        </li>
                      );
                    }}
                  </For>
                </ul>
              </Show>
              </Show>
            </>
          )}
        </Show>
      </Show>

      <Show when={sheet()} keyed>
        {(mode) => (
          <McpSheet
            mode={mode}
            project={sheetProject()}
            taken={taken}
            servers={() => state()?.servers ?? []}
            onCatalog={(query) => mcpCatalog(at(), query)}
            onSave={saveFromSheet}
            onClose={() => setSheet(undefined)}
          />
        )}
      </Show>
    </div>
  );
}

/** The project's trust state and its one control (`/trust` lands here). */
export function TrustControl(props: { trust?: ProjectTrust; busy: boolean; onTrust: () => void; onRevoke: () => void }) {
  const t = () => props.trust;
  return (
    <div class="mcps-trust" id="mcp-trust" data-state={t()?.state ?? 'unknown'} data-testid="mcp-trust">
      <Show
        when={t()}
        fallback={
          <>
            <ShieldAlert size={15} aria-hidden="true" />
            <span class="mcps-trust-text">This PC doesn’t report project trust (older UniPi Oino).</span>
          </>
        }
      >
        {(tr) => (
          <Show
            when={tr().state === 'trusted'}
            fallback={
              <>
                <ShieldAlert size={15} aria-hidden="true" />
                <span class="mcps-trust-text"><strong>Not trusted</strong> — project servers aren’t loaded.</span>
                <button type="button" class="btn-primary settings-btn mcps-btn" disabled={props.busy} onClick={props.onTrust} data-testid="mcp-trust-grant">Trust this project</button>
              </>
            }
          >
            <ShieldCheck size={15} aria-hidden="true" />
            <span class="mcps-trust-text"><strong>Trusted</strong>{tr().path ? <> — <code>{shortFile(tr().path!)}</code></> : null}</span>
            <button type="button" class="btn-secondary settings-btn mcps-btn" disabled={props.busy} onClick={props.onRevoke} data-testid="mcp-trust-revoke">Stop trusting</button>
          </Show>
        )}
      </Show>
    </div>
  );
}
