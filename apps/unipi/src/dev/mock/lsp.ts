/**
 * The dev/mock language server (WP-A21): a scripted JSON-RPC peer behind
 * `openLsp` — it answers the requests the capture scenarios exercise
 * (hover, completion, signature help, definition, references, rename,
 * formatting), pushes one diagnostic batch per `didOpen`, and models
 * the take-over (`lsp.taken` → `closed(E_LSP_TAKEN)` after the
 * handshake, the `editor-conflict` scenario).
 */
import type { AppError } from '@unipi/protocol';
import type { LspSink, LspStream } from '../../lib/host/client';

export type MockLspSpec = {
  /** Chip label / `lsp_open` `server` field. */
  name?: string;
  /** Languages a server exists for (default: everything asked). */
  languages?: string[];
  /** Diagnostics pushed after `textDocument/didOpen` (per uri suffix). */
  diagnostics?: { severity?: number; message: string }[];
  /** Push the `E_LSP_TAKEN` close shortly after the handshake. */
  taken?: boolean;
};

type Json = Record<string, unknown>;

const CAPABILITIES = {
  capabilities: {
    hoverProvider: true,
    completionProvider: { triggerCharacters: ['.', ':', '"'] },
    signatureHelpProvider: { triggerCharacters: ['(', ','] },
    definitionProvider: true,
    declarationProvider: true,
    implementationProvider: true,
    typeDefinitionProvider: true,
    referencesProvider: true,
    renameProvider: { prepareProvider: false },
    documentFormattingProvider: true,
    textDocumentSync: { openClose: true, change: 2 },
  },
  serverInfo: { name: 'mock-lsp', version: '1.0' },
};

/** `lsp_servers` (the Settings › Language servers roster). */
export function mockLspServers(spec: MockLspSpec | undefined, root: string | undefined): unknown {
  const name = spec?.name ?? 'mock-lsp';
  const languages = spec?.languages ?? ['typescript', 'rust', 'python'];
  return {
    root: root ?? null,
    servers: [
      ...languages.map((language) => ({
        language,
        name: `${name} (${language})`,
        available: true,
        command: 'mock-lsp --stdio',
      })),
      {
        language: 'cobol',
        name: 'cobol-language-server',
        available: false,
        install_hint: 'Install a COBOL language server to enable it',
        command: 'cobol-ls --stdio',
      },
    ],
  };
}

