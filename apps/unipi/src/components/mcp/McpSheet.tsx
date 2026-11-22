/**
 * The Add / Edit sheet of Settings › MCP (OIN-101 WI-3, SPEC §2): a dialog on the desktop, a full-screen sheet on
 * the phone (`SheetFrame`, one `full` detent). Two tabs over ONE draft:
 *
 *  - Form: Name, Scope (Global / this project), Type (HTTP | Command), URL or Command + Args, Headers / Env as
 *    key–value rows, each with a Secret toggle (on by default for authorization|token|key|secret keys).
 *  - Paste JSON: one entry, `{name: entry}`, or a whole `{mcpServers: {…}}` (Claude Code / Cursor / pi). It is
 *    parsed and PREVIEWED in the Form tab — nothing is saved from the paste box itself.
 *
 * Secrets: a stored one is `${NAME}` in the file and shows here as •••• + [Replace]; a new value goes to the PC's
 * secret store on save (the entry sent to the host only ever holds the `${NAME}` reference). After Save the sheet
 * shows the resulting status of each saved server (connected, or the full error) before it closes.
 */
import { createMemo, createResource, createSignal, For, Index, Show } from 'solid-js';
import { createStore, produce } from 'solid-js/store';
import { CircleAlert, LibraryBig, Lock, Plus, X } from 'lucide-solid';
import { SheetFrame } from '../phone/Sheet';
import type { CatalogItem, SaveReply } from './api';
import {
  buildEntry, draftFromCatalog, draftFromEntry, draftsFromPaste, emptyDraft, isSecretKey, missingRequired, newRow, parsePaste, projectLabel, serverStatus, validateDraft,
  type Draft, type KvRow, type McpScope, type McpServer, type McpType,
} from './model';
import './mcp.css';

export type SheetMode = { kind: 'add' } | { kind: 'edit'; server: McpServer } | { kind: 'override'; server: McpServer };

export type SheetProject = { cwd: string; /** project files can be written (trusted, host understands projects) */ writable: boolean; why?: string };

export type McpSheetProps = {
  mode: SheetMode;
  /** The page's project, when scoped to one. */
  project?: SheetProject;
  /** Names that already exist in `scope` (an add must not silently replace one). */
  taken: (scope: McpScope) => string[];
  /** Live list: the saved view follows each server's status as it connects. */
  servers: () => McpServer[];
  onSave: (scope: McpScope, entries: Record<string, Record<string, unknown>>, secrets: Record<string, string>) => Promise<SaveReply>;
  onClose: () => void;
  onCatalog?: (query: string) => Promise<CatalogItem[]>;
};

const PASTE_HINT = '{\n  "mcpServers": {\n    "linear": { "url": "https://mcp.linear.app/mcp", "headers": { "Authorization": "Bearer …" } }\n  }\n}';

function initialDraft(mode: SheetMode, project?: SheetProject): Draft {
  if (mode.kind === 'add') return emptyDraft(project?.writable ? 'project' : 'global');
  const s = mode.server;
  const scope: McpScope = mode.kind === 'override' || s.badge !== 'global' ? 'project' : 'global';
  return draftFromEntry(s.name, s.entry ?? {}, scope);
}

