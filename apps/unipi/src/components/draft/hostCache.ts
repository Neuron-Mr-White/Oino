/**
 * What a PC offers, remembered for the new-chat draft (P0-09): the draft has
 * no session yet, and oino lists commands and models per session. Every chat
 * the composer shows records its `/` commands, and every model list the draft
 * fetches is kept, per PC, so a draft opened before any chat (an app restart)
 * still has the full `/` menu and model menu. localStorage, small, best effort.
 */
import type { CommandInfo } from '../../lib/session/types';
import type { ModelInfo } from '../../lib/model/picker';

const KEY = (hostId: string, what: 'commands' | 'models') => `unipi.draft.${what}.${hostId}`;
const MAX_ROWS = 2000;

function read<T>(key: string): T[] | undefined {
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return undefined;
    const parsed = JSON.parse(raw) as unknown;
    return Array.isArray(parsed) && parsed.length ? (parsed as T[]) : undefined;
  } catch {
    return undefined;
  }
}

function write(key: string, rows: readonly unknown[]): void {
  try {
    localStorage.setItem(key, JSON.stringify(rows.slice(0, MAX_ROWS)));
  } catch {
    // quota / private mode: the cache is an optimisation only
  }
}

/** The last `/` commands a chat on this PC listed (name, description, source only). */
export function cachedCommands(hostId: string): CommandInfo[] | undefined {
  return read<CommandInfo>(KEY(hostId, 'commands'));
}

let lastCommands = new Map<string, string>();
export function rememberCommands(hostId: string, commands: readonly CommandInfo[]): void {
  if (!commands.length) return;
  const rows = commands.map((c) => ({ name: c.name, ...(c.description ? { description: c.description } : {}), ...(c.source ? { source: c.source } : {}) }));
  const sig = JSON.stringify(rows);
  if (lastCommands.get(hostId) === sig) return;
  lastCommands.set(hostId, sig);
  write(KEY(hostId, 'commands'), rows);
}

export function cachedModels(hostId: string): ModelInfo[] | undefined {
  return read<ModelInfo>(KEY(hostId, 'models'));
}

export function rememberModels(hostId: string, models: readonly ModelInfo[]): void {
  if (!models.length) return;
  write(
    KEY(hostId, 'models'),
    models.map((m) => ({ provider: m.provider, id: m.id, ...(m.name ? { name: m.name } : {}), ...(m.reasoning !== undefined ? { reasoning: m.reasoning } : {}), ...(m.input ? { input: m.input } : {}), ...(m.thinkingLevelMap ? { thinkingLevelMap: m.thinkingLevelMap } : {}) })),
  );
}

/** Tests. */
export function resetHostCache(): void {
  lastCommands = new Map();
}
