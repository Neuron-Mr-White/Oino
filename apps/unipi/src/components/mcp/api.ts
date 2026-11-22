/**
 * Settings › MCP — the host calls (SPEC §1/§2). Extension services run in Settings' own command chat, rooted at
 * the project when one is picked (`settingsCommandChat(host, _, cwd)`), so the extension answers for that project's
 * folder and never for the folder the app started in:
 *
 *   mcp.servers {cwd?}                           → servers, warnings, files, trust
 *   mcp.save {scope, cwd?, entries, secrets}     → writes `.oino/mcp.json` (project) / `~/.oino/mcp.json` (global)
 *   mcp.remove {scope, cwd?, name}
 *   mcp.set-enabled {scope, cwd?, name, enabled} → in project scope on a global server: a partial override
 *   mcp.server-action {action, name?, cwd?}      → reconnect / reload / sync (unchanged)
 *   _oino/trust_get {cwd} · _oino/trust_set {path, trusted}   (host ops, any chat)
 *
 * A host or extension from before this contract answers "unknown service / command": that is `Unsupported`,
 * which the callers turn into a plain sentence — never a broken page.
 */
import { hostClient } from '../../lib/host/factory';
import { autonomyKnown, autonomyLevel, type AutonomyLevel } from '../../lib/session/autonomy';
import { rememberedAutonomy } from '../../lib/session/chatAutonomy';
import { hubs } from '../../lib/session/hub';
import { sessions, type SessionHandle } from '../../lib/session/store';
import type { SessionCard } from '../../lib/host/client';
import { resolveHostId } from '../../lib/host/localId';
import { settingsCommandChat } from '../../lib/settings/run';
import { parseMcpState, parseTrust, type McpScope, type McpState, type ProjectTrust } from './model';

export class Unsupported extends Error {
  constructor(what: string) {
    super(`This PC’s UniPi Oino is too old for ${what} — update it, then reopen this page.`);
    this.name = 'Unsupported';
  }
}

/** The host/extension has no such service or command (an older build). */
export function isUnsupportedError(e: unknown): boolean {
  const m = String((e as { message?: unknown })?.message ?? e);
  return /unknown (service|command|op|host_cmd)|no such (service|command)|not (a )?(registered|supported)|unsupported|E_UNSUPPORTED|E_UNKNOWN|no service/i.test(m);
}

const NO_AGENT = 'Couldn’t reach the agent on this PC — try again in a moment.';

const trimSlash = (p: string): string => p.replace(/[\\/]+$/, '');
const RANK: Record<AutonomyLevel, number> = { low: 0, medium: 1, high: 2 };

/** An open chat of the project whose autonomy the project's servers run at. `live` = its handle is still in the registry. */
export type ProjectChat = { level: AutonomyLevel; title: string; live?: SessionHandle };

/**
 * N2: the open chat whose autonomy applies to a project's servers. The services set is shared by every chat of the
 * project, so a Settings action must never run lower than any open chat: the highest level wins; on a tie the most
 * recently used chat (registry order, then the hub's last activity). Chats the registry already evicted (LRU) still
 * count while the hub has their card, with the level they last reported.
 */
