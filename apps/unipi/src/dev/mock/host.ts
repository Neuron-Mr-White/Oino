/**
 * Mock host (app spec §3.15, WP-A1): `MockHost implements HostClient` with
 * in-memory hosts, session cards, sessions, fs tree, git status and cron
 * data, for the capture harness (`vite.config.capture.ts`) and component
 * tests (`createMockHost(scenario)`). Never opens a socket.
 *
 * Every op is answered: scenario `ops` overrides first, then the built-in
 * table below, else `null` (logged with `unhandled: true` so a capture can
 * spot an op a screen needs). Every request, session command and stream
 * open is recorded on `world.log` (and `window.__UNIPI_MOCK_LOG__` in the
 * browser) for assertions.
 */
import { createSignal } from 'solid-js';
import {
  APP_HOST,
  type Closable,
  type EventsStream,
  type HostClient,
  type HostConn,
  type HubEvent,
  type LspStream,
  type RpcCommand,
  type SessionCard,
  type SessionEventDto,
  type SessionSnapshot,
  type SessionStream,
  type TermStream,
} from '../../lib/host/client';

export type MockOp = (args: Record<string, unknown>, host: MockHost) => unknown;

/** One step of a session script: an event, or the stream dropping (`down` ms unreachable; `Infinity` = stays down). */
export type MockStep = { at: number; dto: SessionEventDto } | { at: number; drop: { code?: string; message?: string; down?: number } };

/**
 * A recorded session: snapshot + script replayed on a virtual clock (from the
 * first open). A reopen with `sinceSeq` gets a resume patch (`replay` = the
 * due events after it), like the host's ring (host.md §7.4). `cmd` answers
 * session commands by `type` (default: `get_commands` from the snapshot,
 * `get_entries` from `entries`, everything else `{}`).
 */
export type MockSession = {
  snapshot: SessionSnapshot;
  events?: MockStep[];
  /** What `get_entries` answers (the host's entries after the script ran). */
  entries?: { entries: unknown[]; leafId: string | null };
  cmd?: Record<string, (cmd: RpcCommand, host: MockHost) => unknown>;
};

/** One changed file of a mock repo (the `git status` row the Git pane draws). */
export type MockGitFile = {
  path: string;
  area: 'staged' | 'unstaged' | 'untracked' | 'conflict';
  /** Porcelain letter (`M`, `A`, `D`, `?`, `U`; default `M`, `?` for untracked). */
  status?: string;
  add?: number;
  del?: number;
  binary?: boolean;
  /** Rename source. */
  orig?: string;
};

/** A mock repo (WP-A15): every `git_op` action answers from this one spec. */
export type MockGit = {
  repo?: boolean;
  root?: string;
  branch?: string;
  head?: string;
  detached?: boolean;
  upstream?: string;
  ahead?: number;
  behind?: number;
  remotes?: string[];
  stashes?: number;
  operation?: string;
  /** `git status` rows, in the grouping the pane shows. */
  files?: MockGitFile[];
  /** Branch source: the base ref; `null` = no base (the pane's empty state). */
  base?: string | null;
  /** `git log` (newest first); `short`/`author`/`time`/`refs` default. */
  commits?: Array<{ hash: string; short?: string; subject: string; author?: string; time?: number; refs?: string[] }>;
  /** The status list was capped ("Showing the first 2 000 changed files"). */
  more?: boolean;
  /** Diff text per path (default: a plausible hunk). */
  diffs?: Record<string, string>;
};

export type MockFsNode = { kind: 'dir'; children: Record<string, MockFsNode> } | { kind: 'file'; text: string; mtime?: number };

export type MockHostSpec = {
  id: string;
  name: string;
  /** `local` = This computer (desktop app), else iroh. */
  transport?: 'local' | 'iroh';
  conn?: HostConn;
  cards?: SessionCard[];
  sessions?: Record<string, MockSession>;
  /** Root folders → tree. */
  fs?: Record<string, MockFsNode>;
  git?: MockGit;
  cron?: unknown[];
  ops?: Record<string, MockOp>;
};