export function McpSheet(props: McpSheetProps) {
  const [drafts, setDrafts] = createStore<Draft[]>([initialDraft(props.mode, props.project)]);
  const [idx, setIdx] = createSignal(0);
  const [tab, setTab] = createSignal<'form' | 'paste' | 'catalog'>('form');
  const [query, setQuery] = createSignal('');
  const [catalog] = createResource(() => tab() === 'catalog' ? query() : undefined,
    (q) => props.onCatalog?.(q) ?? Promise.resolve([]));
  const useCatalog = (item: CatalogItem) => {
    setDrafts([draftFromCatalog(item.name, item.entry, d().scope)]);
    setIdx(0);
    setTab('form');
  };
  const [paste, setPaste] = createSignal('');
  const [pasteError, setPasteError] = createSignal<string | undefined>();
  const [imported, setImported] = createSignal<string | undefined>();
  const [busy, setBusy] = createSignal(false);
  const [failure, setFailure] = createSignal<string | undefined>();
  const [saved, setSaved] = createSignal<{ names: string[]; notice?: string; message?: string } | undefined>();
  const [showProblems, setShowProblems] = createSignal(false);
  const editing = () => props.mode.kind !== 'add';
  const d = () => drafts[idx()]!;
  const noEntry = () => props.mode.kind !== 'add' && !props.mode.server.entry;

  const problems = (i: number) => validateDraft(drafts[i]!, { taken: editing() ? [] : props.taken(drafts[i]!.scope) });
  const allValid = () => drafts.every((_, i) => Object.keys(problems(i)).length === 0);
  const shown = () => (showProblems() ? problems(idx()) : {});
  /** N4: a catalog placeholder still empty blocks Save (the row says which value is missing). */
  const blocked = () => drafts.some((x) => missingRequired(x).length > 0);

  const set = <K extends keyof Draft>(key: K, value: Draft[K]) => setDrafts(idx(), key, value);
  const rowsKey = (): 'headers' | 'env' => (d().type === 'http' ? 'headers' : 'env');
  const patchRow = (id: number, patch: Partial<KvRow>) =>
    setDrafts(idx(), rowsKey(), produce((rows: KvRow[]) => {
      const row = rows.find((r) => r.id === id);
      if (row) Object.assign(row, patch);
    }));
  const addRow = (key = '') => setDrafts(idx(), rowsKey(), produce((rows: KvRow[]) => void rows.push(newRow(key))));
  const dropRow = (id: number) => setDrafts(idx(), rowsKey(), (rows) => rows.filter((r) => r.id !== id));

  const parsed = createMemo(() => (paste().trim() ? parsePaste(paste()) : undefined));

  const preview = () => {
    const r = parsePaste(paste());
    if (!r.ok) return setPasteError(r.error);
    setPasteError(undefined);
    const scope = d().scope;
    const list = draftsFromPaste(r.items, scope, d().name.trim());
    setDrafts(list);
    setIdx(0);
    setShowProblems(false);
    setImported(list.length === 1 ? 'Imported 1 server from the pasted JSON. Check it, then Save.' : `Imported ${list.length} servers from the pasted JSON. Check each one, then Save.`);
    setTab('form');
  };

  const save = async () => {
    setShowProblems(true);
    if (!allValid()) {
      const bad = drafts.findIndex((_, i) => Object.keys(problems(i)).length > 0);
      if (bad >= 0) setIdx(bad);
      setTab('form');
      return;
    }
    setBusy(true);
    setFailure(undefined);
    try {
      const byScope = new Map<McpScope, { entries: Record<string, Record<string, unknown>>; secrets: Record<string, string> }>();
      for (const draft of drafts) {
        const built = buildEntry(draft);
        const bucket = byScope.get(draft.scope) ?? { entries: {}, secrets: {} };
        bucket.entries[draft.name.trim()] = built.entry;
        Object.assign(bucket.secrets, built.secrets);
        byScope.set(draft.scope, bucket);
      }
      let notice: string | undefined;
      let message: string | undefined;
      for (const [scope, b] of byScope) {
        const reply = await props.onSave(scope, b.entries, b.secrets);
        if (!reply.ok) {
          setFailure(reply.message ?? 'The host did not save the server.');
          return;
        }
        notice ??= reply.notice;
        message ??= reply.message;
      }
      setSaved({ names: drafts.map((x) => x.name.trim()), ...(notice ? { notice } : {}), ...(message ? { message } : {}) });
    } catch (e) {
      setFailure(String((e as { message?: unknown })?.message ?? e));
    } finally {
      setBusy(false);
    }
  };

  const title = () => (props.mode.kind === 'add' ? 'Add MCP server' : props.mode.kind === 'override' ? `Override ${props.mode.server.name} in Oino` : `Edit ${props.mode.server.name}`);
  const projectName = () => (props.project ? projectLabel(props.project.cwd) : '');

  return (
    <SheetFrame
      open
      onClose={() => props.onClose()}
      label={title()}
      title={title()}
      detents={['full']}
      noDrag
      class="mcps-sheet"
      testId="mcp-sheet"
      footer={
        <Show
          when={!saved()}
          fallback={
            <div class="mcps-foot">
              <button type="button" class="btn-primary mcps-foot-btn" onClick={() => props.onClose()} data-testid="mcp-sheet-done">Done</button>
            </div>
          }
        >
          <div class="mcps-foot">
            <Show when={failure()}>
              <p class="mcps-failure" role="alert" data-testid="mcp-sheet-failure"><CircleAlert size={14} /> <span>{failure()}</span></p>
            </Show>
            <button type="button" class="btn-secondary mcps-foot-btn" onClick={() => props.onClose()} data-testid="mcp-sheet-cancel">Cancel</button>
            <button type="button" class="btn-primary mcps-foot-btn" disabled={busy() || tab() !== 'form' || blocked()} onClick={() => void save()} data-testid="mcp-sheet-save">
              {busy() ? 'Saving…' : drafts.length > 1 ? `Save ${drafts.length} servers` : 'Save'}
            </button>
          </div>
        </Show>
      }
    >
      <Show
        when={!saved()}
        fallback={
          <div class="mcps-saved" data-testid="mcp-sheet-saved">
            <For each={saved()!.names}>
              {(name) => {
                const row = () => props.servers().find((s) => s.name === name);
                const st = () => (row() ? serverStatus(row()!, { errorShownBelow: true }) : undefined);
                return (
                  <div class="mcps-saved-row" data-testid={`mcp-saved-${name}`}>
                    <strong>{name}</strong>
                    <span class="mcps-status" data-kind={st()?.kind ?? 'connecting'}>{st()?.label ?? 'Connecting…'}</span>
                    <Show when={row()?.error}>
                      <p class="mcps-err"><CircleAlert size={13} /> <span>{row()!.error}</span></p>
                    </Show>
                  </div>
                );
              }}
            </For>
            <p class="mcps-note">Saved{props.project && drafts.some((x) => x.scope === 'project') ? ` in ${projectName()}` : ''}. The servers reload now; this updates as each one connects.</p>
            <Show when={saved()!.notice}>
              <p class="mcps-note" data-testid="mcp-sheet-notice"><Lock size={12} /> {saved()!.notice}</p>
            </Show>
          </div>
        }
      >
        <div class="mcps-sheet-body">
          <Show when={props.mode.kind === 'add'}>
            <div class="mcps-tabrow">
              <div class="seg mcps-tabs" role="tablist" aria-label="How to add">
                <button type="button" role="tab" class="seg-option" aria-selected={tab() === 'form'} aria-checked={tab() === 'form'} onClick={() => setTab('form')} data-testid="mcp-tab-form">Form</button>
                <button type="button" role="tab" class="seg-option" aria-selected={tab() === 'paste'} aria-checked={tab() === 'paste'} onClick={() => setTab('paste')} data-testid="mcp-tab-paste">Paste JSON</button>
              </div>
              <Show when={props.onCatalog}>
                <button type="button" class="btn-secondary settings-btn mcps-btn mcps-browse" aria-pressed={tab() === 'catalog'} onClick={() => setTab('catalog')} data-testid="mcp-browse-catalog">
                  <LibraryBig size={14} aria-hidden="true" /> Browse catalog
                </button>
              </Show>
            </div>
          </Show>

          <Show when={tab() === 'catalog'}>
            <label class="mcps-field">Search the catalog
              <input type="search" class="settings-input" value={query()} onInput={(e) => setQuery(e.currentTarget.value)} placeholder="Search the catalog…" data-testid="mcp-catalog-search" />
            </label>
            <Show when={catalog.error}><p role="alert">Could not load the catalog. The Form and Paste JSON tabs still work.</p></Show>
            <ul class="mcps-cat" data-testid="mcp-catalog-results">
              <For each={catalog() ?? []}>
                {(item) => (
                  <li>
                    <button type="button" class="mcps-cat-item" data-catalog-id={item.id} onClick={() => useCatalog(item)}>
                      <span class="mcps-cat-name">{item.label}</span>
                      <span class="mcps-cat-desc">{item.description}</span>
                    </button>
                  </li>
                )}
              </For>
            </ul>
            <Show when={catalog.state === 'ready' && !(catalog() ?? []).length}><p class="mcps-note">Nothing in the catalog matches that.</p></Show>
          </Show>
          <Show when={tab() === 'paste'}>
            <div class="mcps-field">
              <label class="mcps-label" for="mcp-paste">Paste a server’s JSON</label>
              <textarea
                id="mcp-paste"
                class="settings-textarea mcps-paste"
                rows={9}
                spellcheck={false}
                autocomplete="off"
                placeholder={PASTE_HINT}
                value={paste()}
                onInput={(e) => {
                  setPaste(e.currentTarget.value);
                  setPasteError(undefined);
                }}
                data-testid="mcp-paste"
              />
              <p class="mcps-hint">One entry, <code>{'{ "name": entry }'}</code>, or a whole <code>{'{ "mcpServers": { … } }'}</code> from Claude Code, Cursor or pi. Nothing is saved until you check it in the form.</p>
              <Show when={pasteError() ?? (parsed() && !parsed()!.ok ? (parsed() as { error: string }).error : undefined)}>
                {(err) => <p class="mcps-problem" role="alert" data-testid="mcp-paste-error">{err()}</p>}
              </Show>
              <Show when={parsed()?.ok ? (parsed() as { items: unknown[] }).items.length : 0}>
                {(n) => <p class="mcps-hint" data-testid="mcp-paste-found">Found {n()} server{n() === 1 ? '' : 's'}.</p>}
              </Show>
              <div class="mcps-paste-actions">
                <button type="button" class="btn-secondary mcps-foot-btn" disabled={!parsed()?.ok} onClick={preview} data-testid="mcp-paste-preview">Preview in form</button>
              </div>
            </div>
          </Show>

          <Show when={tab() === 'form'}>
            <Show when={imported()}>
              <p class="mcps-note mcps-imported" data-testid="mcp-imported">{imported()}</p>
            </Show>
            <Show when={noEntry()}>
              <p class="mcps-note" role="status" data-testid="mcp-no-entry">This PC did not send the server’s settings, so the fields start empty. Saving replaces the entry.</p>
            </Show>
            <Show when={drafts.length > 1}>
              <div class="mcps-chips" role="tablist" aria-label="Servers in the paste">
                <Index each={drafts}>
                  {(x, i) => (
                    <button type="button" role="tab" class="mcps-chip" aria-selected={idx() === i} onClick={() => setIdx(i)} data-testid={`mcp-draft-${i}`}>
                      {x().name || `Server ${i + 1}`}
                    </button>
                  )}
                </Index>
              </div>
            </Show>

            <div class="mcps-field">
              <label class="mcps-label" for="mcp-name">Name</label>
              <input
                id="mcp-name"
                class="settings-input mcps-input"
                autofocus={!editing()}
                autocomplete="off"
                spellcheck={false}
                readOnly={editing()}
                placeholder="e.g. linear"
                value={d().name}
                onInput={(e) => set('name', e.currentTarget.value)}
                aria-invalid={!!shown().name}
                data-testid="mcp-name"
              />
              <Show when={shown().name}><p class="mcps-problem" role="alert">{shown().name}</p></Show>
            </div>

            <div class="mcps-field">
              <span class="mcps-label" id="mcp-scope-label">Scope</span>
              <div class="seg" role="radiogroup" aria-labelledby="mcp-scope-label">
                <button type="button" role="radio" class="seg-option" aria-checked={d().scope === 'global'} disabled={editing() && d().scope !== 'global'} onClick={() => set('scope', 'global')} data-testid="mcp-scope-global">Global</button>
                <button
                  type="button"
                  role="radio"
                  class="seg-option"
                  aria-checked={d().scope === 'project'}
                  disabled={!props.project?.writable || (editing() && d().scope !== 'project')}
                  title={!props.project ? 'Pick a project in the scope picker first' : props.project.why}
                  onClick={() => set('scope', 'project')}
                  data-testid="mcp-scope-project"
                >
                  {props.project ? `Project: ${projectName()}` : 'This project'}
                </button>
              </div>
              <p class="mcps-hint">
                {d().scope === 'project'
                  ? <>Saved in <code>.oino/mcp.json</code> of {projectName() || 'the project'}; only chats in that folder see it.</>
                  : <>Saved in <code>~/.oino/mcp.json</code>; every project sees it.</>}
                <Show when={props.project && !props.project.writable && props.project.why}> {props.project!.why}</Show>
              </p>
            </div>

            <div class="mcps-field">
              <span class="mcps-label" id="mcp-type-label">Type</span>
              <div class="seg" role="radiogroup" aria-labelledby="mcp-type-label">
                <For each={[['http', 'HTTP'], ['command', 'Command']] as Array<[McpType, string]>}>
                  {([value, label]) => (
                    <button type="button" role="radio" class="seg-option" aria-checked={d().type === value} onClick={() => set('type', value)} data-testid={`mcp-type-${value}`}>{label}</button>
                  )}
                </For>
              </div>
            </div>

            <Show
              when={d().type === 'http'}
              fallback={
                <>
                  <div class="mcps-field">
                    <label class="mcps-label" for="mcp-command">Command</label>
                    <input id="mcp-command" class="settings-input mcps-input mcps-mono" autocomplete="off" spellcheck={false} placeholder="e.g. npx" value={d().command} onInput={(e) => set('command', e.currentTarget.value)} aria-invalid={!!shown().command} data-testid="mcp-command" />
                    <Show when={shown().command}><p class="mcps-problem" role="alert">{shown().command}</p></Show>
                  </div>
                  <div class="mcps-field">
                    <label class="mcps-label" for="mcp-args">Arguments</label>
                    <input id="mcp-args" class="settings-input mcps-input mcps-mono" autocomplete="off" spellcheck={false} placeholder="e.g. -y @modelcontextprotocol/server-filesystem /tmp" value={d().args} onInput={(e) => set('args', e.currentTarget.value)} data-testid="mcp-args" />
                    <p class="mcps-hint">Separated by spaces; use quotes around an argument that has spaces.</p>
                  </div>
                </>
              }
            >
              <div class="mcps-field">
                <label class="mcps-label" for="mcp-url">URL</label>
                <input id="mcp-url" class="settings-input mcps-input mcps-mono" type="url" inputMode="url" autocomplete="off" spellcheck={false} placeholder="https://mcp.linear.app/mcp" value={d().url} onInput={(e) => set('url', e.currentTarget.value)} aria-invalid={!!shown().url} data-testid="mcp-url" />
                <Show when={shown().url}><p class="mcps-problem" role="alert">{shown().url}</p></Show>
              </div>
            </Show>

            <div class="mcps-field" data-testid={d().type === 'http' ? 'mcp-headers' : 'mcp-env'}>
              <span class="mcps-label">{d().type === 'http' ? 'Headers' : 'Environment variables'}</span>
              <Index each={d()[rowsKey()]}>
                {(row) => <KvLine row={row()} noun={d().type === 'http' ? 'header' : 'variable'} patch={(p) => patchRow(row().id, p)} drop={() => dropRow(row().id)} />}
              </Index>
              <Show when={shown()[rowsKey()]}><p class="mcps-problem" role="alert">{shown()[rowsKey()]}</p></Show>
              <button type="button" class="mcps-add-row" onClick={() => addRow()} data-testid="mcp-add-row">
                <Plus size={14} /> Add {d().type === 'http' ? 'header' : 'variable'}
              </button>
              <p class="mcps-hint"><Lock size={11} /> Secret values go to this PC’s secret store; the file keeps only a <code>{'${NAME}'}</code> reference.</p>
            </div>
          </Show>
        </div>
      </Show>
    </SheetFrame>
  );
}

