/**
 * The tree's editor area (WP-A20): the existing CodeMirror editor
 * (`screens/Editor.tsx`, UNI-155/190/191/265) hosted inside the Files pane
 * tab — same theme, languages, vim mode, baseHash save with the conflict
 * sheet, markdown preview ↔ source, and the shared editor-target slot
 * (`lib/editorTarget.ts`) with its unsaved-changes guard, so the phone
 * `/files/edit` screen and this pane area are two hosts of one editor.
 * A21 adds tabs, LSP features and the "Open in external editor" button.
 */
import { createEffect, createSignal, onCleanup, onMount, Show, For } from 'solid-js';
import { Eye, FileText, FileWarning, MessageSquarePlus, Pencil, WrapText, X } from 'lucide-solid';
import { EditorView, drawSelection, keymap, lineNumbers } from '@codemirror/view';
import { EditorState } from '@codemirror/state';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { search, searchKeymap, openSearchPanel } from '@codemirror/search';
import { Vim, vim, getCM } from '@replit/codemirror-vim';
import { syntaxHighlighting } from '@codemirror/language';
import { classHighlighter } from '@lezer/highlight';
import { languageForPath, renderDiffHtml, editDiff, type ParsedDiff } from '../../lib/markdown';
import { editorTheme } from '../../lib/editorTheme';
import { fsRead, fsWrite } from '../../lib/fs';
import { clearEditorTarget, getEditorTarget, setEditorGuard, setEditorTarget, type EditorTarget } from '../../lib/editorTarget';
import { queueMention } from '../../lib/chat/mention';
import { openChatState } from '../../lib/session/open';
import { useSettings } from '../../settings/context';
import { BoardSheet } from '../board/BoardSheet';
import { MdBlock } from '../chat/MdBlock';
import { renderBlocks } from '../../lib/chat/markdown';
import { EmptyState } from '../EmptyState';

type LoadState =
  | { kind: 'empty' }
  | { kind: 'loading' }
  | { kind: 'error'; message: string }
  | { kind: 'ready'; text: string; hash: string; truncated: boolean };