export type MockScenario = {
  name: string;
  hosts: MockHostSpec[];
  /** App-level op overrides (`list_hosts`, `forward_list`, …). */
  appOps?: Record<string, MockOp>;
  /** 0 = settle scripts instantly (default); otherwise playback speed multiplier. */
  speed?: number;
  /** Capture only: runs in `boot.ts` after storage is cleared (seed prefs, plug stand-in pane views). */
  setup?: () => void;
};

export type MockLogEntry = { host: string; kind: 'request' | 'cmd' | 'open'; op: string; args?: unknown; unhandled?: true; at: number };

declare global {
  interface Window {
    __UNIPI_MOCK_LOG__?: MockLogEntry[];
  }
}

type Err = { code: string; message: string; hint?: string; retryable: boolean };
const err = (code: string, message: string, hint?: string): Err => ({ code, message, hint, retryable: code === 'E_TRANSPORT' || code === 'E_TIMEOUT' });

function walk(root: Record<string, MockFsNode> | undefined, path: string): MockFsNode | undefined {
  if (!root) return undefined;
  const base = Object.keys(root).sort((a, b) => b.length - a.length).find((r) => path === r || path.startsWith(`${r}/`));
  if (!base) return undefined;
  let node: MockFsNode | undefined = root[base];
  for (const part of path.slice(base.length).split('/').filter(Boolean)) {
    node = node?.kind === 'dir' ? node.children[part] : undefined;
  }
  return node;
}

/** The repo half of every `git_op` answer (proto `GitRepoData`). */
function mockRepo(g: MockGit) {
  return {
    repo: g.repo !== false,
    root: g.root,
    branch: g.branch ?? 'main',
    head: g.head ?? 'a1b2c3d4',
    detached: g.detached ?? false,
    upstream: g.upstream,
    ahead: g.ahead ?? 0,
    behind: g.behind ?? 0,
    remotes: g.remotes ?? ['origin'],
    stashes: g.stashes ?? 0,
    operation: g.operation,
  };
}

/** A plausible unified diff for a row (rows may carry `diffs[path]` instead). */
function mockDiff(g: MockGit, path: string, file?: MockGitFile) {
  if (g.diffs?.[path]) return { text: g.diffs[path]!, truncated: false, binary: false };
  if (file?.binary) return { text: '', truncated: false, binary: true };
  const old = `const total = items.length;\nreturn total;\n`;
  const next = `const total = items.reduce((n, item) => n + item.weight, 0);\nreturn total;\n`;
  return {
    text: [
      `diff --git a/${path} b/${path}`,
      `--- a/${path}`,
      `+++ b/${path}`,
      `@@ -12,7 +12,9 @@ export function tally(items: Item[]): number {`,
      '   // Count what the list weighs, not how many rows it has.',
      ...old.split('\n').slice(0, -1).map((line) => `-${line}`),
      ...next.split('\n').slice(0, -1).map((line) => `+${line}`),
      ' }',
      '',
    ].join('\n'),
    truncated: false,
    binary: false,
  };
}

const mockRow = (g: MockGit, f: MockGitFile) => ({
  path: f.path,
  area: f.area,
  status: f.status ?? (f.area === 'untracked' ? '?' : f.area === 'conflict' ? 'U' : 'M'),
  add: f.add ?? 0,
  del: f.del ?? 0,
  binary: f.binary ?? false,
  ...(f.orig ? { orig: f.orig } : {}),
});

