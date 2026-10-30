/**
 * Editor (UNI-155): CodeMirror 6 EditorView with a key bar above the
 * keyboard (Tab, brackets, quotes, arrows, undo/redo, Save), language
 * highlighting from the shared loader (lib/markdown/highlight.ts),
 * dirty indicator, Save → fs_write with baseHash. On E_CONFLICT a sheet
 * offers Keep mine / Load theirs / Show diff. Big/truncated files open
 * read-only; images/PDF/binary route to the previewer.
 */
import { createEffect, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import {
  ChevronLeft,
  Eye,
  MessageSquarePlus,
  Pencil,
  Redo2,
  Undo2,
  WrapText,
} from 'lucide-solid';
import { EditorView, drawSelection, keymap, lineNumbers } from '@codemirror/view';
import { EditorState } from '@codemirror/state';
import { defaultKeymap, history, historyKeymap, redo, undo } from '@codemirror/commands';
import { search, searchKeymap, openSearchPanel } from '@codemirror/search';
import { Vim, vim, getCM } from '@replit/codemirror-vim';
import { syntaxHighlighting } from '@codemirror/language';
import { classHighlighter } from '@lezer/highlight';
import { languageForPath, renderDiffHtml, editDiff, type ParsedDiff } from '../lib/markdown';
import { editorTheme } from '../lib/editorTheme';
import { getEditorTarget, clearEditorTarget, setEditorGuard, setEditorTarget, type EditorTarget } from '../lib/editorTarget';
import { desktopShell } from '../lib/layout';
import { fsRead, fsWrite } from '../lib/fs';
import { blobTransport } from '../lib/blob';
import { classifyMedia } from '../lib/media/previewer';
import { openPreviewer } from '../lib/media/previewerState';
import { pushBack } from '../lib/back';
import { hapticTick } from '../lib/haptics';
import { useSettings } from '../settings/context';
import { useHosts } from '../hosts/context';
import { queueMention } from '../lib/chat/mention';
import { chatVisible, openChatState } from '../lib/session/open';
import { BoardSheet } from '../components/board/BoardSheet';
import { renderBlocks } from '../lib/chat/markdown';
import { MdBlock } from '../components/chat/MdBlock';
import './files.css';

type LoadState =
  | { kind: 'loading' }
  | { kind: 'error'; message: string }
  | { kind: 'binary' }
  | { kind: 'ready'; text: string; hash: string; mtime: number; truncated: boolean };

// `:w` always saves through whichever Editor instance is currently
// mounted — Vim.defineEx is global/static, so it's registered once and
// forwards to a holder this component updates on mount/cleanup.
let currentVimSave: (() => void) | null = null;
/** `:q` (asks if dirty), `:q!` (discard), `:wq`/`:x` (save then leave). */
let currentVimQuit: ((how: 'ask' | 'force' | 'save') => void) | null = null;
let vimExRegistered = false;
function ensureVimExCommands() {
  if (vimExRegistered) return;
  vimExRegistered = true;
  Vim.defineEx('write', 'w', () => {
    currentVimSave?.();
  });
  Vim.defineEx('quit', 'q', (_cm: unknown, params: { argString?: string; input?: string }) => {
    const force = (params.input ?? '').trim().endsWith('!') || (params.argString ?? '').trim() === '!';
    currentVimQuit?.(force ? 'force' : 'ask');
  });
  Vim.defineEx('wq', 'wq', () => currentVimQuit?.('save'));
  Vim.defineEx('xit', 'x', () => currentVimQuit?.('save'));
}

export default function Editor() {
  const navigate = useNavigate();
  const [settings] = useSettings();
  const hosts = useHosts();
  const haptic = () => void hapticTick(settings.input.haptics);

  const target = getEditorTarget();
  const [state, setState] = createSignal<LoadState>({ kind: 'loading' });
  const [dirty, setDirty] = createSignal(false);
  const [wrap, setWrap] = createSignal(settings.appearance.codeWrap);
  const [saving, setSaving] = createSignal(false);
  const [conflict, setConflict] = createSignal<{ theirHash: string; mine: string; mineBefore: string } | undefined>(undefined);
  const [toast, setToast] = createSignal<string | null>(null);
  const [vimMode, setVimMode] = createSignal<string | null>(null);
  const vimOn = () => settings.editor.vim;
  // UNI-191 "Edit as text": a target opened with asText starts read-only
  // (saving rewrites the file as text) until the user explicitly unlocks it.
  const [editAnywayUnlocked, setEditAnywayUnlocked] = createSignal(false);
  const forcedReadOnly = () => !!target?.asText && !editAnywayUnlocked();

  let container: HTMLDivElement | undefined;
  let view: EditorView | undefined;
  let baseHash: string | undefined;
  let originalText = '';

  const showToast = (text: string) => {
    setToast(text);
    setTimeout(() => setToast((t) => (t === text ? null : t)), 2500);
  };

  /** Back / ✕ with unsaved changes asks first (Save / Discard / Keep editing). */
  const [leaveAsk, setLeaveAsk] = createSignal(false);
  // UNI-117 desktop: another file picked in the list while this one is dirty — ask, then open it.
  let pendingNext: EditorTarget | undefined;
  setEditorGuard((next) => {
    if (!dirty() || saving()) return true;
    pendingNext = next;
    setLeaveAsk(true);
    return false;
  });
  onCleanup(() => setEditorGuard(undefined));
  function close() {
    if (dirty() && !saving()) {
      setLeaveAsk(true);
      return;
    }
    leave();
  }
  async function saveAndLeave() {
    await save();
    if (!dirty() && !conflict()) {
      setLeaveAsk(false);
      leave();
    } else setLeaveAsk(false);
  }
  function leave() {
    setLeaveAsk(false);
    if (pendingNext) {
      const next = pendingNext;
      pendingNext = undefined;
      setEditorGuard(undefined);
      setEditorTarget(next);
      return;
    }
    clearEditorTarget();
    // Opened from a chat ("Open in editor" in Changes this turn): back
    // returns to that chat — same layer, same scroll — instead of
    // dropping into Files (UNI-172).
    if (chatVisible()) {
      navigate('/chat');
      return;
    }
    navigate('/files');
  }

  onMount(() => onCleanup(pushBack(close)));
  // Desktop: Ctrl/⌘+S saves and Ctrl/⌘+F finds even when the editor itself isn't focused.
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!desktopShell() || !(e.ctrlKey || e.metaKey) || e.altKey || e.shiftKey) return;
      if ((e.target as HTMLElement | null)?.closest?.('.cm-editor')) return; // CodeMirror's keymap has it
      const k = e.key.toLowerCase();
      if (k === 's') {
        e.preventDefault();
        if (dirty()) void save();
      } else if (k === 'f' && view) {
        e.preventDefault();
        openSearchPanel(view);
      }
    };
    window.addEventListener('keydown', onKey);
    onCleanup(() => window.removeEventListener('keydown', onKey));
  });

  async function load() {
    if (!target) {
      setState({ kind: 'error', message: 'No file to open' });
      return;
    }
    setState({ kind: 'loading' });
    const result = await fsRead(target.hostId, target.path, undefined, target.asText);
    if (!result.ok) {
      if (result.error.code === 'E_BINARY') {
        setState({ kind: 'binary' });
        void openBinary();
        return;
      }
      if (result.error.code === 'E_NOT_FOUND') {
        // Deleted since the turn that edited it (UNI-178).
        setState({ kind: 'error', message: `This file no longer exists: ${target.path.split('/').pop() ?? target.path}` });
        return;
      }
      setState({ kind: 'error', message: result.error.message });
      return;
    }
    baseHash = result.value.hash;
    originalText = result.value.text;
    setState({ kind: 'ready', text: result.value.text, hash: result.value.hash, mtime: result.value.mtime, truncated: result.value.truncated });
  }

  async function openBinary() {
    if (!target) return;
    try {
      const got = await blobTransport().getBytes(target.hostId, { path: target.path });
      const kind = classifyMedia(got.mime, target.path);
      const blob = new Blob([got.bytes as BlobPart], { type: got.mime || 'application/octet-stream' });
      const name = target.path.split('/').pop() ?? target.path;
      const source = { hostId: target.hostId, path: target.path };
      if (kind === 'image') {
        openPreviewer({ kind: 'image', items: [{ url: URL.createObjectURL(blob), mime: got.mime, name }], index: 0 });
      } else if (kind === 'pdf' || kind === 'audio' || kind === 'video' || kind === 'other') {
        openPreviewer({ kind, name, mime: got.mime, url: URL.createObjectURL(blob), source });
      } else {
        openPreviewer({ kind, name, mime: got.mime, text: await blob.text(), source });
      }
    } catch {
      showToast("Couldn't open that file");
    }
    leave();
  }

  createEffect(() => {
    void load();
  });

  async function mountEditor(text: string, readOnly: boolean) {
    if (!container) return;
    view?.destroy();
    setVimMode(null);
    const extensions = [
      // Z-114 (WP-A24): line numbers follow the Appearance setting.
      ...(settings.appearance.codeLineNumbers ? [lineNumbers()] : []),
      history(),
      search({ top: true }),
      // Mod-s saves (desktop keyboards; harmless on the phone); Mod-f is searchKeymap's find.
      keymap.of([{ key: 'Mod-s', preventDefault: true, run: () => (void save(), true) }, ...defaultKeymap, ...historyKeymap, ...searchKeymap]),
      editorTheme,
      // Same `tok-*` classes (and colours) as chat/board code blocks.
      syntaxHighlighting(classHighlighter),
      EditorView.updateListener.of((update) => {
        if (update.docChanged) setDirty(true);
      }),
      EditorState.readOnly.of(readOnly),
    ];
    if (vimOn() && !readOnly) {
      ensureVimExCommands();
      // vim() draws its own visual-mode selection + block cursor; it needs
      // drawSelection, or a visual selection is invisible (UNI-190).
      extensions.unshift(vim(), drawSelection());
    }
    if (wrap()) extensions.push(EditorView.lineWrapping);
    if (target) {
      const lang = await languageForPath(target.path);
      if (lang) extensions.push(lang);
    }
    view = new EditorView({
      state: EditorState.create({ doc: text, extensions }),
      parent: container,
    });
    if (target?.line) {
      const lineNo = Math.min(Math.max(1, target.line), view.state.doc.lines);
      const line = view.state.doc.line(lineNo);
      view.dispatch({ selection: { anchor: line.from }, effects: EditorView.scrollIntoView(line.from, { y: 'center' }) });
    }
    if (vimOn() && !readOnly) {
      currentVimSave = saveFromVim;
      currentVimQuit = quitFromVim;
      const cm = getCM(view);
      if (cm) {
        setVimMode((cm.state.vim?.mode ?? 'normal').toUpperCase());
        cm.on('vim-mode-change', (e: { mode: string; subMode?: string }) => {
          setVimMode(e.mode.toUpperCase());
        });
      }
    } else {
      currentVimSave = null;
      currentVimQuit = null;
    }
  }

  function quitFromVim(how: 'ask' | 'force' | 'save') {
    if (how === 'force') leave();
    else if (how === 'save') void (dirty() ? saveAndLeave() : Promise.resolve(leave()));
    else close();
  }

  function saveFromVim() {
    void save();
  }

  function pressVimEsc() {
    haptic();
    if (!view) return;
    const cm = getCM(view);
    if (cm) Vim.handleKey(cm, '<Esc>', 'user');
    view.focus();
  }

  createEffect(() => {
    const s = state();
    if (s.kind === 'ready') {
      setDirty(false);
      void mountEditor(s.text, s.truncated || forcedReadOnly());
    }
  });

  onCleanup(() => {
    view?.destroy();
    if (currentVimSave === saveFromVim) currentVimSave = null;
    if (currentVimQuit === quitFromVim) currentVimQuit = null;
  });

  let lastVimOn = vimOn();
  createEffect(() => {
    const on = vimOn();
    const s = state();
    if (on !== lastVimOn && s.kind === 'ready' && view) {
      lastVimOn = on;
      const text = view.state.doc.toString();
      void mountEditor(text, s.truncated || forcedReadOnly());
    } else {
      lastVimOn = on;
    }
  });

  function editAnyway() {
    haptic();
    setEditAnywayUnlocked(true);
    const s = state();
    if (s.kind === 'ready' && view) {
      const text = view.state.doc.toString();
      void mountEditor(text, s.truncated);
    }
  }

  function toggleWrap() {
    haptic();
    setWrap((w) => !w);
    const s = state();
    if (s.kind === 'ready' && view) {
      const text = view.state.doc.toString();
      void mountEditor(text, s.truncated);
    }
  }

  function insertAtCursor(text: string) {
    if (!view) return;
    const { from, to } = view.state.selection.main;
    view.dispatch({ changes: { from, to, insert: text }, selection: { anchor: from + text.length } });
    view.focus();
  }

  function wrapSelection(open: string, close: string) {
    if (!view) return;
    const { from, to } = view.state.selection.main;
    const selected = view.state.sliceDoc(from, to);
    view.dispatch({
      changes: { from, to, insert: `${open}${selected}${close}` },
      selection: { anchor: from + open.length, head: from + open.length + selected.length },
    });
    view.focus();
  }

  function moveCursor(dx: number, dy: number) {
    if (!view) return;
    const doc = view.state.doc;
    const pos = view.state.selection.main.head;
    if (dy !== 0) {
      const line = doc.lineAt(pos);
      const targetLineNo = Math.min(Math.max(1, line.number + dy), doc.lines);
      const targetLine = doc.line(targetLineNo);
      const col = pos - line.from;
      const next = Math.min(targetLine.from + col, targetLine.to);
      view.dispatch({ selection: { anchor: next } });
    } else {
      const next = Math.min(Math.max(0, pos + dx), doc.length);
      view.dispatch({ selection: { anchor: next } });
    }
    view.focus();
  }

  async function save() {
    if (!target || !view) return;
    const s = state();
    if (s.kind !== 'ready') return;
    haptic();
    setSaving(true);
    const text = view.state.doc.toString();
    const result = await fsWrite(target.hostId, target.path, text, baseHash, false);
    setSaving(false);
    if (result.ok) {
      baseHash = result.value.hash;
      setDirty(false);
      showToast('Saved');
      return;
    }
    if (result.error.code === 'E_CONFLICT') {
      const theirHash = (result.error.detail as { hash?: string } | undefined)?.hash ?? '';
      setConflict({ theirHash, mine: text, mineBefore: originalText });
      return;
    }
    showToast(result.error.message);
  }

  async function keepMine() {
    const c = conflict();
    if (!target || !c) return;
    setConflict(undefined);
    const result = await fsWrite(target.hostId, target.path, c.mine, undefined, false);
    if (result.ok) {
      baseHash = result.value.hash;
      setDirty(false);
      showToast('Saved (overwrote theirs)');
    } else {
      showToast(result.error.message);
    }
  }

  async function loadTheirs() {
    const c = conflict();
    if (!target || !c) return;
    setConflict(undefined);
    await load();
  }

  const [diffView, setDiffView] = createSignal<ParsedDiff | undefined>(undefined);
  async function showDiff() {
    const c = conflict();
    if (!target || !c) return;
    const theirs = await fsRead(target.hostId, target.path);
    const theirsText = theirs.ok ? theirs.value.text : '';
    const hunk = editDiff(theirsText, c.mine);
    setDiffView({ files: [{ oldPath: target.path, newPath: target.path, hunks: [hunk] }] });
  }

  function mentionInChat() {
    if (!target) return;
    const open = openChatState();
    if (open && open.target.hostId === target.hostId) {
      queueMention(`${open.target.hostId}:${open.target.session}`, `@${target.path}`);
      showToast('Added @path to the chat draft');
      return;
    }
    void navigator.clipboard?.writeText(`@${target.path}`).then(
      () => showToast('Copied @path — paste it into a chat'),
      () => showToast('Could not copy'),
    );
  }

  const fileName = () => (target ? target.path.split('/').pop() ?? target.path : '');
  // UNI-265: Markdown opens rendered (Preview) with an Edit toggle; the
  // preview shows the editor's current text, unsaved edits included.
  const isMarkdown = () => /\.(md|markdown|mdx)$/i.test(target?.path ?? '');
  const [preview, setPreview] = createSignal(isMarkdown());
  const [previewText, setPreviewText] = createSignal('');
  const togglePreview = () => {
    haptic();
    if (!preview()) setPreviewText(view?.state.doc.toString() ?? '');
    setPreview(!preview());
    if (!preview()) queueMicrotask(() => view?.focus());
  };
  createEffect(() => {
    // First load: the preview shows the file as read.
    const st = state();
    if (st.kind === 'ready' && preview() && !previewText()) setPreviewText(view?.state.doc.toString() ?? (st as { text?: string }).text ?? '');
  });

  return (
    <div class="editor-screen" data-testid="editor-screen">
      <header class="editor-head detail-head">
        <button type="button" class="ghost-icon-button" aria-label="Back to files" onClick={close}>
          <ChevronLeft size={22} />
        </button>
        <div class="editor-title">
          <span class="editor-title-name" title={fileName()}>
            <span class="editor-title-text">{fileName()}</span>
            <Show when={dirty()}>
              <span class="editor-dirty-dot" aria-label="Unsaved changes" />
            </Show>
          </span>
          <span class="editor-title-path truncate">{target?.path}</span>
        </div>
        <Show when={isMarkdown() && state().kind === 'ready'}>
          <button type="button" class="editor-mode-btn" aria-label={preview() ? 'Edit' : 'Preview'} aria-pressed={preview()} data-testid="editor-preview-toggle" onClick={togglePreview}>
            <Show when={preview()} fallback={<Eye size={16} />}>
              <Pencil size={16} />
            </Show>
            <span>{preview() ? 'Edit' : 'Preview'}</span>
          </button>
        </Show>
        <button type="button" class="ghost-icon-button" aria-label="Toggle word wrap" onClick={toggleWrap}>
          <WrapText size={20} />
        </button>
        <button type="button" class="ghost-icon-button" aria-label="Mention in chat" onClick={mentionInChat}>
          <MessageSquarePlus size={20} />
        </button>
      </header>

      <Show when={state().kind === 'ready' && (state() as { truncated?: boolean }).truncated}>
        <p class="editor-readonly-note">This file is big — showing the start only, read-only.</p>
      </Show>

      <Show when={state().kind === 'ready' && forcedReadOnly()}>
        <p class="editor-readonly-note" data-testid="editor-as-text-note">
          This file wasn't text — showing it decoded, read-only.{' '}
          <button type="button" class="underline" onClick={editAnyway}>Edit anyway</button>
          {' '}(saving will rewrite the file as text).
        </p>
      </Show>

      <div class="editor-body">
        <Show when={state().kind === 'loading'}>
          <p class="p-4 text-text-muted">Loading…</p>
        </Show>
        <Show when={state().kind === 'error'}>
          <p class="p-4 text-wm-red">{(state() as { message: string }).message}</p>
        </Show>
        <div class="editor-cm" ref={container} classList={{ hidden: state().kind !== 'ready' || (isMarkdown() && preview()) }} />
        <Show when={state().kind === 'ready' && isMarkdown() && preview()}>
          <div class="editor-md-preview md" data-testid="editor-md-preview">
            <For each={renderBlocks(previewText(), false)}>{(html) => <MdBlock html={html} class="chat-block" />}</For>
          </div>
        </Show>
      </div>

      <Show when={state().kind === 'ready' && !(state() as { truncated?: boolean }).truncated && !(isMarkdown() && preview())}>
        <div class="editor-toolbar" role="toolbar" aria-label="Editor tools">
          <div class="editor-toolbar-keys">
          <Show when={vimOn() && vimMode()}>
            <span class="editor-vim-mode" data-testid="editor-vim-mode">{vimMode()}</span>
          </Show>
          <Show when={vimOn()}>
            <button type="button" class="editor-key editor-key-esc" aria-label="Esc" onClick={pressVimEsc}>Esc</button>
          </Show>
          <button type="button" class="editor-key editor-key-wide" onClick={() => insertAtCursor('\t')}>Tab</button>
          <button type="button" class="editor-key" onClick={() => wrapSelection('{', '}')}>{'{}'}</button>
          <button type="button" class="editor-key" onClick={() => wrapSelection('(', ')')}>()</button>
          <button type="button" class="editor-key" onClick={() => wrapSelection('[', ']')}>[]</button>
          <button type="button" class="editor-key" onClick={() => wrapSelection('"', '"')}>"</button>
          <button type="button" class="editor-key" onClick={() => wrapSelection("'", "'")}>'</button>
          <button type="button" class="editor-key" aria-label="Left" onClick={() => moveCursor(-1, 0)}>←</button>
          <button type="button" class="editor-key" aria-label="Right" onClick={() => moveCursor(1, 0)}>→</button>
          <button type="button" class="editor-key" aria-label="Up" onClick={() => moveCursor(0, -1)}>↑</button>
          <button type="button" class="editor-key" aria-label="Down" onClick={() => moveCursor(0, 1)}>↓</button>
          <button type="button" class="editor-key" aria-label="Undo" onClick={() => view && undo(view)}><Undo2 size={16} /></button>
          <button type="button" class="editor-key" aria-label="Redo" onClick={() => view && redo(view)}><Redo2 size={16} /></button>
          </div>
          {/* UNI-189: Find + Save stay pinned at the right; only the keys scroll. */}
          <div class="editor-toolbar-end">
          <button type="button" class="editor-key editor-key-wide" aria-label="Find" onClick={() => view && openSearchPanel(view)}>Find</button>
          <button type="button" class="editor-key editor-key-save" disabled={saving() || !dirty()} onClick={() => void save()}>
            {saving() ? '…' : 'Save'}
          </button>
          </div>
        </div>
      </Show>

      <BoardSheet open={leaveAsk()} onClose={() => ((pendingNext = undefined), setLeaveAsk(false))} label="Unsaved changes" testId="editor-leave-sheet">
        <p class="p-2 text-text-muted">Save your changes to {target?.path.split('/').pop()} before leaving?</p>
        <div class="files-sheet-actions">
          <button type="button" class="files-sheet-btn" onClick={() => ((pendingNext = undefined), setLeaveAsk(false))}>Keep editing</button>
          <button type="button" class="files-sheet-btn danger" onClick={() => leave()}>Discard</button>
          <button type="button" class="files-sheet-btn primary" onClick={() => void saveAndLeave()}>Save</button>
        </div>
      </BoardSheet>

      <Show when={conflict()}>
        <BoardSheet open onClose={() => setConflict(undefined)} label="Someone else changed this file" testId="editor-conflict-sheet">
          <p class="p-2 text-text-muted">The file on the PC changed since you opened it. What would you like to do?</p>
          <Show when={diffView()}>
            {(d) => <div class="md" innerHTML={renderDiffHtml(d())} />}
          </Show>
          <div class="conflict-sheet-row">
            <button type="button" class="conflict-sheet-btn primary" onClick={() => void keepMine()}>Keep mine (overwrite)</button>
            <button type="button" class="conflict-sheet-btn" onClick={() => void loadTheirs()}>Load theirs</button>
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
