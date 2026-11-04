// [chat-load] Chat stability rig: one session served from a COPY of a real chat (.jsonl),
// paged like the host (`page_start`: the newest `limit` ≤ 500 entries before the cursor,
// moved forward to the first user message of the page; the oldest entries dropped past a
// 512 KiB frame; big strings cut like the host's block bounding). Loaded by run.sh's vite
// config only — never part of the app build. URL: #/chat?scenario=owner-chat&chat=<file>.
import { genCards, genFsTree } from '/src/dev/mock/gen';

const params = new URLSearchParams(location.hash.split('?')[1] ?? '');
const file = params.get('chat') ?? '';
const dir = (import.meta as unknown as { env: Record<string, string> }).env.VITE_CHAT_DIR ?? '';
const xhr = new XMLHttpRequest();
xhr.open('GET', `/@fs${dir}/${file}`, false);
xhr.send();
const BLOCK = 32 * 1024;
const cut = (v: unknown): unknown => {
  if (typeof v === 'string') return v.length > BLOCK ? `${v.slice(0, BLOCK)}\n… (cut)` : v;
  if (Array.isArray(v)) return v.map(cut);
  if (v && typeof v === 'object') return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, cut(x)]));
  return v;
};
type E = { type: string; id: string; message?: { role?: string } };
const lines = xhr.responseText.split('\n').filter(Boolean);
const header = JSON.parse(lines[0] ?? '{}') as { id?: string };
const all: E[] = lines
  .slice(1)
  .map((l) => {
    try {
      return cut(JSON.parse(l)) as E;
    } catch {
      return undefined;
    }
  })
  .filter((e): e is E => !!e && typeof e.id === 'string');
const isUser = (e: E | undefined) => e?.type === 'message' && e.message?.role === 'user';
const size = (e: E) => JSON.stringify(e).length;
const FRAME = 512 * 1024;
function page(end: number, limit: number): number {
  let start = Math.max(0, end - Math.min(500, Math.max(1, limit)));
  if (start > 0) {
    for (let a = start; a < end; a++)
      if (isUser(all[a])) {
        start = a;
        break;
      }
  }
  let bytes = 0;
  for (let i = start; i < end; i++) bytes += size(all[i]!);
  while (bytes > FRAME && start < end - 1) bytes -= size(all[start++]!);
  return start;
}
const SID = header.id ?? 'rig-session';
const tailFrom = page(all.length, 80);
const w = window as unknown as { __RL_PAGES__: number; __RL_CHAT__: unknown };
w.__RL_PAGES__ = 0;
w.__RL_CHAT__ = { sid: SID, entries: all.length, users: all.filter(isUser).length, tail: all.length - tailFrom };
const cards = genCards(6);
cards[0] = { ...cards[0]!, session_id: SID, name: file.replace(/\.jsonl$/, ''), cwd: '/home/qa/project', run_state: 'idle' } as never;

export const ownerChat = {
  name: 'owner-chat',
  hosts: [
    {
      id: 'pc-coffee',
      name: 'coffee',
      conn: { state: 'connected' },
      cards,
      fs: genFsTree(),
      sessions: {
        [SID]: {
          snapshot: {
            session_id: SID,
            reset: true,
            seq: 0,
            state: { sessionId: SID, isStreaming: false, isCompacting: false, model: { provider: 'anthropic', id: 'claude' }, thinkingLevel: 'medium', messageCount: all.length, pendingMessageCount: 0 },
            entries: all.slice(tailFrom),
            leaf_id: all.at(-1)?.id,
            page: { total: all.length, has_more: tailFrom > 0, cursor: all[tailFrom]?.id },
            commands: [],
            ui: { scope: 'session', sessionId: SID, rev: 0, entries: [], dialogs: [] },
            topics: [],
          },
          events: [],
          cmd: {
            get_entries: (cmd: { before?: unknown; limit?: unknown }) => {
              if (typeof cmd.before !== 'string') return { entries: [], leafId: all.at(-1)?.id };
              w.__RL_PAGES__++;
              const end = all.findIndex((e) => e.id === cmd.before);
              const from = page(end, Number(cmd.limit ?? 80));
              return { entries: all.slice(from, end), total: all.length, hasMore: from > 0, cursor: all[from]?.id ?? null };
            },
          },
        },
      },
    },
  ],
};