/** `git_op` (WP-A15): every action answers from the spec, writes mutate `files`. */
function mockGitOp(cmd: { action: string; [k: string]: unknown }, g: MockGit): unknown {
  const files = g.files ?? [];
  const withDiff = (f: MockGitFile) => ({ ...mockRow(g, f), diff: mockDiff(g, f.path, f) });
  switch (cmd.action) {
    case 'repo':
      return mockRepo(g);
    case 'status':
      return { ...mockRepo(g), files: files.map((f) => mockRow(g, f)), more: g.more ?? false };
    case 'diff': {
      const path = String(cmd.file ?? '');
      const file = files.find((f) => f.path === path);
      return mockDiff(g, path, file);
    }
    case 'turn_files': {
      const asked = new Set((cmd.files as string[] | undefined) ?? []);
      return { ...mockRepo(g), files: files.filter((f) => asked.has(f.path)).map(withDiff) };
    }
    case 'branch_files':
      return {
        ...mockRepo(g),
        base: g.base === undefined ? (g.upstream ?? 'main') : g.base,
        files: g.base === null ? [] : files.map(withDiff),
      };
    case 'log': {
      const all = g.commits ?? [];
      const skip = Number(cmd.skip ?? 0);
      const limit = Number(cmd.limit ?? 50);
      return {
        commits: all.slice(skip, skip + limit).map((c) => ({
          hash: c.hash,
          short: c.short ?? c.hash.slice(0, 7),
          subject: c.subject,
          author: c.author ?? 'You',
          time: c.time ?? 1_760_000_000,
          refs: c.refs ?? [],
        })),
        more: skip + limit < all.length,
      };
    }
    case 'show': {
      const rev = String(cmd.rev ?? '');
      const commit = (g.commits ?? []).find((c) => c.hash === rev || c.hash.startsWith(rev));
      const row = { hash: rev, short: rev.slice(0, 7), subject: commit?.subject ?? 'Commit', author: commit?.author ?? 'You', time: commit?.time ?? 1_760_000_000, refs: commit?.refs ?? [] };
      return {
        commit: row,
        body: '',
        files: files.slice(0, 4).map((f) => ({ path: f.path, status: f.status ?? 'M', add: f.add ?? 0, del: f.del ?? 0 })),
        diff: files.slice(0, 4).map((f) => mockDiff(g, f.path, f).text).join(''),
        truncated: false,
      };
    }
    case 'branches':
      return {
        branches: [
          { name: g.branch ?? 'main', current: true, upstream: g.upstream, ahead: g.ahead ?? 0, behind: g.behind ?? 0, time: 1_760_000_000, subject: g.commits?.[0]?.subject ?? 'Commit' },
          { name: 'feat/review-pane', time: 1_759_000_000, subject: 'Add the review pane' },
          ...(g.remotes ?? []).map((r) => ({ name: `${r}/${g.branch ?? 'main'}`, remote: true })),
        ],
      };
    case 'stage': {
      const paths = (cmd.paths as string[] | undefined) ?? files.map((f) => f.path);
      for (const f of files) if (paths.includes(f.path) && f.area !== 'conflict') f.area = 'staged';
      return null;
    }
    case 'unstage': {
      const paths = (cmd.paths as string[] | undefined) ?? files.map((f) => f.path);
      for (const f of files) if (paths.includes(f.path) && f.area === 'staged') f.area = f.status === 'A' ? 'untracked' : 'unstaged';
      return null;
    }
    case 'discard':
      g.files = files.filter((f) => !((cmd.paths as string[] | undefined) ?? []).includes(f.path));
      return null;
    case 'commit': {
      const subject = String(cmd.message ?? '').split('\n')[0];
      g.files = files.filter((f) => f.area !== 'staged');
      const head = { hash: g.head ?? 'a1b2c3d4', subject, author: 'You', time: Math.floor(Date.now() / 1000), refs: [`HEAD -> ${g.branch ?? 'main'}`] };
      g.commits = [head, ...(g.commits ?? [])];
      return { hash: head.hash, subject };
    }
    case 'switch':
      g.branch = String(cmd.branch ?? g.branch ?? 'main');
      return null;
    case 'pull':
      return { output: 'Already up to date.' };
    case 'push':
      return { output: `To origin\n   ${g.branch ?? 'main'} -> ${g.branch ?? 'main'}` };
    default:
      return { output: 'Done.' };
  }
}