export function projectChat(hostId: string, cwd?: string): ProjectChat | undefined {
  if (!cwd) return undefined;
  const host = resolveHostId(hostId);
  const here = trimSlash(cwd);
  const registry = sessions();
  const found: ProjectChat[] = [];
  const seen = new Set<string>();
  let cards: SessionCard[] = [];
  try { cards = Object.values(hubs().ensure(host).state.cards); } catch { cards = []; }
  const cardOf = (id: string) => cards.find((c) => c.session_id === id);
  const keys = registry.keys();
  for (let i = keys.length - 1; i >= 0; i--) {
    const key = keys[i]!;
    const split = key.indexOf('/');
    if (resolveHostId(key.slice(0, split)) !== host) continue;
    const id = key.slice(split + 1);
    const live = registry.peek(key.slice(0, split), id);
    if (live?.state.conn !== 'live' || trimSlash(String(live.state.info.cwd ?? '')) !== here) continue;
    seen.add(id);
    if (!autonomyKnown(live)) continue;
    found.push({ level: autonomyLevel(live), title: live.state.name?.trim() || cardOf(id)?.name?.trim() || UNTITLED, live });
  }
  const rest = cards
    .filter((c) => !seen.has(c.session_id) && c.cwd && trimSlash(c.cwd) === here && c.run_state !== 'crashed' && c.run_state !== 'taken_over')
    .sort((a, b) => (b.last_activity ?? b.updated_at ?? 0) - (a.last_activity ?? a.updated_at ?? 0));
  for (const c of rest) {
    const level = rememberedAutonomy(host, c.session_id);
    if (level) found.push({ level, title: c.name?.trim() || UNTITLED });
  }
  // Array#sort is stable: equal levels keep the preference order above.
  return found.sort((a, b) => RANK[b.level] - RANK[a.level])[0];
}

const UNTITLED = 'Untitled chat';

/** The open chat's handle when it is the one the project's servers should run at (kept for callers that only want a live chat). */
export function mcpContextChat(hostId: string, cwd?: string) {
  return projectChat(hostId, cwd)?.live;
}

/**
 * The row attribution (N5/N6): nothing on Global (there is no project); in a project, the chat whose autonomy applies
 * — by its title, never a session id — or the Settings default when no chat of the project is open.
 */
export function mcpAutonomyContext(hostId: string, cwd?: string, defaultLevel = 'medium'): string | undefined {
  if (!cwd) return undefined;
  const chat = projectChat(hostId, cwd);
  return chat ? `Autonomy ${chat.level} · from chat “${chat.title}”` : `Autonomy ${defaultLevel} · Settings default (no open chat in this project)`;
}

async function chatFor(hostId: string, cwd?: string) {
  const chat = projectChat(hostId, cwd);
  if (chat?.live) return chat.live;
  const handle = await settingsCommandChat(hostId, undefined, cwd);
  if (!handle) throw new Error(NO_AGENT);
  // Re-evaluate on every action, not only when the scratch session was born. An open chat of the project (even one
  // the registry let go) sets the level: never reload its servers lower than it runs.
  let level: string | undefined = chat?.level;
  if (!level) {
    const raw = await hostClient(hostId).request<{ value?: unknown }>('host_cmd', { cmd: { kind: 'get_settings', key: 'autonomy.defaultLevel' } });
    level = typeof raw.value === 'string' ? raw.value : 'medium';
  }
  if (['low', 'medium', 'high'].includes(level)) await handle.cmd({ type: 'set_autonomy', level } as never);
  return handle;
}

async function service(hostId: string, cwd: string | undefined, name: string, payload: Record<string, unknown> = {}): Promise<unknown> {
  const handle = await chatFor(hostId, cwd);
  try {
    return await handle.cmd<unknown>({ type: '_oino/service_call', name, payload: { ...(cwd ? { cwd } : {}), ...payload } });
  } catch (e) {
    if (isUnsupportedError(e)) throw new Unsupported(name);
    throw e;
  }
}

const rec = (v: unknown): Record<string, unknown> => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : {});

export type Scoped = { hostId: string; cwd?: string };

/** Session-free status read: visiting scopes never pins hidden chat runtimes. */
export async function loadMcp(at: Scoped): Promise<McpState> {
  let result: unknown;
  try { result = await hostClient(at.hostId).request('host_cmd', { cmd: { kind: 'mcp_servers', ...(at.cwd ? { cwd: at.cwd } : {}) } }); }
  catch (e) { if (isUnsupportedError(e)) throw new Unsupported('MCP settings'); throw e; }
  const state = parseMcpState(result);
  // The extension knows effective trust; only the host knows saved/undecided and protected paths.
  const trust = at.cwd ? await getTrust(at.hostId, at.cwd) : undefined;
  return trust ? { ...state, trust } : state;
}

