/**
 * One editor tab's CodeMirror view (WP-A21): the old `EditorArea` body —
 * fs_read → EditorView → fs_write(baseHash) with the conflict sheet,
 * markdown preview, vim mode, wrap — plus the LSP wiring: on mount it
 * acquires the pooled `(root, language)` connection and appends the
 * `client.plugin(uri)` + `languageServerExtensions()` extensions; every
 * failure degrades to plain editing. The view lives for the tab's whole
 * life (its host hides it on tab switches), so undo history and LSP
 * plugin state survive.
 */
import { createEffect, createSignal, onCleanup, onMount, Show, For } from 'solid-js';
import { Eye, FileWarning, MessageSquarePlus, Pencil, WrapText } from 'lucide-solid';
import { EditorView, drawSelection, keymap, lineNumbers } from '@codemirror/view';
import { EditorState } from '@codemirror/state';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { search, searchKeymap, openSearchPanel } from '@codemirror/search';
import { Vim, vim, getCM } from '@replit/codemirror-vim';
import { syntaxHighlighting } from '@codemirror/language';
import { classHighlighter } from '@lezer/highlight';
import { StateEffect } from '@codemirror/state';
import { languageForPath, renderDiffHtml, editDiff, type ParsedDiff } from '../../lib/markdown';
import { editorTheme } from '../../lib/editorTheme';
import { fsRead, fsWrite } from '../../lib/fs';
import { queueMention } from '../../lib/chat/mention';
import { activeChatTargetFor, openNewChat } from '../../lib/session/open';
import { useSettings } from '../../settings/context';
import { hostClient } from '../../lib/host/factory';
import { acquireLsp, type PooledLsp } from '../../lib/lsp/pool';
import { lspExtensions } from '../../lib/lsp/features';
import { BoardSheet } from '../board/BoardSheet';
import { MdBlock } from '../chat/MdBlock';
import { renderBlocks } from '../../lib/chat/markdown';
import type { EditorStore, EditorTab } from '../../lib/editor/tabs';

type LoadState =
  | { kind: 'loading' }
  | { kind: 'error'; message: string }
  | { kind: 'ready'; text: string; hash: string; truncated: boolean };

