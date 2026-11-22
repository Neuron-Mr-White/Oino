/**
 * The language server pool (WP-A21, app spec §3.12): one `LSPClient`
 * per `(hostId, root, language)` shared by every editor tab of that
 * root — `acquire` attaches the stream (the host spawns the server on
 * demand), `release` drops the last tab's reference and detaches (the
 * host keeps the server warm for `lsp.idle_minutes`).
 *
 * Graceful degradation is the spec: a host without the binary (or an
 * older host/app build) just means plain editing — `acquire` resolves
 * `null`, the status chip shows `off`, nothing else changes.
 */
import { createSignal } from 'solid-js';
import { LSPClient, languageServerExtensions } from '@codemirror/lsp-client';
import type { Extension } from '@codemirror/state';
import type { AppError } from '@unipi/protocol';
import type { HostClient } from '../host/client';
import { UniPiWorkspace, languageForPath, type WorkspaceHooks } from './workspace';
import { streamTransport, type DispatchTransport } from './transport';
import { fileUri } from './uris';
import { recordDiagnostics } from './diagnostics';

export type LspStatus = 'off' | 'connecting' | 'ready' | 'degraded' | 'taken';

export interface PooledLsp {
  readonly hostId: string;
  readonly root: string;
  readonly language: string;
  readonly serverName: string;
  readonly status: () => LspStatus;
  readonly client: LSPClient | null;
  /** The extension one editor tab installs (`null` while unavailable). */
  plugin(uri: string, languageId: string): Extension | null;
  /** Drops one tab's reference; the last one detaches the stream. */
  release(): void;
}

interface Entry {
  key: string;
  host: HostClient;
  root: string;
  language: string;
  refs: number;
  client: LSPClient | null;
  /** Settled once the client initialized (or failed). */
  ready: Promise<LSPClient | null>;
  status: () => LspStatus;
  setStatus: (next: LspStatus) => void;
  serverName: string;
  closed: boolean;
}

const pool = new Map<string, Entry>();

function keyOf(hostId: string, root: string, language: string): string {
  return `${hostId}${root}${language}`;
}



/**
 * Acquires (or shares) the `(root, language)` connection for `host`.
 * `hooks.displayFile` is the editor tabs' "open path → EditorView".
 *
 * Returns `null` when the server can't run (not installed, unsupported
 * host, refused) — the caller edits without LSP.
 */
export async function acquireLsp(
  host: HostClient,
  root: string,
  path: string,
  hooks: WorkspaceHooks,
): Promise<PooledLsp | null> {
  const language = languageForPath(path);
  const key = keyOf(host.hostId, root, language);
  const existing = pool.get(key);
  if (existing) {
    existing.refs++;
    await existing.ready;
    if (existing.closed) {
      pool.delete(key);
      return acquireLsp(host, root, path, hooks);
    }
    if (!existing.client) {
      existing.refs--;
      return null;
    }
    return pooled(existing);
  }

  const [status, setStatus] = createSignal<LspStatus>('connecting');
  const entry: Entry = {
    key,
    host,
    root,
    language,
    refs: 1,
    client: null,
    ready: Promise.resolve(null),
    status,
    setStatus,
    serverName: language,
    closed: false,
  };
  pool.set(key, entry);
  entry.ready = connect(entry, hooks);
  const client = await entry.ready;
  if (!client) {
    pool.delete(key);
    setStatus('off');
    return null;
  }
  return pooled(entry);
}

async function connect(entry: Entry, hooks: WorkspaceHooks): Promise<LSPClient | null> {
  let resolveAttach: ((data: { server: string }) => void) | undefined;
  const attached = new Promise<{ server: string }>((resolve) => {
    resolveAttach = resolve;
  });
  // The transport exists before the stream so a synchronous `message`
  // (the mock's) can't be lost — it queues until `connect` subscribes.
  let stream!: { send(json: unknown): void; close(): void };
  const inner: DispatchTransport = streamTransport({ send: (json) => stream.send(json) });
  let closedError: AppError | undefined;
  stream = entry.host.openLsp(
    { root: entry.root, language: entry.language },
    {
      message(json: unknown) {
        inner.dispatch(typeof json === 'string' ? json : JSON.stringify(json));
      },
      attached(data: unknown) {
        const d = data as { server?: string } | undefined;
        entry.serverName = d?.server ?? entry.language;
        resolveAttach?.({ server: entry.serverName });
      },
      closed(error?: AppError) {
        closedError = error;
        entry.closed = true;
        entry.setStatus(error?.code === 'E_LSP_TAKEN' ? 'taken' : 'degraded');
      },
    },
  );

  const client = new LSPClient({
    rootUri: fileUri(entry.root),
    workspace: (c) => new UniPiWorkspace(c, entry.host.hostId, entry.root, hooks),
    // The tree's diagnostic badges come from the same notifications the
    // plugin consumes — returning false lets the built-in handler run.
    notificationHandlers: {
      'textDocument/publishDiagnostics': (_client, params: { uri: string; diagnostics: { severity?: number }[] }) => {
        recordDiagnostics(params.uri, params.diagnostics ?? []);
        return false;
      },
    },
    // The whole feature set (hover, completion, signature help,
    // go-to-*, references panel, rename, format, diagnostics) — the
    // per-editor `client.plugin` extension picks them up via the client.
    extensions: languageServerExtensions(),
  });
  // A sync `closed` (mock, unsupported host) still resolves.
  if (entry.closed) {
    stream.close();
    return null;
  }
  try {
    client.connect(inner);
    await client.initializing;
  } catch {
    entry.setStatus('degraded');
    stream.close();
    return null;
  }
  if (closedError || entry.closed) {
    stream.close();
    return null;
  }
  const attach = await Promise.race([
    attached,
    new Promise<null>((resolve) => setTimeout(() => resolve(null), 10_000)),
  ]);
  if (attach) entry.serverName = attach.server;
  entry.client = client;
  entry.setStatus('ready');
  return client;
}

function pooled(entry: Entry): PooledLsp {
  return {
    hostId: entry.host.hostId,
    root: entry.root,
    language: entry.language,
    serverName: entry.serverName,
    status: entry.status,
    client: entry.client,
    plugin(uri, languageId) {
      return entry.client?.plugin(uri, languageId) ?? null;
    },
    release() {
      entry.refs--;
      if (entry.refs <= 0) {
        pool.delete(entry.key);
        entry.closed = true;
        entry.client?.disconnect();
      }
    },
  };
}

/** Detaches every pooled connection (app close / host switch). */
export function closeAllLsp(): void {
  for (const entry of [...pool.values()]) {
    entry.closed = true;
    entry.client?.disconnect();
  }
  pool.clear();
}

/** Test introspection. */
export function lspPoolSize(): number {
  return pool.size;
}
