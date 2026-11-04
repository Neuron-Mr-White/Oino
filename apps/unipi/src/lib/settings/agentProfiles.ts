/**
 * Subagent profiles for Settings › Subagents › Agent profiles (owner5 #17):
 * the subagents extension's own file format, read and written through the
 * host's generic fs ops (no extension command, no chat).
 *
 * Mirrors `extensions/unipi/subagents/src/profiles.rs`:
 * - built-ins `subagent_explore` / `subagent_general` (read-only here);
 * - custom agents are `<name>.md` (or `<name>/{AGENT,AGENTS,agent,agents}.md`)
 *   with YAML front matter (`name`, `description`, `model`,
 *   `tools`/`allowed-tools`, `thinking`, `max-nesting`); the body is the
 *   appended system prompt;
 * - any profile can be switched off: `subagents.disabled: [names]` (global,
 *   a project value overrides), written through the settings write path;
 * - global agents live in the extension's `/ext-global/profiles/`
 *   (`<oino dir>/state/unipi.subagents/global/profiles` on the host), project
 *   agents in `<project>/.oino/agents/` (project wins over global).
 */
import { z } from 'zod';
import { fsDelete, fsList, fsMkdir, fsRead, fsWrite } from '../fs';
import { hostCmd } from '../../settings/hostCmd';
import type { HostResult } from '../host';

export type ProfileSource = 'builtin' | 'global' | 'project';

export type AgentProfile = {
  id: string;
  description: string;
  /** Tool allowlist (`undefined` = all tools except the nesting ones). */
  tools?: string[];
  model?: string;
  thinking?: string;
  maxNesting?: number;
  systemPrompt: string;
  source: ProfileSource;
  /** The file it was read from (custom agents). */
  path?: string;
};

export const EXPLORE_TOOLS = ['read', 'grep', 'find', 'ls', 'web_search', 'memory_search', 'memory_list'];
export const DIR_CANDIDATES = ['AGENT.md', 'AGENTS.md', 'agent.md', 'agents.md'];
export const THINKING_LEVELS = ['off', 'minimal', 'low', 'medium', 'high', 'xhigh'] as const;
/** Tools offered in the editor (any tool already in a profile is offered too). */
export const KNOWN_TOOLS = [
  'read', 'write', 'edit', 'bash', 'grep', 'find', 'ls',
  'web_search', 'web_fetch', 'memory_search', 'memory_list', 'memory_store',
  'todo', 'ask_user', 'browser',
];

const EXPLORE_PROMPT =
  'You are a read-only exploration subagent. Investigate the task using only read and search tools; you cannot edit files or run commands. Report concrete findings with file paths and line numbers, and say clearly what you could not determine.';
const GENERAL_PROMPT =
  'You are a general-purpose subagent working on a task delegated by a parent agent. Complete the task, verify your work, and finish with a concise report: what you did, files changed, how you verified it, and anything left open. Do not talk to the user; your final message goes to the parent agent.';

export function builtinProfiles(): AgentProfile[] {
  return [
    { id: 'subagent_explore', description: 'Read-only codebase exploration and research.', tools: [...EXPLORE_TOOLS], systemPrompt: EXPLORE_PROMPT, source: 'builtin' },
    { id: 'subagent_general', description: 'General-purpose tasks including code changes.', systemPrompt: GENERAL_PROMPT, source: 'builtin' },
  ];
}

type FmValue = string | string[];

function unquote(raw: string): string {
  const t = raw.trim();
  if (t.length >= 2 && t.startsWith('"') && t.endsWith('"')) {
    try {
      return JSON.parse(t) as string;
    } catch {
      return t.slice(1, -1);
    }
  }
  if (t.length >= 2 && t.startsWith("'") && t.endsWith("'")) return t.slice(1, -1).replace(/''/g, "'");
  return t;
}

