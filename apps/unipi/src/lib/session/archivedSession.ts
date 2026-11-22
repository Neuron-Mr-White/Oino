/**
 * Read-only archived session handle (D-17): opens a trashed session from its
 * `.jsonl` file (`card.file` via `fsRead`) without calling `session_open` (which
 * only resolves live sessions in `<sessions root>/`). Shows the full cached/disk
 * transcript immediately with a Restore banner; restoring promotes the route
 * seamlessly to a live `sessions().ensure(...)` handle.
 */
import { batch, createMemo, untrack } from 'solid-js';
import { createStore, produce, unwrap } from 'solid-js/store';
import { fsRead } from '../fs';
import type { SessionCard } from '../host/client';
import { applySnapshot, initialState } from './reducer';
import type { SessionHandle } from './store';
import { createTurnsIndex } from './turns';
import type { OutboxItem, SessionEntry, SessionState } from './types';

const isObj = (v: unknown): v is Record<string, unknown> => !!v && typeof v === 'object' && !Array.isArray(v);

/** Parses an Oino `.jsonl` session file into snapshot entries + latest header/meta facts. */
export function parseArchivedJsonl(text: string): {
  entries: SessionEntry[];
  leafId: string | null;
  cwd?: string;
  sessionName?: string;
  model?: { provider?: string; id?: string };
  thinkingLevel?: string;
} {
  const entries: SessionEntry[] = [];
  let cwd: string | undefined;
  let sessionName: string | undefined;
  let model: { provider?: string; id?: string } | undefined;
  let thinkingLevel: string | undefined;
  for (const rawLine of text.split('\n')) {
    const line = rawLine.trim();
    if (!line) continue;
    let parsed: unknown;
    try {
      parsed = JSON.parse(line);
    } catch {
      continue;
    }
    if (!isObj(parsed) || typeof parsed.type !== 'string') continue;
    if (parsed.type === 'session') {
      if (typeof parsed.cwd === 'string' && parsed.cwd) cwd = parsed.cwd;
      continue;
    }
    if (typeof parsed.id !== 'string') continue;
    if (parsed.type === 'session_info' && typeof parsed.name === 'string') {
      sessionName = parsed.name;
    } else if (parsed.type === 'model_change') {
      const id = typeof parsed.modelId === 'string' ? parsed.modelId : typeof parsed.model === 'string' ? parsed.model : undefined;
      const provider = typeof parsed.provider === 'string' ? parsed.provider : undefined;
      if (id) model = { ...(provider ? { provider } : {}), id };
    } else if (parsed.type === 'thinking_level_change') {
      const lvl = typeof parsed.thinkingLevel === 'string' ? parsed.thinkingLevel : typeof parsed.level === 'string' ? parsed.level : undefined;
      if (lvl) thinkingLevel = lvl;
    }
    entries.push(parsed as SessionEntry);
  }
  return {
    entries,
    leafId: entries.at(-1)?.id ?? null,
    ...(cwd ? { cwd } : {}),
    ...(sessionName !== undefined ? { sessionName } : {}),
    ...(model ? { model } : {}),
    ...(thinkingLevel ? { thinkingLevel } : {}),
  };
}

export function createArchivedSession(hostId: string, card: SessionCard): SessionHandle {
  const sessionId = card.session_id;
  const init = initialState(hostId, sessionId);
  init.conn = 'live';
  if (card.name?.trim()) init.name = card.name.trim();
  init.info = { cwd: card.cwd ?? '', ...(card.name?.trim() ? { sessionName: card.name.trim() } : {}) };

  const [state, setState] = createStore<SessionState>(init);
  const index = createTurnsIndex();
  const turns = createMemo(() => {
    void state.entries.length;
    void state.entriesRev;
    void state.leafId;
    return untrack(() => index.update(unwrap(state)));
  });

  let disposed = false;
  /**
   * chat-vanish: a read that failed used to leave `conn: 'live'` with no entries — the
   * "new chat" empty state for a chat that has a transcript, and Retry did nothing. Now it
   * says so (the timeline's error card) and `retry()` reads again.
   */
  const load = () => {
    if (!card.file) {
      setState(produce((s) => ((s.conn = 'closed'), (s.connError = { code: 'E_NOT_FOUND', message: 'This archived chat has no file to read.', retryable: false } as never))));
      return;
    }
    setState('conn', 'connecting');
    void fsRead(hostId, card.file, true, true)
      .then((res) => {
        if (disposed) return;
        if (!res.ok) throw res.error;
        const parsed = parseArchivedJsonl(res.value.text);
        batch(() =>
          setState(
            produce((s) => {
              applySnapshot(s, {
                session_id: sessionId,
                seq: 1,
                reset: true,
                entries: parsed.entries,
                leaf_id: parsed.leafId,
                state: {
                  cwd: parsed.cwd ?? card.cwd ?? '',
                  sessionName: parsed.sessionName ?? card.name ?? undefined,
                  isStreaming: false,
                  ...(parsed.model ? { model: parsed.model } : {}),
                  ...(parsed.thinkingLevel ? { thinkingLevel: parsed.thinkingLevel } : {}),
                },
              });
            }),
          ),
        );
      })
      .catch((error: unknown) => {
        if (disposed) return;
        const e = error as { code?: string; message?: string } | undefined;
        setState(
          produce((s) => {
            s.conn = 'closed';
            s.connError = { code: e?.code ?? 'E_INTERNAL', message: e?.message || "Couldn't read this archived chat.", retryable: true } as never;
          }),
        );
      });
  };
  load();

  return {
    key: `archived:${hostId}/${sessionId}`,
    hostId,
    sessionId: () => sessionId,
    state,
    turns,
    send(text): OutboxItem {
      return { ref: 'archived', text, mode: 'prompt', status: 'lost', at: Date.now(), error: 'Archived chat — restore to reply' };
    },
    resend() {},
    dismiss() {},
    abort: async () => undefined,
    async cmd<T = unknown>(cmd: { type: string; ids?: unknown }): Promise<T> {
      if (cmd.type === 'get_commands') return { commands: [] } as T;
      if (cmd.type === '_oino/get_shortcuts') return { shortcuts: [] } as T;
      if (cmd.type === '_oino/render') return { node: null } as T;
      if (cmd.type === 'get_tree') {
        return { tree: [], leafId: state.leafId } as T;
      }
      if (cmd.type === 'get_entries') {
        const ids = Array.isArray(cmd.ids) ? new Set(cmd.ids.map(String)) : undefined;
        const list = ids ? state.entries.filter((e) => ids.has(e.id)) : state.entries;
        return { entries: unwrap(list), leafId: state.leafId } as T;
      }
      return {} as T;
    },
    subscribeTopics() {},
    async getEntries(ids) {
      const want = new Set(ids);
      return unwrap(state.entries.filter((e) => want.has(e.id)));
    },
    retry() {
      if (!disposed) load();
    },
    takeBack() {},
    acquire: () => () => {},
    close() {
      disposed = true;
    },
  };
}