const BUILTIN: Record<string, MockOp> = {
  host_info: (_a, h) => ({ name: h.spec.name, version: '2.0.0', proto: '2.0.0', tools: ['oino', 'unipi-kanboard'] }),
  // host.md §7.2: a bare array of cards, newest activity first.
  sessions_list: (_a, h) => h.spec.cards ?? [],
  session_meta_set: (a, h) => {
    const card = h.spec.cards?.find((c) => c.session_id === a.session);
    if (!card) throw err('E_NOT_FOUND', `No session ${String(a.session)}`);
    for (const key of ['pinned', 'unread', 'title_locked'] as const) {
      if (typeof a[key] === 'boolean') (card as Record<string, unknown>)[key] = a[key];
    }
    // Like the real host, the card goes to *this* host's connections only.
    h.emit({ kind: 'session_changed', card: { ...card } });
    return { ...card };
  },
  fs_roots: (_a, h) => ({ roots: Object.keys(h.spec.fs ?? {}) }),
  fs_list: (a, h) => {
    const path = String(a.path ?? '');
    const node = walk(h.spec.fs, path);
    if (node?.kind !== 'dir') throw err('E_NOT_FOUND', `No folder ${path}`);
    const entries = Object.entries(node.children).map(([name, child]) => ({
      name,
      kind: child.kind === 'dir' ? 'dir' : 'file',
      size: child.kind === 'file' ? child.text.length : 0,
      mtime: child.kind === 'file' ? (child.mtime ?? 0) : 0,
    }));
    return { entries, more: false };
  },
  fs_stat: (a, h) => {
    const node = walk(h.spec.fs, String(a.path ?? ''));
    if (!node) throw err('E_NOT_FOUND', 'Not found');
    return { kind: node.kind, size: node.kind === 'file' ? node.text.length : 0, mtime: 0 };
  },
  fs_read: (a, h) => {
    const node = walk(h.spec.fs, String(a.path ?? ''));
    if (node?.kind !== 'file') throw err('E_NOT_FOUND', 'Not found');
    return { size: node.text.length, hash: `h${node.text.length}`, mtime: node.mtime ?? 0, truncated: false, text: node.text };
  },
  fs_search: () => ({ results: [], more: false }),
  // WP-A15: the whole `git` op family answers from the one mock repo.
  git_op: (a, h) => mockGitOp(a.cmd as { action: string }, h.spec.git ?? {}),
  // WP-A15 (Z-98): the refcounted watcher; the root `git_changed` events carry.
  git_watch: (a, h) => {
    const g = h.spec.git ?? {};
    h.watch(String(a.root ?? g.root ?? ''), a.on !== false);
    return { root: g.repo === false ? null : (g.root ?? a.root ?? null) };
  },
  cron_list: (_a, h) => ({ jobs: h.spec.cron ?? [], paused: false }),
  cron_status: () => ({ paused: false, concurrency: 1, jobs: [] }),
  ports_list: () => ({ ports: [], any: true, allowed: [] }),
  push_list: (_a, h) => ({ enabled: true, host_name: h.spec.name, pushes: [] }),
  feature_requests: () => ({ requests: [] }),
  devices_list: () => ({ devices: [] }),
};

const APP_BUILTIN: Record<string, MockOp> = {
  forward_list: () => [],
  local_requests: () => [],
  local_host: () => ({ installed: false }),
  push_save_as: () => null,
  push_open: () => null,
};

export class MockHost implements HostClient {
  readonly hostId: string;
  readonly conn;
  private readonly setConn;
  /** Repo roots a `git_watch{on:true}` asked for (off drops them, like the host's refcount). */
  readonly watched = new Set<string>();
  constructor(
    readonly spec: MockHostSpec,
    private readonly world: MockWorld,
  ) {
    this.hostId = spec.id;
    const [conn, setConn] = createSignal<HostConn>(spec.conn ?? { state: 'connected' });
    this.conn = conn;
    this.setConn = setConn;
  }