/** pi `parseFrontmatter` subset (same rules as the extension). */
export function parseFrontMatter(raw: string): { fm: Map<string, FmValue>; body: string } {
  const fm = new Map<string, FmValue>();
  const text = raw.replace(/^\uFEFF/, '');
  const rest = text.startsWith('---\n') ? text.slice(4) : text.startsWith('---\r\n') ? text.slice(5) : undefined;
  if (rest === undefined) return { fm, body: text };
  const end = rest.indexOf('\n---');
  if (end < 0) return { fm, body: text };
  const block = rest.slice(0, end);
  const after = rest.slice(end + 4);
  const nl = after.indexOf('\n');
  const body = nl >= 0 ? after.slice(nl + 1) : '';
  let list: string | undefined;
  for (const line of block.split(/\r?\n/)) {
    const trimmed = line.trimEnd();
    if (!trimmed.trim() || trimmed.trimStart().startsWith('#')) continue;
    const item = trimmed.trimStart();
    if (item.startsWith('- ') && list) {
      const cur = fm.get(list);
      if (Array.isArray(cur)) cur.push(unquote(item.slice(2)));
      continue;
    }
    const colon = trimmed.indexOf(':');
    if (colon < 0) continue;
    const key = trimmed.slice(0, colon).trim();
    const value = trimmed.slice(colon + 1).trim();
    if (!value) {
      fm.set(key, []);
      list = key;
    } else if (value.startsWith('[') && value.endsWith(']')) {
      fm.set(key, value.slice(1, -1).split(',').map(unquote).filter(Boolean));
      list = undefined;
    } else {
      fm.set(key, unquote(value));
      list = undefined;
    }
  }
  return { fm, body };
}

const textOf = (fm: Map<string, FmValue>, key: string): string | undefined => {
  const v = fm.get(key);
  return typeof v === 'string' ? v : undefined;
};

function toolList(v: FmValue | undefined): string[] | undefined {
  if (v === undefined) return undefined;
  if (Array.isArray(v)) return [...v];
  return v.split(',').map((t) => t.trim()).filter(Boolean);
}

/** unipi `loadAgentFile`. */
export function parseAgent(raw: string, source: ProfileSource, pathName: string, path?: string): AgentProfile {
  const { fm, body } = parseFrontMatter(raw);
  const id = textOf(fm, 'name')?.trim() || pathName;
  const nesting = Number.parseInt(textOf(fm, 'max-nesting') ?? '', 10);
  return {
    id,
    description: textOf(fm, 'description') ?? id,
    model: textOf(fm, 'model'),
    thinking: textOf(fm, 'thinking'),
    tools: toolList(fm.get('allowed-tools') ?? fm.get('tools')),
    maxNesting: Number.isFinite(nesting) ? nesting : undefined,
    systemPrompt: body.trim(),
    source,
    ...(path ? { path } : {}),
  };
}

/** unipi `NAME_RE` + duplicates; `undefined` = fine. */
export function validateName(name: string, existing: readonly string[]): string | undefined {
  if (!/^[a-z0-9][a-z0-9_-]{0,47}$/.test(name)) return 'Use lowercase letters, digits, - or _ (max 48).';
  if (builtinProfiles().some((b) => b.id === name)) return `${name} is a built-in profile name.`;
  if (existing.includes(name)) return `An agent named ${name} already exists.`;
  return undefined;
}

export type AgentDraft = { name: string; description: string; model?: string; thinking?: string; tools?: string[]; maxNesting?: number; prompt: string };

/** unipi `agentMarkdown` (+ `max-nesting` kept when the file had it). */
export function agentMarkdown(d: AgentDraft): string {
  const fm = [`name: ${d.name}`, `description: ${JSON.stringify(d.description)}`];
  if (d.model?.trim()) fm.push(`model: ${d.model.trim()}`);
  if (d.thinking?.trim()) fm.push(`thinking: ${d.thinking.trim()}`);
  if (d.tools?.length) {
    fm.push('allowed-tools:');
    for (const t of d.tools) fm.push(`  - ${t}`);
  }
  if (d.maxNesting !== undefined) fm.push(`max-nesting: ${d.maxNesting}`);
  return `---\n${fm.join('\n')}\n---\n\n${d.prompt.trim()}\n`;
}

/** unipi `promptTemplate`. */
export function promptTemplate(name: string, description: string): string {
  return `You are the ${name} subagent: ${description}\n\nWork only on the task you are given. You do not see the parent conversation; everything you need is in the task.\n\nFinish with a concise report for the parent agent: what you found or did, with file paths and line numbers, and anything you could not determine.`;
}

/** The two custom folders on the host. */
export function profileDirs(oinoDir: string | undefined, project: string | undefined): { global?: string; project?: string } {
  const trim = (p: string) => p.replace(/[\\/]+$/, '');
  return {
    ...(oinoDir ? { global: `${trim(oinoDir)}/state/unipi.subagents/global/profiles` } : {}),
    ...(project ? { project: `${trim(project)}/.oino/agents` } : {}),
  };
}