export type SaveReply = { ok: boolean; message?: string; state?: McpState; notice?: string };

function replyOf(raw: unknown): SaveReply {
  const r = rec(raw);
  const state = r.servers && typeof r.servers === 'object' ? parseMcpState(Array.isArray(r.servers) ? { servers: r.servers } : r.servers) : undefined;
  return {
    ok: r.ok !== false,
    ...(typeof r.message === 'string' && r.message ? { message: r.message } : {}),
    ...(typeof r.notice === 'string' && r.notice ? { notice: r.notice } : {}),
    ...(state ? { state } : {}),
  };
}

export async function saveMcp(at: Scoped, scope: McpScope, entries: Record<string, Record<string, unknown>>, secrets: Record<string, string>): Promise<SaveReply> {
  return replyOf(await service(at.hostId, scope === 'project' ? at.cwd : undefined, 'mcp.save', { scope, entries, secrets }));
}

export async function removeMcp(at: Scoped, scope: McpScope, name: string): Promise<SaveReply> {
  return replyOf(await service(at.hostId, scope === 'project' ? at.cwd : undefined, 'mcp.remove', { scope, name }));
}

/** On/off. A global server in project scope writes a partial override into the project's `.oino/mcp.json`. */
export async function setMcpEnabled(at: Scoped, scope: McpScope, name: string, enabled: boolean): Promise<SaveReply> {
  try {
    return replyOf(await service(at.hostId, scope === 'project' ? at.cwd : undefined, 'mcp.set-enabled', { scope, name, enabled }));
  } catch (e) {
    // Extension before WI-2: the old action only knows global servers.
    if (!(e instanceof Unsupported) || scope === 'project') throw e;
    return replyOf(await service(at.hostId, undefined, 'mcp.server-action', { action: enabled ? 'enable' : 'disable', name }));
  }
}

/** `reconnect` (one server), `reload` (the scope; global = every live project) or `sync` (catalog). */
export async function mcpAction(at: Scoped, action: 'reconnect' | 'reload' | 'sync', name?: string): Promise<SaveReply> {
  return replyOf(await service(at.hostId, at.cwd, 'mcp.server-action', { action, scope: at.cwd ? 'project' : 'global', ...(name ? { name } : {}) }));
}

/* ── trust (host ops) ───────────────────────────────────────────────────── */

async function hostOp(hostId: string, cwd: string | undefined, cmd: Record<string, unknown>): Promise<unknown> {
  const handle = await chatFor(hostId, cwd);
  try {
    return await handle.cmd<unknown>(cmd as never);
  } catch (e) {
    if (isUnsupportedError(e)) throw new Unsupported('project trust');
    throw e;
  }
}

/** `_oino/trust_get {cwd}`. `undefined` = the host does not know trust (older build). */
export async function getTrust(hostId: string, cwd: string): Promise<ProjectTrust | undefined> {
  try {
    const result = await hostClient(hostId).request('host_cmd', { cmd: { kind: 'trust_get', cwd } });
    return parseTrust(result);
  } catch (e) {
    if (e instanceof Unsupported || isUnsupportedError(e)) return undefined;
    throw e;
  }
}

/** `_oino/trust_set {path, trusted}` — the host saves the decision, then reloads project resources + MCP of every chat under `path`. */
export async function setTrust(hostId: string, cwd: string, path: string, trusted: boolean): Promise<void> {
  await hostOp(hostId, cwd, { type: '_oino/trust_set', path, trusted });
}

export type CatalogItem = { id: string; name: string; label: string; description: string; entry: Record<string, unknown> };
export async function mcpCatalog(at: Scoped, query: string): Promise<CatalogItem[]> {
  const raw = rec(await service(at.hostId, at.cwd, 'mcp.catalog', { query }));
  return (Array.isArray(raw.entries) ? raw.entries : []) as CatalogItem[];
}
