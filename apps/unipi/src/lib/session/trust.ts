/**
 * Project trust as the host reports it (OIN-101 WI-1 contract, SPEC §1): the session snapshot's
 * `project_trust {state, path, protected[]}` and `_oino/trust_get {cwd}`'s answer. Read leniently:
 * an older host sends none, which the UI treats as "nothing to ask".
 */
import { createSignal } from 'solid-js';

export type TrustState = 'trusted' | 'untrusted' | 'undecided';
export type ProjectTrust = { state: TrustState; path?: string; protected: string[] };

const obj = (v: unknown): Record<string, unknown> => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : {});
const str = (v: unknown): string | undefined => (typeof v === 'string' && v ? v : undefined);

/** `project_trust` of a session snapshot / `_oino/trust_get`'s answer, leniently. `undefined` = the host sent none. */
export function parseTrust(raw: unknown): ProjectTrust | undefined {
  const o = obj(raw);
  const word = str(o.state) ?? (typeof o.trusted === 'boolean' ? (o.trusted ? 'trusted' : 'untrusted') : undefined);
  if (!word) return undefined;
  const w = word.toLowerCase();
  const state: TrustState = w === 'trusted' || w === 'yes' ? 'trusted' : w === 'undecided' || w === 'unknown' || w === 'ask' || w === 'pending' ? 'undecided' : 'untrusted';
  const prot = Array.isArray(o.protected)
    ? o.protected.map((p) => (typeof p === 'string' ? p : str(obj(p).path) ?? str(obj(p).label) ?? '')).filter(Boolean)
    : [];
  return { state, ...(str(o.path) ? { path: str(o.path) } : {}), protected: prot };
}


/** A decision the user just made in this window, by folder (the banner hides at once; the host's next snapshot agrees). */
const [decisions, setDecisions] = createSignal<Readonly<Record<string, boolean>>>({});
export const trustDecisions = decisions;
export function noteTrustDecision(path: string, trusted: boolean): void {
  setDecisions({ ...decisions(), [path]: trusted });
}
/** Tests. */
export function resetTrustDecisions(): void {
  setDecisions({});
}

/** The folder a snapshot's `project_trust` decides on: its `path`, else the chat's own folder. */
export const trustFolder = (t: ProjectTrust, cwd?: string): string => t.path ?? cwd ?? '';

/** The banner shows for an undecided folder that has protected project resources (SPEC §1). Decided-here folders never do. */
export function needsTrustDecision(t: ProjectTrust | undefined, cwd?: string, decided: Readonly<Record<string, boolean>> = decisions()): t is ProjectTrust {
  if (!t || t.state !== 'undecided' || t.protected.length === 0) return false;
  const covers = (path: string) => Object.keys(decided).some((d) => path === d || path.startsWith(`${d.replace(/[\\/]+$/, '')}/`));
  return !(covers(trustFolder(t, cwd)) || (cwd !== undefined && covers(cwd)));
}

const MCP_FILE = /(^|[\\/])(\.?mcp(-config)?\.json)$/i;

/**
 * "2 MCP config files (.oino/mcp.json, .mcp.json), skills, settings" — what the folder carries,
 * grouped for one line of the banner. Pure.
 */
export function describeProtected(items: readonly string[]): string {
  const mcp = items.filter((p) => MCP_FILE.test(p));
  const rest = items.filter((p) => !MCP_FILE.test(p));
  const words = new Set<string>();
  for (const p of rest) {
    const clean = p.replace(/^\.\//, '').replace(/[\\/]+$/, '');
    const last = clean.split(/[\\/]/).filter(Boolean);
    const head = last.find((seg) => seg !== '.oino' && seg !== '.pi' && seg !== '.unipi' && seg !== '.agents') ?? last.at(-1) ?? clean;
    words.add(head.replace(/\.(md|json)$/i, ''));
  }
  const parts: string[] = [];
  if (mcp.length) parts.push(`MCP servers (${mcp.join(', ')})`);
  parts.push(...words);
  return parts.join(', ');
}

/** Folders offered by "Trust parent": up to three levels up, never the filesystem root or a bare home folder. */
export function parentChoices(path: string): string[] {
  const out: string[] = [];
  let cur = path.replace(/[\\/]+$/, '');
  for (let i = 0; i < 3; i++) {
    const cut = Math.max(cur.lastIndexOf('/'), cur.lastIndexOf('\\'));
    if (cut <= 0) break;
    cur = cur.slice(0, cut);
    const depth = cur.split(/[\\/]/).filter(Boolean).length;
    if (depth < 2 || /^\/(home|Users)\/[^/]+$/.test(cur)) break;
    out.push(cur);
  }
  return out;
}