  /** Scenario/test control: flip the connection (e.g. go offline mid-capture). */
  setConnection(next: HostConn): void {
    this.setConn(next);
  }

  async retry(): Promise<void> {
    this.world.record({ host: this.hostId, kind: 'request', op: 'host_info' });
  }

  async request<T = unknown>(op: string, args: Record<string, unknown> = {}): Promise<T> {
    if (this.hostId === APP_HOST) return this.world.appRequest(op, args) as Promise<T>;
    const handler: MockOp | undefined = this.spec.ops?.[op] ?? BUILTIN[op];
    this.world.record({ host: this.hostId, kind: 'request', op, args, ...(handler === undefined ? { unhandled: true as const } : {}) });
    if (this.conn().state === 'offline') {
      throw err('E_TRANSPORT', this.conn().message ?? `Can't reach ${this.spec.name}`, this.conn().hint);
    }
    return (handler ? await handler(args, this) : null) as T;
  }

  openSession(req: Parameters<HostClient['openSession']>[0], sink: Parameters<HostClient['openSession']>[1]): SessionStream {
    this.world.record({ host: this.hostId, kind: 'open', op: `session_${req.kind}`, args: req });
    const timers: ReturnType<typeof setTimeout>[] = [];
    let open = true;
    const id = req.session ?? '';
    const session = req.session ? this.spec.sessions?.[req.session] : undefined;
    const key = `${this.hostId}/${id}`;
    const end = (error?: Err) => {
      if (!open) return;
      open = false;
      timers.forEach(clearTimeout);
      this.world.streams.get(key)?.delete(handle);
      sink.closed(error as never);
    };
    const handle = { push: (items: SessionEventDto[]) => open && sink.events(items), drop: (error?: Err) => end(error) };
    queueMicrotask(() => {
      if (!open) return;
      if (this.conn().state === 'offline') return end(err('E_TRANSPORT', `Can't reach ${this.spec.name}`));
      if (!session) return end(err('E_NOT_FOUND', `No session ${req.session ?? ''}`));
      const clock = this.world.clock(key);
      if (this.world.isDown(key)) return end(err('E_TRANSPORT', 'The host stopped answering'));
      (this.world.streams.get(key) ?? this.world.streams.set(key, new Set()).get(key)!).add(handle);
      const script = session.events ?? [];
      const speed = this.world.scenario.speed ?? 0;
      const elapsed = speed ? clock.elapsed() * speed : Infinity;
      const since = req.sinceSeq ?? 0;
      const dueEvents = script.flatMap((s) => ('dto' in s && s.at <= elapsed && s.dto.seq > since ? [s.dto] : []));
      if (since > 0) {
        sink.snapshot({ session_id: id, reset: false, seq: Math.max(since, ...dueEvents.map((e) => e.seq)), replay: dueEvents } as SessionSnapshot);
      } else {
        sink.snapshot(session.snapshot);
        if (dueEvents.length) sink.events(dueEvents);
      }
      script.forEach((step, i) => {
        if (!('drop' in step)) {
          if (step.at > elapsed) timers.push(setTimeout(() => open && sink.events([step.dto]), (step.at - elapsed) / speed));
          return;
        }
        const fire = () => {
          if (!open || this.world.fired(key, i)) return;
          this.world.markDown(key, (step.drop.down ?? 0) / (speed || 1));
          end(err(step.drop.code ?? 'E_TRANSPORT', step.drop.message ?? 'The connection dropped'));
        };
        // Speed 0 settles at once: a drop that has not fired yet takes the stream down right after the replay.
        if (step.at <= elapsed) queueMicrotask(fire);
        else timers.push(setTimeout(fire, (step.at - elapsed) / speed));
      });
    });
    return {
      cmd: async <T,>(cmd: RpcCommand): Promise<T> => {
        this.world.record({ host: this.hostId, kind: 'cmd', op: cmd.type, args: cmd });
        if (!open) throw err('E_TRANSPORT', 'The session stream closed');
        const handler = session?.cmd?.[cmd.type];
        if (handler) return (await handler(cmd, this)) as T;
        if (cmd.type === 'get_commands') return { commands: session?.snapshot.commands ?? [] } as T;
        if (cmd.type === 'get_entries' && session?.entries) {
          const since = typeof cmd.since === 'string' ? cmd.since : undefined;
          const all = session.entries.entries as Array<{ id?: string }>;
          const at = since ? all.findIndex((e) => e.id === since) : -1;
          return { entries: since ? all.slice(at + 1) : all, leafId: session.entries.leafId } as T;
        }
        return {} as T;
      },
      close: () => {
        open = false;
        timers.forEach(clearTimeout);
        this.world.streams.get(key)?.delete(handle);
      },
    };
  }

