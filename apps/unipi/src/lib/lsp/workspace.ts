/**
 * The workspace the pool's `LSPClient` runs against (WP-A21, app spec
 * §3.12 "one workspace per project root"). Unlike lsp-client's default,
 * this one knows about the whole repo:
 *
 * - `openFile`/`closeFile` are driven by editor tabs. Closing a tab does
 *   **not** `didClose` the document — the server keeps it warm for the
 *   next open (the spec's own words); the attach ending is what retires
 *   the server's view.
 * - `requestFile` loads files the server asks about (`workspace/symbol`,
 *   rename references to closed files, …) through `fs_read` and opens
 *   them on the server.
 * - `displayFile` opens the target file in a real editor tab and hands
 *   its view back — that's how go-to-definition / find-references jump
 *   across files.
 * - `updateFile` applies server-side edits (rename, formatting, code
 *   actions) to whatever tab holds the file.
 */
import { EditorView } from '@codemirror/view';
import { EditorState, Text } from '@codemirror/state';
import type { TransactionSpec } from '@codemirror/state';
import { Workspace, LSPPlugin, type LSPClient, type WorkspaceFile } from '@codemirror/lsp-client';
import { fsRead, fsWrite } from '../fs';
import { fileUri } from './uris';

export interface WorkspaceHooks {
  /** `displayFile`: open the file in the editor tabs; resolves the view
   *  once mounted (null when the file can't be shown). */
  displayFile(path: string): Promise<EditorView | null>;
  /** Server-side edits that couldn't be applied (a closed file whose
   *  disk write hit a conflict) — the UI surfaces them. */
  onEditConflict?(paths: string[]): void;
}

class OpenFile implements WorkspaceFile {
  version = 0;
  doc: Text = Text.empty;
  private views: EditorView[] = [];

  constructor(
    readonly uri: string,
    readonly languageId: string,
  ) {}

  getView(main?: EditorView): EditorView | null {
    if (main && this.views.includes(main)) return main;
    return this.views[0] ?? null;
  }

  attach(view: EditorView): void {
    if (!this.views.includes(view)) this.views.push(view);
    this.doc = view.state.doc;
  }

  detach(view: EditorView): void {
    const index = this.views.indexOf(view);
    if (index >= 0) this.views.splice(index, 1);
  }
}

export class UniPiWorkspace extends Workspace {
  files: OpenFile[] = [];

  constructor(
    client: LSPClient,
    readonly hostId: string,
    readonly root: string,
    private hooks: WorkspaceHooks,
  ) {
    super(client);
  }

  override getFile(uri: string): OpenFile | null {
    return this.files.find((f) => f.uri === uri) ?? null;
  }

  /** The file for an app path, creating+loading it through `fs_read`. */
  async ensureFile(path: string, languageId: string): Promise<OpenFile | null> {
    const uri = fileUri(path);
    const existing = this.getFile(uri);
    if (existing) return existing;
    const read = await fsRead(this.hostId, path);
    if (!read.ok) return null;
    const file = new OpenFile(uri, languageId);
    file.doc = Text.of(read.value.text.split('\n'));
    this.files.push(file);
    this.client.didOpen(file);
    return file;
  }

  override openFile(uri: string, languageId: string, view: EditorView): void {
    let file = this.getFile(uri);
    if (!file) {
      file = new OpenFile(uri, languageId);
      this.files.push(file);
      file.attach(view);
      this.client.didOpen(file);
      return;
    }
    file.attach(view);
  }

  override closeFile(uri: string, view: EditorView): void {
    // The spec: "closing a tab does not close the LSP document (the
    // server keeps it warm for the next open)". The file stays in
    // `this.files`; the view detaches so `updateFile` won't target it.
    this.getFile(uri)?.detach(view);
  }

  /** Every open file's unsynced changes (the client's `sync` calls
   *  this). The per-view `LSPPlugin` already accumulates the incremental
   *  change sets — we flush them and mark the file's version up. */
  override syncFiles() {
    const updates = [];
    for (const file of this.files) {
      const view = file.getView();
      if (!view) continue;
      const plugin = LSPPlugin.get(view);
      if (!plugin) continue;
      const changes = plugin.unsyncedChanges;
      if (changes.empty) continue;
      updates.push({ changes, file, prevDoc: file.doc });
      file.doc = view.state.doc;
      file.version++;
      plugin.clear();
    }
    return updates;
  }