export function EditorArea(props: { onBinary: (path: string) => void }) {
  const [settings] = useSettings();
  const target = () => getEditorTarget();
  const [state, setState] = createSignal<LoadState>({ kind: 'empty' });
  const [dirty, setDirty] = createSignal(false);
  const [wrap, setWrap] = createSignal(true);
  const [saving, setSaving] = createSignal(false);
  const [conflict, setConflict] = createSignal<{ mine: string; mineBefore: string } | undefined>();
  const [diffView, setDiffView] = createSignal<ParsedDiff | undefined>(undefined);
  const [toast, setToast] = createSignal<string | null>(null);
  const [vimMode, setVimMode] = createSignal<string | null>(null);
  const [editAnyway, setEditAnyway] = createSignal(false);
  const vimOn = () => settings.editor.vim;
  const forcedReadOnly = () => !!target()?.asText && !editAnyway();

  let container: HTMLDivElement | undefined;
  let view: EditorView | undefined;
  let baseHash: string | undefined;
  let originalText = '';

  const showToast = (text: string) => {
    setToast(text);
    setTimeout(() => setToast((t) => (t === text ? null : t)), 2500);
  };
  const fileName = () => target()?.path.split('/').pop() ?? '';
  const isMarkdown = () => /\.(md|markdown|mdx)$/i.test(target()?.path ?? '');
  const [preview, setPreview] = createSignal(false);
  const [previewText, setPreviewText] = createSignal('');

  // Another file picked while this one is dirty: ask, then open it (UNI-117).
  const [leaveAsk, setLeaveAsk] = createSignal(false);
  let pendingNext: EditorTarget | undefined;
  setEditorGuard((next) => {
    if (!dirty() || saving()) return true;
    pendingNext = next;
    setLeaveAsk(true);
    return false;
  });
  onCleanup(() => setEditorGuard(undefined));

  async function load() {
    const t = target();
    setPreview(!!t && isMarkdown());
    setPreviewText('');
    setDiffView(undefined);
    setConflict(undefined);
    setEditAnyway(false);
    if (!t) {
      setState({ kind: 'empty' });
      view?.destroy();
      view = undefined;
      return;
    }
    setState({ kind: 'loading' });
    const result = await fsRead(t.hostId, t.path, undefined, t.asText);
    if (target() !== t) return;
    if (!result.ok) {
      if (result.error.code === 'E_BINARY') return props.onBinary(t.path);
      setState({ kind: 'error', message: result.error.code === 'E_NOT_FOUND' ? `${t.path.split('/').pop()} no longer exists` : result.error.message });
      return;
    }
    baseHash = result.value.hash;
    originalText = result.value.text;
    setState({ kind: 'ready', text: result.value.text, hash: result.value.hash, truncated: result.value.truncated });
  }
  createEffect(() => void load());

  async function save() {
    const t = target();
    if (!t || !view || state().kind !== 'ready') return false;
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

  async function keepMine() {
    const c = conflict();
    const t = target();
    if (!t || !c) return;
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
    const t = target();
    if (!t || !c) return;
    const theirs = await fsRead(t.hostId, t.path);
    setDiffView({ files: [{ oldPath: t.path, newPath: t.path, hunks: [editDiff(theirs.ok ? theirs.value.text : '', c.mine)] }] });
  }

  async function mountEditor(text: string, readOnly: boolean) {
    if (!container) return;
    view?.destroy();
    setVimMode(null);
    const extensions = [
      lineNumbers(),
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
    const t = target();
    if (t) {
      const lang = await languageForPath(t.path);
      if (lang) extensions.push(lang);
    }
    view = new EditorView({ state: EditorState.create({ doc: text, extensions }), parent: container });
    const line = t?.line;
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

  createEffect(() => {
    const s = state();
    if (s.kind === 'ready') {
      setDirty(false);
      void mountEditor(s.text, s.truncated || forcedReadOnly());
    }
  });
  createEffect(() => {
    // First load: the markdown preview shows the file as read (UNI-265).
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

  // Mod-S / Mod-F work while the editor isn't focused (pane-level keys).
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
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
  onCleanup(() => view?.destroy());

  const toggleWrap = () => {
    setWrap((w) => !w);
    if (view) void mountEditor(view.state.doc.toString(), state().kind === 'ready' && !!(state() as { truncated?: boolean }).truncated);
  };
  const togglePreview = () => {
    if (!preview()) setPreviewText(view?.state.doc.toString() ?? '');
    setPreview(!preview());
    if (!preview()) queueMicrotask(() => view?.focus());
  };

  function mention() {
    const t = target();
    if (!t) return;
    const open = openChatState();
    if (open && open.target.hostId === t.hostId) {
      queueMention(`${open.target.hostId}:${open.target.session}`, `@${t.path} `);
      showToast('Added @path to the chat draft');
      return;
    }
    void navigator.clipboard?.writeText(`@${t.path}`).then(
      () => showToast('Copied @path — paste it into a chat'),
      () => showToast('Could not copy'),
    );
  }

  function close() {
    if (dirty() && !saving()) {
      setLeaveAsk(true);
      return;
    }
    clearEditorTarget();
  }
  async function saveAndLeave() {
    const ok = await save();
    setLeaveAsk(false);
    if (ok || !dirty()) applyPending();
  }
  function applyPending() {
    setLeaveAsk(false);
    if (pendingNext) {
      const next = pendingNext;
      pendingNext = undefined;
      setEditorGuard(undefined);
      setEditorTarget(next);
      return;
    }
    clearEditorTarget();
  }

  return (
    <div class="feditor" data-testid="files-editor">
      <div class="feditor-bar" role="toolbar" aria-label="Editor tools">
        <Show when={target()}>
          <button type="button" class="ftree-tool" aria-label="Close file" onClick={close} data-testid="files-editor-close">
            <X size={15} />
          </button>
        </Show>
        <div class="feditor-title" title={target()?.path}>
          <span class="feditor-name">{fileName() || 'No file'}</span>
          <Show when={dirty()}>
            <span class="editor-dirty-dot" aria-label="Unsaved changes" />
          </Show>
          <Show when={vimOn() && vimMode()}>
            <span class="feditor-vim" data-testid="files-editor-vim">{vimMode()}</span>
          </Show>
        </div>
        <span class="ftree-bar-fill" />
        <Show when={isMarkdown() && state().kind === 'ready'}>
          <button type="button" class={`ftree-tool${preview() ? ' on' : ''}`} aria-pressed={preview()} aria-label={preview() ? 'Edit' : 'Preview'} data-testid="files-editor-preview" onClick={togglePreview}>
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
        <Show when={target()} fallback={<EmptyState compact icon={<FileText size={22} />} title="No file open" body="Pick a file in the tree." testId="files-editor-empty" />}>
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
              <button type="button" class="underline" onClick={() => setEditAnyway(true)}>Edit anyway</button>
              {' '}(saving will rewrite the file as text).
            </p>
          </Show>
          <div class="feditor-cm" ref={container} classList={{ hidden: state().kind !== 'ready' || (isMarkdown() && preview()) }} />
          <Show when={state().kind === 'ready' && isMarkdown() && preview()}>
            <div class="feditor-md md" data-testid="files-editor-md">
              <For each={renderBlocks(previewText(), false)}>{(html) => <MdBlock html={html} class="chat-block" />}</For>
            </div>
          </Show>
        </Show>
      </div>

      <BoardSheet open={leaveAsk()} onClose={() => ((pendingNext = undefined), setLeaveAsk(false))} label="Unsaved changes" testId="files-editor-leave">
        <p class="p-2 text-text-muted">Save your changes to {fileName()} before leaving?</p>
        <div class="files-sheet-actions">
          <button type="button" class="files-sheet-btn" onClick={() => ((pendingNext = undefined), setLeaveAsk(false))}>Keep editing</button>
          <button type="button" class="files-sheet-btn danger" onClick={applyPending}>Discard</button>
          <button type="button" class="files-sheet-btn primary" onClick={() => void saveAndLeave()}>Save</button>
        </div>
      </BoardSheet>

      <Show when={conflict()}>
        <BoardSheet open onClose={() => setConflict(undefined)} label="Someone else changed this file" testId="files-editor-conflict">
          <p class="p-2 text-text-muted">The file on the PC changed since you opened it. What would you like to do?</p>
          <Show when={diffView()}>
            {(d) => <div class="md" innerHTML={renderDiffHtml(d())} />}
          </Show>
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