export function Editor(props: {
  tab: EditorTab;
  store: EditorStore;
  root?: () => string | undefined;
  visible: () => boolean;
  onBinary: (path: string) => void;
  onLsp: (tab: string, pooled: PooledLsp | null) => void;
  onRequestClose: (tab: EditorTab) => void;
}) {
  const [settings] = useSettings();
  const tab = () => props.tab;
  const [state, setState] = createSignal<LoadState>({ kind: 'loading' });
  const [dirty, setDirty] = createSignal(false);
  const [wrap, setWrap] = createSignal(settings.appearance.codeWrap);
  const [saving, setSaving] = createSignal(false);
  const [conflict, setConflict] = createSignal<{ mine: string; mineBefore: string } | undefined>();
  const [diffView, setDiffView] = createSignal<ParsedDiff | undefined>(undefined);
  const [toast, setToast] = createSignal<string | null>(null);
  const [vimMode, setVimMode] = createSignal<string | null>(null);
  const [editAnyway, setEditAnyway] = createSignal(false);
  const vimOn = () => settings.editor.vim;
  // "Edit as text" (UNI-191): a real text file opens editable; only a file the
  // host refused as binary is shown decoded, read-only (owner5 #24).
  const [decoded, setDecoded] = createSignal(false);
  const forcedReadOnly = () => decoded() && !editAnyway();
  const isMarkdown = () => /\.(md|markdown|mdx)$/i.test(tab().path);
  const [preview, setPreview] = createSignal(false);
  const [previewText, setPreviewText] = createSignal('');

  let container: HTMLDivElement | undefined;
  let view: EditorView | undefined;
  let baseHash: string | undefined;
  let originalText = '';
  let pooled: PooledLsp | null = null;

  const showToast = (text: string) => {
    setToast(text);
    setTimeout(() => setToast((t) => (t === text ? null : t)), 2500);
  };

  async function load() {
    const t = tab();
    setPreview(isMarkdown());
    setPreviewText('');
    setDiffView(undefined);
    setConflict(undefined);
    setEditAnyway(false);
    setState({ kind: 'loading' });
    let result = await fsRead(t.hostId, t.path);
    setDecoded(false);
    if (!result.ok && result.error.code === 'E_BINARY' && t.asText) {
      result = await fsRead(t.hostId, t.path, undefined, true);
      setDecoded(result.ok);
    }
    if (!result.ok) {
      if (result.error.code === 'E_BINARY') return props.onBinary(t.path);
      setState({
        kind: 'error',
        message: result.error.code === 'E_NOT_FOUND' ? `${t.path.split('/').pop()} no longer exists` : result.error.message,
      });
      return;
    }
    baseHash = result.value.hash;
    originalText = result.value.text;
    setState({ kind: 'ready', text: result.value.text, hash: result.value.hash, truncated: result.value.truncated });
  }

  async function save() {
    const t = tab();
    if (!view || state().kind !== 'ready') return false;
    setSaving(true);
    const text = view.state.doc.toString();
    const result = await fsWrite(t.hostId, t.path, text, baseHash, false);
    setSaving(false);
    if (result.ok) {
      baseHash = result.value.hash;
      originalText = text;
      setDirty(false);
      showToast('Saved');
      return true;
    }
    if (result.error.code === 'E_CONFLICT') {
      setConflict({ mine: text, mineBefore: originalText });
      return false;
    }
    showToast(result.error.message);
    return false;
  }
  props.store.registerOps(tab().id, { save, reconnectLsp: () => void attachLsp() });

  async function keepMine() {
    const c = conflict();
    const t = tab();
    if (!c) return;
    setConflict(undefined);
    const result = await fsWrite(t.hostId, t.path, c.mine, undefined, false);
    if (result.ok) {
      baseHash = result.value.hash;
      originalText = c.mine;
      setDirty(false);
      showToast('Saved (overwrote theirs)');
    } else showToast(result.error.message);
  }

  async function showDiff() {
    const c = conflict();
    const t = tab();
    if (!c) return;
    const theirs = await fsRead(t.hostId, t.path);
    setDiffView({ files: [{ oldPath: t.path, newPath: t.path, hunks: [editDiff(theirs.ok ? theirs.value.text : '', c.mine)] }] });
  }

  async function mountEditor(text: string, readOnly: boolean) {
    if (!container) return;
    view?.destroy();
    setVimMode(null);
    const extensions = [
      ...(settings.appearance.codeLineNumbers ? [lineNumbers()] : []),
      history(),
      search({ top: true }),
      keymap.of([{ key: 'Mod-s', preventDefault: true, run: () => (void save(), true) }, ...defaultKeymap, ...historyKeymap, ...searchKeymap]),
      editorTheme,
      syntaxHighlighting(classHighlighter),
      EditorView.updateListener.of((update) => {
        if (update.docChanged) setDirty(true);
      }),
      EditorState.readOnly.of(readOnly),
    ];
    if (vimOn() && !readOnly) extensions.unshift(vim(), drawSelection());
    if (wrap()) extensions.push(EditorView.lineWrapping);
    const lang = await languageForPath(tab().path);
    if (lang) extensions.push(lang);
    // LSP rides the pool: a resolved connection appends its extension
    // bundle; anything failing just leaves plain editing (§3.12).
    extensions.push(...lspExtensions(pooled, tab().path));
    view = new EditorView({ state: EditorState.create({ doc: text, extensions }), parent: container });
    props.store.registerView(tab().id, view);
    const line = tab().line;
    if (line) {
      const lineNo = Math.min(Math.max(1, line), view.state.doc.lines);
      const at = view.state.doc.line(lineNo);
      view.dispatch({ selection: { anchor: at.from }, effects: EditorView.scrollIntoView(at.from, { y: 'center' }) });
    }
    if (vimOn() && !readOnly) {
      const cm = getCM(view);
      if (cm) {
        setVimMode((cm.state.vim?.mode ?? 'normal').toUpperCase());
        cm.on('vim-mode-change', (e: { mode: string }) => setVimMode(e.mode.toUpperCase()));
      }
    }
  }

  async function attachLsp() {
    const t = tab();
    const host = hostClient(t.hostId);
    const root = props.root?.();
    // A re-acquire (take-back) first drops this tab's reference on the
    // old, evicted connection — then attaches a fresh one (the host's
    // single-attach makes this stream the owner again).
    pooled?.release();
    pooled = null;
    if (!host || !root) {
      props.onLsp(t.id, null);
      return;
    }
    const acquired = await acquireLsp(host, root, t.path, {
      displayFile: (path) => {
        const next = props.store.open({ hostId: t.hostId, path });
        return props.store.viewReady(next.id);
      },
      onEditConflict: (paths) => {
        // A server-side edit (rename/code action) hit files that
        // couldn't be written — list them instead of silently losing it.
        showToast(`Couldn't apply edits to ${paths.map((p) => p.split('/').pop()).join(', ')} (changed on disk)`);
      },
    });
    pooled = acquired;
    props.onLsp(t.id, acquired);
    const extra = lspExtensions(acquired, t.path);
    if (extra.length && view) {
      view.dispatch({ effects: StateEffect.appendConfig.of(extra) });
    }
  }

  createEffect(() => {
    const s = state();
    if (s.kind === 'ready') {
      setDirty(false);
      void mountEditor(s.text, s.truncated || forcedReadOnly());
    }
  });
  createEffect(() => {
    const s = state();
    if (s.kind === 'ready' && preview() && !previewText()) setPreviewText(view?.state.doc.toString() ?? s.text);
  });
  let lastVim = vimOn();
  createEffect(() => {
    const on = vimOn();
    const s = state();
    if (on !== lastVim && s.kind === 'ready' && view) {
      lastVim = on;
      void mountEditor(view.state.doc.toString(), s.truncated || forcedReadOnly());
    } else lastVim = on;
  });
  createEffect(() => props.store.setDirty(tab().id, dirty()));

  // Mod-S / Mod-F while this editor is the visible one but unfocused.
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!props.visible()) return;
      if (!(e.ctrlKey || e.metaKey) || e.altKey || e.shiftKey) return;
      if ((e.target as HTMLElement | null)?.closest?.('.cm-editor')) return;
      const k = e.key.toLowerCase();
      if (k === 's' && dirty()) {
        e.preventDefault();
        void save();
      } else if (k === 'f' && view) {
        e.preventDefault();
        openSearchPanel(view);
      }
    };
    window.addEventListener('keydown', onKey);
    onCleanup(() => window.removeEventListener('keydown', onKey));
  });
  onMount(() => {
    void load();
    void attachLsp();
  });
  onCleanup(() => {
    props.store.registerView(tab().id, null);
    view?.destroy();
    view = undefined;
    pooled?.release();
    pooled = null;
  });

  const toggleWrap = () => {
    setWrap((w) => !w);
    const s = state();
    if (s.kind === 'ready' && view) void mountEditor(view.state.doc.toString(), s.truncated || forcedReadOnly());
  };
  const togglePreview = () => {
    if (!preview()) setPreviewText(view?.state.doc.toString() ?? '');
    setPreview(!preview());
    if (!preview()) queueMicrotask(() => view?.focus());
  };

  function mention() {
    const t = tab();
    const leaf = t.path.split('/').pop() || t.path;
    const active = activeChatTargetFor(t.hostId);
    if (active) {
      queueMention(active.mentionKey, `@${t.path} `);
      showToast(`Added @${leaf} to the chat draft`);
      return;
    }
    const cwd = props.root?.() ?? (t.path.replace(/\/[^/]*$/, '') || undefined);
    queueMention(`draft:${t.hostId}`, `@${t.path} `);
    openNewChat(t.hostId, cwd);
    showToast(`Added @${leaf} to the chat draft`);
  }

  return (
    <div class="feditor" data-testid={`files-editor-${tab().path}`}>
      <div class="feditor-bar" role="toolbar" aria-label="Editor tools">
        <div class="feditor-title" title={tab().path}>
          <Show when={vimOn() && vimMode()}>
            <span class="feditor-vim" data-testid="files-editor-vim">{vimMode()}</span>
          </Show>
        </div>
        <span class="ftree-bar-fill" />
        <Show when={isMarkdown() && state().kind === 'ready'}>
          <button
            type="button"
            class={`ftree-tool${preview() ? ' on' : ''}`}
            aria-pressed={preview()}
            aria-label={preview() ? 'Edit' : 'Preview'}
            data-testid="files-editor-preview"
            onClick={togglePreview}
          >
            <Show when={preview()} fallback={<Eye size={15} />}>
              <Pencil size={15} />
            </Show>
          </button>
        </Show>
        <button type="button" class={`ftree-tool${wrap() ? ' on' : ''}`} aria-pressed={wrap()} aria-label="Toggle word wrap" onClick={toggleWrap}>
          <WrapText size={15} />
        </button>
        <button type="button" class="ftree-tool" aria-label="Mention in chat" onClick={mention}>
          <MessageSquarePlus size={15} />
        </button>
        <button type="button" class="feditor-save" disabled={saving() || !dirty()} data-testid="files-editor-save" onClick={() => void save()}>
          {saving() ? '…' : 'Save'}
        </button>
      </div>

      <div class="feditor-body">
        <Show when={state().kind === 'loading'}>
          <p class="feditor-note">Loading…</p>
        </Show>
        <Show when={state().kind === 'error'}>
          <p class="feditor-note" role="alert" data-testid="files-editor-error">
            <FileWarning size={14} /> {(state() as { message: string }).message}
          </p>
        </Show>
        <Show when={state().kind === 'ready' && (state() as { truncated?: boolean }).truncated}>
          <p class="feditor-note">This file is big — showing the start only, read-only.</p>
        </Show>
        <Show when={state().kind === 'ready' && forcedReadOnly()}>
          <p class="feditor-note" data-testid="files-editor-as-text">
            This file wasn't text — showing it decoded, read-only.{' '}
            <button type="button" class="underline" onClick={() => setEditAnyway(true)}>Edit anyway</button>{' '}
            (saving will rewrite the file as text).
          </p>
        </Show>
        <div class="feditor-cm" ref={container} classList={{ hidden: state().kind !== 'ready' || (isMarkdown() && preview()) }} />
        <Show when={state().kind === 'ready' && isMarkdown() && preview()}>
          <div class="feditor-md chat-prose md" data-testid="files-editor-md">
            <For each={renderBlocks(previewText(), false)}>{(html) => <MdBlock html={html} class="chat-block" />}</For>
          </div>
        </Show>
      </div>

      <Show when={conflict()}>
        <BoardSheet open onClose={() => setConflict(undefined)} label="Someone else changed this file" testId="files-editor-conflict">
          <p class="p-2 text-text-muted">The file on the PC changed since you opened it. What would you like to do?</p>
          <Show when={diffView()}>{(d) => <div class="md" innerHTML={renderDiffHtml(d())} />}</Show>
          <div class="conflict-sheet-row">
            <button type="button" class="conflict-sheet-btn primary" onClick={() => void keepMine()}>Keep mine (overwrite)</button>
            <button type="button" class="conflict-sheet-btn" onClick={() => (setConflict(undefined), void load())}>Load theirs</button>
            <button type="button" class="conflict-sheet-btn" onClick={() => void showDiff()}>Show diff</button>
          </div>
        </BoardSheet>
      </Show>

      <Show when={toast()}>
        <div class="files-toast" role="status">{toast()}</div>
      </Show>
    </div>
  );
}