export function openMockLsp(
  spec: MockLspSpec | undefined,
  sink: LspSink,
  world?: { record(entry: { host: string; kind: string; op: string; args?: unknown }): void; hostId?: string },
): LspStream {
  const name = spec?.name ?? 'mock-lsp';
  const docs = new Map<string, number>();
  let nextId = 0;
  let closed = false;

  const respond = (message: Json) => {
    sink.message(JSON.stringify(message));
  };

  const takenTimer =
    spec?.taken === true
      ? setTimeout(() => {
          if (!closed) sink.closed({ code: 'E_LSP_TAKEN', message: 'language features moved to another device — attach again to take them back', retryable: false } as AppError);
        }, 1200)
      : undefined;

  queueMicrotask(() => {
    sink.attached?.({ server: name, attached: false, initialize_result: null });
  });

  const diagnosticBatch = (uri: string) => {
    const list = spec?.diagnostics ?? [
      { severity: 2, message: 'mock warning — the dev server always flags line 1' },
    ];
    respond({
      jsonrpc: '2.0',
      method: 'textDocument/publishDiagnostics',
      params: {
        uri,
        diagnostics: list.map((diag, i) => ({
          range: { start: { line: i, character: 0 }, end: { line: i, character: 6 } },
          severity: diag.severity ?? 2,
          message: diag.message,
          source: name,
        })),
      },
    });
  };

  const handle = (message: Json) => {
    const id = message.id;
    const params = (message.params ?? {}) as Json;
    const textDocument = (params.textDocument ?? {}) as Json;
    const uri = (textDocument.uri ?? params.uri ?? 'file:///dev/null') as string;
    switch (message.method) {
      case 'initialize':
        respond({ jsonrpc: '2.0', id, result: CAPABILITIES });
        return;
      case 'initialized':
        return;
      case 'shutdown':
        respond({ jsonrpc: '2.0', id, result: null });
        return;
      case 'exit':
        return;
      case 'textDocument/didOpen':
        docs.set(uri, Number(textDocument.version ?? 1));
        setTimeout(() => !closed && diagnosticBatch(uri), 30);
        return;
      case 'textDocument/didChange':
        docs.set(uri, Number(params.textDocument ? (textDocument.version ?? 1) : 1));
        return;
      case 'textDocument/didClose':
        docs.delete(uri);
        return;
      case 'textDocument/hover':
        respond({
          jsonrpc: '2.0',
          id,
          result: {
            contents: { kind: 'markdown', value: '**mock** hover\n\n`symbol` — answered by the dev server (WP-A21).' },
            range: { start: { line: 0, character: 0 }, end: { line: 0, character: 4 } },
          },
        });
        return;
      case 'textDocument/completion':
        respond({
          jsonrpc: '2.0',
          id,
          result: {
            isIncomplete: false,
            items: [
              { label: 'mockComplete', kind: 6, detail: 'mock-lsp', documentation: 'a completion from the dev server' },
              { label: 'mockHelper()', kind: 3, detail: 'mock-lsp' },
              { label: 'MOCK_CONST', kind: 21, detail: 'mock-lsp' },
            ],
          },
        });
        return;
      case 'textDocument/signatureHelp':
        respond({
          jsonrpc: '2.0',
          id,
          result: {
            signatures: [
              { label: 'mockHelper(input: string, count: number): void', documentation: 'mock signature', parameters: [{ label: 'input: string' }, { label: 'count: number' }] },
            ],
            activeSignature: 0,
            activeParameter: 0,
          },
        });
        return;
      case 'textDocument/definition':
      case 'textDocument/declaration':
      case 'textDocument/typeDefinition':
      case 'textDocument/implementation':
        respond({
          jsonrpc: '2.0',
          id,
          result: {
            uri: uri.replace(/[^/]+$/, 'other.ts'),
            range: { start: { line: 2, character: 4 }, end: { line: 2, character: 14 } },
          },
        });
        return;
      case 'textDocument/references':
        respond({
          jsonrpc: '2.0',
          id,
          result: [
            { uri, range: { start: { line: 0, character: 8 }, end: { line: 0, character: 13 } } },
            { uri: uri.replace(/[^/]+$/, 'other.ts'), range: { start: { line: 4, character: 2 }, end: { line: 4, character: 7 } } },
          ],
        });
        return;
      case 'textDocument/rename':
        respond({
          jsonrpc: '2.0',
          id,
          result: {
            changes: {
              [uri]: [
                {
                  range: { start: { line: 0, character: 0 }, end: { line: 0, character: 4 } },
                  newText: String(params.newName ?? 'renamed'),
                },
              ],
            },
          },
        });
        return;
      case 'textDocument/formatting':
        respond({
          jsonrpc: '2.0',
          id,
          result: [
            {
              range: { start: { line: 0, character: 0 }, end: { line: 9999, character: 0 } },
              newText: '// formatted by mock-lsp\n',
            },
          ],
        });
        return;
      default:
        if (id !== undefined && id !== null) respond({ jsonrpc: '2.0', id, result: null });
    }
  };

  return {
    send(json: unknown) {
      if (closed) return;
      const message = (typeof json === 'string' ? JSON.parse(json) : json) as Json;
      world?.record({ host: world.hostId ?? 'this-pc', kind: 'lsp', op: String(message.method ?? `response:${nextId++}`), args: message });
      // Async, like a real server would be.
      setTimeout(() => !closed && handle(message), 0);
    },
    close() {
      closed = true;
      if (takenTimer) clearTimeout(takenTimer);
    },
  };
}