  openEvents(sink: (event: HubEvent) => void): EventsStream {
    this.world.record({ host: this.hostId, kind: 'open', op: 'events_open' });
    this.world.eventSinks.add(sink);
    const hostSinks = this.world.hostSinks.get(this.hostId) ?? this.world.hostSinks.set(this.hostId, new Set()).get(this.hostId)!;
    hostSinks.add(sink);
    return {
      // §3.14: the mock clears the focused card's `unread` like the host does (Z-82).
      focus: (session) => {
        this.world.record({ host: this.hostId, kind: 'request', op: 'session_focus', args: { session } });
        const card = session ? this.spec.cards?.find((c) => c.session_id === session) : undefined;
        if (card?.unread) {
          card.unread = false;
          this.emit({ kind: 'session_changed', card: { ...card } });
        }
      },
      close: () => {
        this.world.eventSinks.delete(sink);
        hostSinks.delete(sink);
      },
    };
  }

  openTerm(req: Parameters<HostClient['openTerm']>[0]): TermStream {
    this.world.record({ host: this.hostId, kind: 'open', op: 'term_open', args: req });
    return { write() {}, resize() {}, close() {} };
  }

  openLsp(req: Parameters<HostClient['openLsp']>[0]): LspStream {
    this.world.record({ host: this.hostId, kind: 'open', op: 'lsp_open', args: req });
    return { send() {}, close() {} };
  }

  /** `git_watch`: remembers the watched roots (off drops, like the host's refcount). */
  watch(root: string, on: boolean): void {
    if (!root) return;
    if (on) this.watched.add(root);
    else this.watched.delete(root);
  }

  /** Test control: one host-side `git_changed` for a watched root (WP-A15, Z-98). */
  gitChanged(root = [...this.watched][0]): void {
    if (root) this.emit({ kind: 'git_changed', root });
  }

  /** Sends a hub event to this host's `openEvents` subscribers only (like the real host). */
  emit(event: HubEvent): void {
    for (const sink of this.world.hostSinks.get(this.hostId) ?? []) sink(event);
  }
}

type StreamHandle = { push(items: SessionEventDto[]): void; drop(error?: Err): void };

export class MockWorld {
  readonly log: MockLogEntry[] = [];
  readonly eventSinks = new Set<(event: HubEvent) => void>();
  /** Per-host event sinks: `MockHost.emit` reaches only its own host's subscribers. */
  readonly hostSinks = new Map<string, Set<(event: HubEvent) => void>>();
  /** Open session streams by `host/session` (tests push events / drop them). */
  readonly streams = new Map<string, Set<StreamHandle>>();
  private readonly clocks = new Map<string, number>();
  private readonly down = new Map<string, number>();
  private readonly firedSteps = new Set<string>();
  private readonly clients = new Map<string, MockHost>();
  private readonly t0 = Date.now();

  constructor(readonly scenario: MockScenario) {
    for (const spec of scenario.hosts) this.clients.set(spec.id, new MockHost(spec, this));
    this.clients.set(APP_HOST, new MockHost({ id: APP_HOST, name: 'App' }, this));
  }