  override async requestFile(uri: string): Promise<WorkspaceFile | null> {
    const existing = this.getFile(uri);
    if (existing) return existing;
    const read = await fsRead(this.hostId, uriToPath(uri));
    if (!read.ok) return null;
    const file = new OpenFile(uri, languageIdForPath(uriToPath(uri)));
    file.doc = Text.of(read.value.text.split('\n'));
    this.files.push(file);
    this.client.didOpen(file);
    return file;
  }

  override updateFile(uri: string, update: TransactionSpec): void {
    const file = this.getFile(uri);
    const view = file?.getView();
    if (view) {
      view.dispatch(update);
      return;
    }
    // A server-side edit (rename, code action) to a file no tab holds:
    // apply it headlessly to the tracked doc, then write it back through
    // fs_write — a base-hash conflict lands in `onEditConflict`, listed
    // for the user like the spec's "conflicts listed".
    if (!file) {
      void this.applyToClosedFile(uri, update);
      return;
    }
    const next = EditorState.create({ doc: file.doc }).update(update).state.doc;
    file.doc = next;
    file.version++;
    void this.writeBack(uri, next.toString());
  }

  /** Loads an untracked closed file, applies the update, writes it back. */
  private async applyToClosedFile(uri: string, update: TransactionSpec): Promise<void> {
    const path = uriToPath(uri);
    const read = await fsRead(this.hostId, path);
    if (!read.ok) {
      this.hooks.onEditConflict?.([path]);
      return;
    }
    const next = EditorState.create({ doc: Text.of(read.value.text.split('\n')) }).update(update).state.doc;
    const write = await fsWrite(this.hostId, path, next.toString(), read.value.hash, false);
    if (!write.ok) this.hooks.onEditConflict?.([path]);
  }

  private async writeBack(uri: string, text: string): Promise<void> {
    const path = uriToPath(uri);
    const stat = await fsRead(this.hostId, path);
    const baseHash = stat.ok ? stat.value.hash : undefined;
    const write = await fsWrite(this.hostId, path, text, baseHash, false);
    if (!write.ok) this.hooks.onEditConflict?.([path]);
  }

  override displayFile(uri: string): Promise<EditorView | null> {
    return this.hooks.displayFile(uriToPath(uri));
  }
}

export function uriToPath(uri: string): string {
  return uri.startsWith('file://') ? decodeURIComponent(uri.slice('file://'.length)) : uri;
}

/** The LSP `languageId` for a path (subset the UI actually opens). */
export function languageIdForPath(path: string): string {
  const name = path.split('/').pop() ?? '';
  const ext = name.includes('.') ? name.split('.').pop()!.toLowerCase() : '';
  switch (ext) {
    case 'rs': return 'rust';
    case 'ts': case 'tsx': return 'typescript';
    case 'js': case 'jsx': case 'mjs': case 'cjs': return 'javascript';
    case 'py': return 'python';
    case 'go': return 'go';
    case 'json': return 'json';
    case 'toml': return 'toml';
    case 'yaml': case 'yml': return 'yaml';
    case 'md': return 'markdown';
    case 'html': return 'html';
    case 'css': return 'css';
    case 'zig': return 'zig';
    case 'c': case 'h': return 'c';
    case 'cc': case 'cpp': case 'cxx': case 'hpp': return 'cpp';
    case 'java': return 'java';
    case 'lua': return 'lua';
    case 'sh': case 'bash': return 'shellscript';
    default: return 'plaintext';
  }
}

/** The pool language key for a path — the `lsp.servers.<language>` key
 *  the host resolves a server from (`typescript` for ts/tsx, …). */
export function languageForPath(path: string): string {
  const id = languageIdForPath(path);
  return id === 'typescript' || id === 'javascript' ? 'typescript' : id;
}