/** Every agent file of one folder (missing folder = none). */
export async function readAgentDir(hostId: string, dir: string, source: ProfileSource): Promise<AgentProfile[]> {
  const listing = await fsList(hostId, dir, true).catch(() => undefined);
  if (!listing?.ok) return [];
  const out: AgentProfile[] = [];
  const entries = [...listing.value.entries].sort((a, b) => a.name.localeCompare(b.name));
  for (const entry of entries) {
    if (entry.kind !== 'dir' && entry.name.endsWith('.md')) {
      const path = `${dir}/${entry.name}`;
      const read = await fsRead(hostId, path).catch(() => undefined);
      if (read?.ok) out.push(parseAgent(read.value.text, source, entry.name.slice(0, -3), path));
    } else if (entry.kind === 'dir') {
      const inner = await fsList(hostId, `${dir}/${entry.name}`, true).catch(() => undefined);
      const file = inner?.ok ? DIR_CANDIDATES.find((c) => inner.value.entries.some((e) => e.name === c)) : undefined;
      if (!file) continue;
      const path = `${dir}/${entry.name}/${file}`;
      const read = await fsRead(hostId, path).catch(() => undefined);
      if (read?.ok) out.push(parseAgent(read.value.text, source, entry.name, path));
    }
  }
  return out;
}

/** Built-ins, then global and project agents (a project agent replaces a global one of the same id). */
export async function loadProfiles(hostId: string, dirs: { global?: string; project?: string }): Promise<{ profiles: AgentProfile[]; warnings: string[] }> {
  const [global, project] = await Promise.all([
    dirs.global ? readAgentDir(hostId, dirs.global, 'global') : Promise.resolve([]),
    dirs.project ? readAgentDir(hostId, dirs.project, 'project') : Promise.resolve([]),
  ]);
  const builtins = builtinProfiles();
  const warnings: string[] = [];
  const merged: AgentProfile[] = [];
  for (const p of [...global, ...project]) {
    if (builtins.some((b) => b.id === p.id)) {
      warnings.push(`${p.id}: custom agent shadows a built-in name — skipped`);
      continue;
    }
    const at = merged.findIndex((m) => m.id === p.id);
    if (at >= 0) merged[at] = p;
    else merged.push(p);
  }
  return { profiles: [...builtins, ...merged], warnings };
}

/** Creates `dir` and any missing parents (the host's mkdir needs an existing parent). */
async function ensureDir(hostId: string, dir: string): Promise<void> {
  const parts = dir.split('/');
  for (let i = 2; i <= parts.length; i += 1) {
    const path = parts.slice(0, i).join('/');
    if (!path) continue;
    const listing = await fsList(hostId, path, true).catch(() => undefined);
    if (listing?.ok) continue;
    await fsMkdir(hostId, path).catch(() => undefined);
  }
}

export type SaveResult = { ok: true; path: string } | { ok: false; error: string };

/**
 * Writes `draft` as `<dir>/<name>.md`. An edited agent keeps its file; a
 * rename writes the new file and removes the old one.
 */
export async function saveProfile(hostId: string, dir: string, draft: AgentDraft, previous?: AgentProfile): Promise<SaveResult> {
  const keepPath = previous?.path && previous.id === draft.name ? previous.path : undefined;
  const path = keepPath ?? `${dir}/${draft.name}.md`;
  await ensureDir(hostId, path.slice(0, path.lastIndexOf('/')));
  const written = await fsWrite(hostId, path, agentMarkdown(draft), undefined, true).catch((e: unknown) => ({ ok: false as const, error: { message: String((e as { message?: string })?.message ?? e) } }));
  if (!written.ok) return { ok: false, error: written.error.message };
  if (previous?.path && previous.path !== path) await fsDelete(hostId, previous.path, true).catch(() => undefined);
  return { ok: true, path };
}

export async function deleteProfile(hostId: string, profile: AgentProfile): Promise<SaveResult> {
  if (!profile.path) return { ok: false, error: 'Built-in profiles can’t be deleted.' };
  const result = await fsDelete(hostId, profile.path, true).catch((e: unknown) => ({ ok: false as const, error: { message: String(e) } }));
  return result.ok ? { ok: true, path: profile.path } : { ok: false, error: result.error.message };
}

// ── Enabled / disabled (agent-disable) ──────────────────────────────────────

/** The subagents settings namespace and its disabled-profiles key (`profiles.rs`). */
export const SUBAGENTS_NAMESPACE = 'subagents';
export const DISABLED_KEY = 'disabled';
/** The profile a call without one runs while it is enabled (`DEFAULT_PROFILE`). */
export const DEFAULT_PROFILE = 'subagent_general';