  /** The `HostClientFactory` (`setHostClientFactory(world.client)` / `window.__UNIPI_HOST__`). */
  readonly client = (hostId: string): MockHost => {
    let c = this.clients.get(hostId);
    if (!c) {
      // An unknown id (a stale selection, a forgotten PC) is an unreachable host.
      c = new MockHost({ id: hostId, name: hostId, conn: { state: 'offline', message: 'Unknown PC' } }, this);
      this.clients.set(hostId, c);
    }
    return c;
  };

  record(entry: Omit<MockLogEntry, 'at'>): void {
    const full = { ...entry, at: Date.now() - this.t0 };
    this.log.push(full);
    if (typeof window !== 'undefined' && window.__UNIPI_HOST__ === this.client) (window.__UNIPI_MOCK_LOG__ ??= []).push(full);
  }

  /** The virtual clock of a session script (starts at its first open). */
  clock(key: string): { elapsed(): number } {
    if (!this.clocks.has(key)) this.clocks.set(key, Date.now());
    const t0 = this.clocks.get(key)!;
    return { elapsed: () => Date.now() - t0 };
  }

  /** A script drop: the session's host is unreachable for `ms` real milliseconds (`Infinity` = stays down). */
  markDown(key: string, ms: number): void {
    this.down.set(key, Date.now() + ms);
  }

  isDown(key: string): boolean {
    return Date.now() < (this.down.get(key) ?? 0);
  }

  /** Marks script step `index` of a session fired; true when it already had. */
  fired(key: string, index: number): boolean {
    const id = `${key}#${index}`;
    if (this.firedSteps.has(id)) return true;
    this.firedSteps.add(id);
    return false;
  }

  /** Test control: the session's host answers again. */
  restore(hostId: string, sessionId: string): void {
    this.down.delete(`${hostId}/${sessionId}`);
  }

  /** Test control: pushes events to every open stream of a session. */
  push(hostId: string, sessionId: string, items: SessionEventDto[]): void {
    for (const s of this.streams.get(`${hostId}/${sessionId}`) ?? []) s.push(items);
  }

  /** Test control: drops every open stream of a session (`E_TRANSPORT` by default). */
  drop(hostId: string, sessionId: string, error: Err = err('E_TRANSPORT', 'The connection dropped')): void {
    for (const s of [...(this.streams.get(`${hostId}/${sessionId}`) ?? [])]) s.drop(error);
  }

  /** Pushes a hub event to every `openEvents` subscriber. */
  emit(event: HubEvent): void {
    for (const sink of this.eventSinks) sink(event);
  }

  async appRequest(op: string, args: Record<string, unknown>): Promise<unknown> {
    const handler: MockOp | undefined = this.scenario.appOps?.[op] ?? APP_BUILTIN[op];
    if (op === 'list_hosts' && !handler) {
      this.record({ host: APP_HOST, kind: 'request', op, args });
      return this.scenario.hosts.map((h, i) => ({
        id: h.id,
        name: h.name,
        ticket: `mock-ticket-${h.id}`,
        paired_at: 1_790_000_000_000 - i * 86_400_000,
        last_path: h.transport === 'local' ? 'local' : 'direct',
        ...(h.transport ? { transport: h.transport } : {}),
      }));
    }
    // Legacy `call(op, {hostId, …})` wrappers: route to that host.
    if (!handler && typeof args.hostId === 'string' && args.hostId !== APP_HOST) {
      const { hostId, ...rest } = args;
      return this.client(hostId as string).request(op, rest);
    }
    this.record({ host: APP_HOST, kind: 'request', op, args, ...(handler === undefined ? { unhandled: true as const } : {}) });
    return handler ? handler(args, this.client(APP_HOST)) : null;
  }
}

/** A mock host world for a scenario: `world.client(hostId)` is the `HostClientFactory`. */
export function createMockHost(scenario: MockScenario): MockWorld {
  return new MockWorld(scenario);
}
