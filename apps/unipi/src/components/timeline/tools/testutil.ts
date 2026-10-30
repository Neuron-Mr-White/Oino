/**
 * Test ctx (WP-A8): a minimal `TimelineCtx` over a fake session — entries
 * indexed, `cmd` answering `_oino/render` from a table, `get_entries` from
 * the entries themselves.
 */
import type { SessionEntry } from '../../../lib/session/types';
import type { SessionHandle } from '../../../lib/session/store';
import type { RefChat } from '../../../lib/chat/pathStat';
import type { TimelineCtx } from '../context';

export type FakeCtxOptions = {
  /** `_oino/render` answers: `"target/key"` → node | null; `"target/key": {never:true}` never resolves. */
  renderers?: Record<string, unknown>;
  /** Extra command handlers (`type` → reply). */
  cmd?: Record<string, unknown>;
  /** Overrides `get_entries` (default: the fixture entries). */
  getEntries?: (ids: readonly string[]) => Promise<SessionEntry[]>;
};

export function fakeCtx(entries: SessionEntry[], opts: FakeCtxOptions = {}): TimelineCtx {
  const byId: Record<string, number> = {};
  entries.forEach((e, i) => (byId[e.id] = i));
  const session = {
    key: `h1/s-${fakeCtxN++}`,
    hostId: 'h1',
    sessionId: () => 's1',
    state: { entries, byId },
    cmd: (command: { type: string; target?: unknown; key?: unknown }) => {
      const extra = opts.cmd?.[command.type];
      if (extra !== undefined) return Promise.resolve(extra);
      if (command.type === '_oino/render') {
        const spec = opts.renderers?.[`${String(command.target)}/${String(command.key)}`];
        if (spec && typeof spec === 'object' && 'never' in (spec as Record<string, unknown>)) return new Promise(() => {});
        return Promise.resolve({ node: spec === undefined ? null : spec });
      }
      return Promise.resolve({});
    },
    getEntries: opts.getEntries ?? ((ids) => Promise.resolve(entries.filter((e) => ids.includes(e.id)))),
  } as unknown as SessionHandle;
  return {
    session,
    hostId: 'h1',
    sessionId: () => 's1',
    cwd: () => '/proj',
    refs: {} as RefChat,
    commands: () => new Set<string>(),
    columnWidth: () => 720,
  };
}

let fakeCtxN = 0;