/** One header / variable row: name, value (•••• + Replace for a stored secret), Secret toggle, remove. */
function KvLine(props: { row: KvRow; noun: string; patch: (p: Partial<KvRow>) => void; drop: () => void }) {
  const stored = () => !!props.row.ref && !props.row.replacing && !props.row.value;
  const needsValue = () => !!props.row.required && !props.row.value.trim();
  return (
    <>
    <div class="mcps-kv" data-secret={props.row.secret ? '' : undefined} data-testid="mcp-kv">
      <input
        class="settings-input mcps-input mcps-mono"
        aria-label={`${props.noun} name`}
        autocomplete="off"
        spellcheck={false}
        placeholder={props.noun === 'header' ? 'Authorization' : 'API_URL'}
        value={props.row.key}
        onInput={(e) => {
          const key = e.currentTarget.value;
          props.patch({ key, ...(props.row.touched ? {} : { secret: isSecretKey(key) || !!props.row.ref }) });
        }}
        data-testid="mcp-kv-key"
      />
      <Show
        when={!stored()}
        fallback={
          <div class="mcps-masked" data-testid="mcp-kv-masked">
            <span aria-label="Stored secret">••••</span>
            <button type="button" class="mcps-link" onClick={() => props.patch({ replacing: true })} data-testid="mcp-kv-replace">Replace</button>
          </div>
        }
      >
        <input
          class="settings-input mcps-input mcps-mono"
          aria-label={`${props.noun} value`}
          type={props.row.secret ? 'password' : 'text'}
          autocomplete="off"
          spellcheck={false}
          placeholder={props.row.ref ? 'New value (leave empty to keep the stored one)' : props.row.required ? 'Required' : props.row.secret ? 'Secret value' : 'Value'}
          aria-required={props.row.required ? true : undefined}
          aria-invalid={needsValue() ? true : undefined}
          value={props.row.value}
          onInput={(e) => { const value = e.currentTarget.value; props.patch({ value, ...(!props.row.touched && /^bearer\s/i.test(value) ? { secret: true } : {}) }); }}
          data-testid="mcp-kv-value"
        />
      </Show>
      <button
        type="button"
        role="switch"
        class="mcps-secret"
        aria-checked={props.row.secret}
        aria-label={`Secret ${props.noun}`}
        title="Store the value in this PC’s secret store"
        onClick={() => {
          const secret = !props.row.secret;
          // A stored reference that stops being secret stays a plain `${NAME}` value.
          props.patch(!secret && props.row.ref ? { secret, touched: true, value: props.row.ref, ref: undefined, replacing: false } : { secret, touched: true });
        }}
        data-testid="mcp-kv-secret"
      >
        <Lock size={13} /> Secret
      </button>
      <button type="button" class="mcps-kv-x" aria-label={`Remove ${props.noun}`} onClick={props.drop} data-testid="mcp-kv-remove"><X size={16} /></button>
    </div>
    <Show when={needsValue()}><p class="mcps-hint mcps-required" data-testid="mcp-kv-required">Required: enter the value for <code>{props.row.key}</code>. This catalog entry does not work without it.</p></Show>
    </>
  );
}
