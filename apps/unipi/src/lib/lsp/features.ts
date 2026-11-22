/**
 * The per-editor LSP extension bundle (WP-A21): hover tooltips,
 * completion, signature help, go-to-definition/…, find references,
 * rename, formatting and diagnostics — the whole `languageServerExtensions`
 * set, plus the pooled client's per-file plugin. Everything degrades: a
 * `null` pooled connection yields no extensions, plain editing.
 */
import type { Extension } from '@codemirror/state';
import type { LSPClient } from '@codemirror/lsp-client';
import type { PooledLsp } from './pool';
import { fileUri } from './uris';
import { languageIdForPath } from './workspace';

/**
 * The extensions one editor for `path` gets from a pooled connection
 * (`null` when the pool declined — plain editing, nothing breaks). The
 * hover/completion/signature/definition/references/rename/format/
 * diagnostics features themselves live in the client's
 * `languageServerExtensions()` config (installed once at `connect`); a
 * tab only needs its `client.plugin(uri, languageId)`.
 */
export function lspExtensions(pooled: PooledLsp | null, path: string): Extension[] {
  const client: LSPClient | null = pooled?.client ?? null;
  if (!client) return [];
  const uri = fileUri(path);
  const languageId = languageIdForPath(path);
  return [client.plugin(uri, languageId)];
}