/** Where the disabled list lives: the global list, and the project's when it overrides. */
export type DisabledState = {
  global: string[];
  /** The project's own list (`undefined` = the project follows the global list). */
  project?: string[];
  /** The project folder the project list belongs to. */
  cwd?: string;
};

/** The list in force (the project's when it has one). */
export const effectiveDisabled = (state: DisabledState): string[] => state.project ?? state.global;

/** Mirrors `profiles::offered`: never empty while there are profiles (all off keeps the first). */
export function offeredIds(ids: readonly string[], disabled: readonly string[]): string[] {
  const on = ids.filter((id) => !disabled.includes(id));
  return on.length > 0 ? on : ids.slice(0, 1);
}

/** Mirrors `profiles::default_profile`: `subagent_general` while on, else the first enabled. */
export function defaultProfileId(ids: readonly string[], disabled: readonly string[]): string | undefined {
  const on = offeredIds(ids, disabled);
  return on.includes(DEFAULT_PROFILE) ? DEFAULT_PROFILE : on[0];
}

export type ToggleResult = { ok: true; list: string[] } | { ok: false; error: string };

/** The disabled list with `id` switched; refuses to switch off the last enabled profile. */
export function toggleDisabled(ids: readonly string[], disabled: readonly string[], id: string, enabled: boolean): ToggleResult {
  // Names of profiles that no longer exist are dropped on the way.
  const next = disabled.filter((d) => d !== id && ids.includes(d));
  if (!enabled) {
    next.push(id);
    if (ids.every((p) => next.includes(p))) return { ok: false, error: 'At least one profile must stay enabled.' };
  }
  return { ok: true, list: next };
}

const ValuesReply = z.object({ values: z.unknown() });
const listOf = (values: unknown): string[] | undefined => {
  const raw = typeof values === 'object' && values !== null ? (values as Record<string, unknown>)[DISABLED_KEY] : undefined;
  return Array.isArray(raw) ? raw.filter((v): v is string => typeof v === 'string') : undefined;
};

/** Reads the global list and, for `project`, the project's own list. */
export async function loadDisabled(hostId: string, project?: string): Promise<DisabledState> {
  const read = (layer: 'global' | 'project') =>
    hostCmd({ kind: 'get_extension_settings', namespace: SUBAGENTS_NAMESPACE, layer, ...(project ? { cwd: project } : {}) }, ValuesReply, hostId).catch(() => undefined);
  const [global, local] = await Promise.all([read('global'), project ? read('project') : Promise.resolve(undefined)]);
  const projectList = local?.ok ? listOf(local.value.values) : undefined;
  return {
    global: (global?.ok ? listOf(global.value.values) : undefined) ?? [],
    ...(projectList ? { project: projectList } : {}),
    ...(project ? { cwd: project } : {}),
  };
}

export type WriteResult = { ok: true } | { ok: false; error: string };

const writeError = (e: unknown): WriteResult => ({ ok: false, error: String((e as { message?: string })?.message ?? e) });
const asWrite = (r: HostResult<null> | WriteResult): WriteResult =>
  r.ok ? { ok: true } : { ok: false, error: typeof r.error === 'string' ? r.error : r.error.message || `Not saved (${r.error.code})` };

/**
 * Stores `list` where the list in force lives: the project's when it overrides,
 * else the global one (an empty global list is unset, back to the default).
 */
export async function saveDisabled(hostId: string, state: DisabledState, list: string[]): Promise<WriteResult> {
  const project = state.project !== undefined && state.cwd;
  const cmd = project
    ? ({ kind: 'set_extension_setting', namespace: SUBAGENTS_NAMESPACE, key: DISABLED_KEY, value: list, scope: 'project', cwd: state.cwd } as const)
    : list.length === 0
      ? ({ kind: 'unset_extension_setting', namespace: SUBAGENTS_NAMESPACE, key: DISABLED_KEY, scope: 'global' } as const)
      : ({ kind: 'set_extension_setting', namespace: SUBAGENTS_NAMESPACE, key: DISABLED_KEY, value: list, scope: 'global' } as const);
  return asWrite(await hostCmd(cmd, z.null(), hostId).catch(writeError));
}

/** Drops the project's own list: the project follows the global list again. */
export async function clearProjectDisabled(hostId: string, state: DisabledState): Promise<WriteResult> {
  if (!state.cwd) return { ok: true };
  return asWrite(
    await hostCmd({ kind: 'unset_extension_setting', namespace: SUBAGENTS_NAMESPACE, key: DISABLED_KEY, scope: 'project', cwd: state.cwd }, z.null(), hostId).catch(writeError),
  );
}
