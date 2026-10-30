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

/** A recorded session: snapshot + `{at (ms), dto}` script replayed on a virtual clock. */
export type MockSession = { snapshot: SessionSnapshot; events?: Array<{ at: number; dto: SessionEventDto }> };

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
  git?: unknown;
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

const BUILTIN: Record<string, MockOp> = {
  host_info: (_a, h) => ({ name: h.spec.name, version: '2.0.0', proto: '2.0.0', tools: ['oino', 'unipi-kanboard'] }),
  sessions_list: (_a, h) => ({ sessions: h.spec.cards ?? [] }),
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
  git_op: (_a, h) => h.spec.git ?? { branch: 'main', ahead: 0, behind: 0, files: [] },
  cron_list: (_a, h) => ({ jobs: h.spec.cron ?? [], paused: false }),
  cron_status: () => ({ paused: false, concurrency: 1, jobs: [] }),
  ports_list: () => ({ ports: [], any: true, allowed: [] }),
  push_list: (_a, h) => ({ enabled: true, host_name: h.spec.name, pushes: [] }),
  feature_requests: () => ({ requests: [] }),
  devices_list: () => ({ devices: [] }),
};

const APP_BUILTIN: Record<string, MockOp> = {
  forward_list: () => [],
  local_host: () => ({ installed: false }),
  push_save_as: () => null,
  push_open: () => null,
};

export class MockHost implements HostClient {
  readonly hostId: string;
  readonly conn;
  private readonly setConn;
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
    const session = req.session ? this.spec.sessions?.[req.session] : undefined;
    queueMicrotask(() => {
      if (!open) return;
      if (this.conn().state === 'offline') return sink.closed(err('E_TRANSPORT', `Can't reach ${this.spec.name}`) as never);
      if (!session) return sink.closed(err('E_NOT_FOUND', `No session ${req.session ?? ''}`) as never);
      sink.snapshot(session.snapshot);
      const script = session.events ?? [];
      const speed = this.world.scenario.speed ?? 0;
      if (!speed) {
        if (script.length) sink.events(script.map((s) => s.dto));
        return;
      }
      for (const step of script) timers.push(setTimeout(() => open && sink.events([step.dto]), step.at / speed));
    });
    return {
      cmd: async <T,>(cmd: RpcCommand): Promise<T> => {
        this.world.record({ host: this.hostId, kind: 'cmd', op: cmd.type, args: cmd });
        return { ok: true } as T;
      },
      close: () => {
        open = false;
        timers.forEach(clearTimeout);
      },
    };
  }

  openEvents(sink: (event: HubEvent) => void): Closable {
    this.world.record({ host: this.hostId, kind: 'open', op: 'events_open' });
    this.world.eventSinks.add(sink);
    return { close: () => this.world.eventSinks.delete(sink) };
  }

  openTerm(req: Parameters<HostClient['openTerm']>[0]): TermStream {
    this.world.record({ host: this.hostId, kind: 'open', op: 'term_open', args: req });
    return { write() {}, resize() {}, close() {} };
  }

  openLsp(req: Parameters<HostClient['openLsp']>[0]): LspStream {
    this.world.record({ host: this.hostId, kind: 'open', op: 'lsp_open', args: req });
    return { send() {}, close() {} };
  }
}

export class MockWorld {
  readonly log: MockLogEntry[] = [];
  readonly eventSinks = new Set<(event: HubEvent) => void>();
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
