/**
 * The composer (WP-A12, app spec §5): the conversation's send box in the
 * bottom dock — a chip-aware field (`ChipEditor`), placeholder by state
 * (Z-57), ↑/↓ prompt history per workspace (Z-58), persisted text drafts
 * (Z-59, `lib/composer/drafts.ts`), long-paste chips (Z-61, `lib/composer/
 * paste.ts`), the `/` and `@` menus (Z-62: commands/prompt templates/skills
 * and files — no `#`/`$` triggers) and the `+` menu (Z-63: Photo, Camera,
 * File, Side question, Board task… — no New session/Resume). Dictation is
 * not built (Z-65); there are no suggested prompts under an empty draft
 * (Z-68). Undo/redo is kept (`lib/composer/undo.ts`): the field's DOM is
 * rebuilt under chips and pastes, so the history lives here.
 *
 * Enter sends (auto: a steer while a run is live — the session store picks),
 * Shift+Enter is a newline, Esc stops a running turn (after closing a menu;
 * WP-A9's rules until WP-A11 refines them).
 *
 * WP-A13 owns the frame around that: attachments with upload states, max 8
 * and the send gate (Z-60, `lib/composer/attachments.ts` — a per-chat store,
 * so a picked image survives a chat switch, D-6), the image editor's ellipse
 * tool (D-5, `./ImageEditor`), the toolbar with the seven-step compression
 * (Z-64, `./Toolbar`) and the permission/plan control bound to the workflow
 * extension (G-6, `./PermissionControl`).
 */
import { createEffect, createMemo, createSignal, on, onCleanup, onMount, For, Show, untrack, type JSX } from 'solid-js';
import { Paperclip, Quote } from 'lucide-solid';
import type { SessionHandle } from '../../lib/session/store';
import {
  MAX_ATTACHMENTS,
  attachmentsReady,
  attach,
  pendingAttachmentsFor,
  removeAttachment,
  replaceImage,
  retryAttachment,
  withAttachmentMentions,
} from '../../lib/composer/attachments';
import { allCommands, applyAt, commandNeedsTask, argQuery, atQuery, isAppBuiltin, isKnownCommand, rankCommands, slashQuery, wantsArgCompletions } from '../../lib/chat/commands';
import {
  builtinPlan,
  builtinWorksWithoutChat,
  runBuiltin,
  runSessionBuiltin,
  type BuiltinDeps,
  type SessionBuiltinDeps,
} from '../../lib/composer/builtins';
import { matchModel } from '../../lib/model/find';
import { rememberCommands } from '../draft/hostCache';
import { connectComposerMentions } from '../../lib/chat/mention';
import { connectEditorTokens } from '../../lib/composer/tokens';
import { modelName, parseModels } from '../../lib/model/picker';
import { parseLevels, setThinkingLevel, thinkingLabel } from '../../lib/model/thinking';
import { appNavigate } from '../../lib/nav';
import { setPaletteOpen } from '../../lib/shell';
import { copyText } from '../../lib/board/share';
import { forkSession, renameSession } from '../../lib/session/actions';
import { openRenameDialog, NAME_MAX } from '../session/RenameDialog';
import { openSessionModal } from '../session/modalTarget';
import { openBtwTab, openTreeTab } from '../panes/tabs/sessionTabs';
import { chatActionsFor } from '../shell/ConversationHeader';
import { openImportSession } from '../command/ImportSession';
import { openSettingsAt } from '../../lib/settings/open';
import { ABOUT_ANCHORS } from '../../screens/settings/anchors';
import { setShortcutsOpen } from '../shell/ShortcutsDialog';
import { openExternal } from '../../lib/board/share';
import { can, isTauriDesktop } from '../../lib/platform';
import { clipboardFiles, describeLocal, readLocalFiles, registerDropZone, startNativeDrop } from '../../lib/localFiles';
import { z } from 'zod';
import { call } from '../../lib/ipc';
import { commandNames } from '../timeline/context';
import { fsList, fsSearch, fsStat } from '../../lib/fs';
import { toAppError } from '@unipi/protocol';
import { addTaskFromChat } from '../../lib/session/open';
import { followUpMode, sendModeForEnter } from '../../lib/composer/sendMode';
import { SendButton } from './SendButton';
import { showActionNotice } from '../../lib/timeline/actions';
import {
  addComposerChip,
  clearComposer,
  composerDraft,
  composerSendText,
  focusComposer,
  registerComposerFocus,
  removeComposerChip,
  removeComposerQuote,
  setComposerText,
  type ComposerChip,
} from '../../lib/composer/drafts';
import { isLongPaste, pastedTextLabel } from '../../lib/composer/paste';
import { promptHistory, walkDown, walkUp } from '../../lib/composer/history';
import { composerHistory } from '../../lib/composer/undo';
import { pathCandidates } from '../../lib/composer/chips';
import { ChipEditor, type ChipEditorApi } from './ChipEditor';
import { SlashMenu } from './SlashMenu';
import { noteCommandRun } from '../../lib/reports';
import { ArgMenu, argItemsOf, type ArgItem } from './ArgMenu';
import { MentionMenu, type MentionItem } from './MentionMenu';
import { PlusMenu } from './PlusMenu';
import { PastedTextChip } from './PastedTextChip';
import { AttachmentChip } from './AttachmentChip';
import { ImageEditor } from './ImageEditor';
import { PermissionControl, PlanToggle } from './PermissionControl';
import { Toolbar, ToolbarSlot } from './Toolbar';
import { ModelChip } from '../model/ModelChip';
import { ThinkingChip } from '../model/ThinkingChip';
import { ContextRing } from '../model/ContextRing';
import { openModelPicker } from '../model/target';
import './composer.css';

/** A paste's tooltip head (Z-61): the first lines, whitespace-collapsed. */
const pasteHead = (text: string): string => {
  const flat = text.replace(/\s+/g, ' ').trimEnd();
  return flat.length > 220 ? `${flat.slice(0, 219).trimEnd()}…` : flat;
};

const errorText = (error: unknown): string => String(toAppError(error).message || 'something went wrong');

/** Local files the composer reads into memory at most (a paste / a drop): the
 *  attachment pipeline uploads `File`s, so anything bigger is refused. */
const MAX_LOCAL_READ = 200 * 1024 * 1024;

export type ComposerProps = {
  session: SessionHandle;
  cwd?: () => string | undefined;
  readOnly?: boolean;
  phone?: boolean;
  /**
   * The new-chat draft (`components/draft`): the session handle is a local
   * stand-in whose `send` starts the real session. Pi built-ins that need a
   * chat (`/model`, `/btw`, the ⋯ actions) and the context ring stay out.
   */
  draft?: boolean;
  /** F-01: why a plain message can't leave yet (no provider…). Send is disabled and the typed text is never touched. `/` commands still run. */
  sendHold?: () => string | undefined;
  /** Draft: replaces the model chip (the session-bound picker can't open yet). */
  modelSlot?: JSX.Element;
  /** A row inside the card, above the field (the draft's workspace picker). */
  header?: JSX.Element;
  /** Overrides the state placeholder. */
  placeholder?: string;
  /** Send is held (the draft while its session is being created). */
  busy?: boolean;
  /** Focus the field on mount (the draft). */
  autofocus?: boolean;
};

export function Composer(props: ComposerProps) {
  const key = () => props.session.key;
  const draft = () => composerDraft(key());
  let editor: ChipEditorApi | undefined;
  const [caret, setCaret] = createSignal(0);
  /** Menus dismissed for this exact draft text (Esc) until it changes. */
  const [dismissed, setDismissed] = createSignal<string | null>(null);
  const running = () => !!props.session.state.run.running;
  const cwd = () => (props.cwd ? props.cwd() : undefined) ?? '';
  const hostId = () => props.session.hostId;

  /* ── placeholder by state (Z-57) ──────────────────────────────────── */
  const hasHistory = createMemo(() => props.session.turns().some((t) => !!t.user));
  const placeholder = () =>
    props.placeholder ??
    (props.readOnly
      ? 'Read-only — this session is open elsewhere'
      : running()
        ? `Keep typing to ${followUpMode() === 'steer' ? 'steer' : 'queue'}…`
        : !hasHistory()
          ? props.phone
            ? 'Ask anything…'
            : 'Ask anything — / for commands, @ for files'
          : 'Ask for follow-up changes');

  /* ── undo/redo (kept from UNI-270; the store is lib/composer/undo) ── */
  const history = () => composerHistory(key());
  const [histRev, setHistRev] = createSignal(0);
  const commit = (text: string, at: number, kind: 'type' | 'delete' | 'other' = 'other') => {
    history().record(text, at, kind);
    setHistRev((n) => n + 1);
    setComposerText(key(), text);
  };
  // Changes made outside the field (a quote, Edit's restore, a history walk)
  // are steps of their own.
  createEffect(() => {
    const text = draft().text;
    if (history().present?.text === text) return;
    history().record(text, text.length, 'other');
    setHistRev((n) => n + 1);
  });
  const travel = (op: 'undo' | 'redo') => {
    const st = op === 'undo' ? history().undo() : history().redo();
    if (!st) return;
    setHistRev((n) => n + 1);
    setDismissed(null);
    setComposerText(key(), st.text);
    queueMicrotask(() => {
      editor?.setCaret(st.caret);
      setCaret(st.caret);
    });
  };

  /* ── `/` commands (Z-62: commands, prompt templates, skills) ──────── */
  const commands = createMemo(() => commandNames(props.session.state.commands));
  // P0-09: the new-chat draft offers this PC's `/` commands before any chat is open.
  createEffect(() => {
    if (props.draft) return;
    const list = props.session.state.commands;
    if (list.length) untrack(() => rememberCommands(hostId(), list));
  });
  const [verified, setVerified] = createSignal<ReadonlySet<string>>(new Set(), { equals: false });
  const markVerified = (token: string) => {
    const next = new Set(verified());
    next.add(token);
    setVerified(next);
  };
  const tokenOpts = () => ({ commands: commands(), verified: (t: string) => verified().has(t), requireTrailingSpace: true });

  const slash = createMemo(() => slashQuery(draft().text, caret()));
  const slashItems = createMemo(() => {
    const q = slash();
    if (!q || dismissed() === draft().text) return [];
    // The draft lists only the built-ins that work before a chat exists (P0-02: no dead rows).
    return rankCommands(allCommands(props.session.state.commands, props.draft ? builtinWorksWithoutChat : undefined), q.query, 60);
  });
  const pickCommand = (name: string) => {
    const q = slash();
    if (!q) return;
    const next = `${draft().text.slice(0, q.start)}/${name} ${draft().text.slice(caret())}`.replace(/\s+$/, ' ');
    commit(next, q.start + name.length + 2);
    queueMicrotask(() => {
      editor?.setCaret(q.start + name.length + 2);
      setCaret(q.start + name.length + 2);
    });
  };

  /* ── `/cmd <arg>` completions (P1-01: the extension's complete-command) ── */
  const arg = createMemo(() => {
    if (props.draft) return null;
    const q = argQuery(draft().text, caret());
    if (!q) return null;
    const c = props.session.state.commands.find((x) => x.name === q.command);
    return c && wantsArgCompletions(c.source) ? q : null;
  });
  const [argItems, setArgItems] = createSignal<{ command: string; from: number; items: ArgItem[] } | null>(null);
  let argSeq = 0;
  createEffect(
    on(
      () => {
        const q = arg();
        return q && dismissed() !== draft().text ? `${q.command}\u0000${q.prefix}` : null;
      },
      (keyed) => {
        const seq = ++argSeq;
        const q = arg();
        if (keyed === null || !q) {
          setArgItems(null);
          return;
        }
        const text = draft().text;
        const before = text.slice(0, caret());
        const timer = setTimeout(() => {
          void props.session
            .cmd<unknown>({ type: '_oino/get_completions', text: before, cursor: [...before].length })
            .then((reply) => {
              if (seq !== argSeq) return;
              const got = argItemsOf(reply, q.command, q.prefix);
              // `from` is a character offset; the draft is indexed in UTF-16 units.
              const from = got.from === null ? q.start : [...before].slice(0, got.from).join('').length;
              setArgItems(got.items.length ? { command: q.command, from, items: got.items } : null);
            })
            .catch(() => {
              if (seq === argSeq) setArgItems(null);
            });
        }, 90);
        onCleanup(() => clearTimeout(timer));
      },
    ),
  );
  const argMenu = () => (arg() && dismissed() !== draft().text ? argItems() : null);
  const pickArg = (item: ArgItem) => {
    const menu = argMenu();
    if (!menu) return;
    const text = draft().text;
    const at = caret();
    const insert = /\s$/.test(item.value) ? item.value : `${item.value} `;
    const next = `${text.slice(0, menu.from)}${insert}${text.slice(at).replace(/^\s+/, '')}`;
    const pos = menu.from + insert.length;
    commit(next, pos);
    queueMicrotask(() => {
      editor?.setCaret(pos);
      setCaret(pos);
    });
  };

  /* ── `@` files (Z-62: fs_list for a bare @, fs_search for a query) ── */
  const at = createMemo(() => atQuery(draft().text, caret()));
  const [mentionItems, setMentionItems] = createSignal<MentionItem[]>([]);
  const mentionOpen = () => !!at() && dismissed() !== draft().text;
  let mentionSeq = 0;
  createEffect(
    on(
      () => (mentionOpen() ? at()!.query : null),
      (query) => {
        const seq = ++mentionSeq;
        if (query === null) {
          setMentionItems([]);
          return;
        }
        const root = cwd();
        const timer = setTimeout(() => {
          if (!root) return;
          const ask =
            query
              ? fsSearch(hostId(), root, query.replace(/^"|"$/g, '').replace(/\/$/, ''), 'name').then(
                  (r): Array<{ path: string; dir: boolean }> => (r.ok ? r.value.results.map((x) => ({ path: x.path, dir: false })) : []),
                )
              : fsList(hostId(), root).then(
                  (r): Array<{ path: string; dir: boolean; leaf: string }> =>
                    r.ok ? r.value.entries.map((e) => ({ path: e.name, dir: e.kind === 'dir', leaf: e.name })) : [],
                );
          void ask
            .then((entries) => rankMentions(query, entries))
            .then((items) => {
              if (seq === mentionSeq) setMentionItems(items);
            })
            .catch(() => {
              if (seq === mentionSeq) setMentionItems([]);
            });
        }, 140);
        onCleanup(() => clearTimeout(timer));
      },
    ),
  );
  const pickMention = (item: MentionItem) => {
    const q = at();
    if (!q) return;
    const value = /[\s"]/.test(item.value) ? `@"${item.value}"` : `@${item.value}`;
    const next = applyAt(draft().text, caret(), q, value, item.dir);
    markVerified(value);
    commit(next.text, next.caret);
    queueMicrotask(() => {
      editor?.setCaret(next.caret);
      setCaret(next.caret);
    });
  };
  // A typed `@path` (not picked) chips once the host confirms it exists.
  const asked = new Set<string>();
  createEffect(
    on(
      () => draft().text,
      (text) => {
        const root = cwd();
        if (!root) return;
        const timer = setTimeout(() => {
          for (const token of pathCandidates(text)) {
            if (asked.has(token) || verified().has(token)) continue;
            asked.add(token);
            const path = token.slice(1).replace(/"/g, '');
            void fsStat(hostId(), path)
              .then(() => markVerified(token))
              .catch(() => {
                // not found: stays plain text
              });
          }
        }, 350);
        onCleanup(() => clearTimeout(timer));
      },
    ),
  );

  /* ── menu keyboard selection (one menu open at a time) ────────────── */
  const menuItems = createMemo<MentionItem[] | undefined>(() => (mentionOpen() && mentionItems().length ? mentionItems() : undefined));
  /** The arg menu yields to an `@` menu typed inside the argument. */
  const argRows = () => (menuItems() ? undefined : argMenu()?.items);
  const [sel, setSel] = createSignal(0);
  createEffect(() => {
    void slashItems().length;
    void mentionItems().length;
    void argMenu();
    setSel(0);
  });

  /* ── prompt history (Z-58, lib/composer/history) ──────────────────── */
  const promptStore = () => promptHistory(`${hostId()}:${cwd()}`);
  let histIndex: number | null = null;
  let histDraft = '';
  const recall = (list: string[], index: number | null) => {
    histIndex = index;
    const text = index === null ? histDraft : (list[index] ?? histDraft);
    setDismissed(null);
    setComposerText(key(), text);
    queueMicrotask(() => {
      editor?.setCaret(text.length);
      setCaret(text.length);
    });
  };
  const enterWalk = () => {
    const list = promptStore().list();
    if (!list.length) return false;
    histDraft = draft().text;
    recall(list, walkUp(list, null));
    return true;
  };
  const historyKey = (e: KeyboardEvent, text: string): boolean => {
    const list = promptStore().list();
    if (!list.length) return false;
    if (e.key === 'ArrowUp' && (histIndex !== null || !text)) {
      if (histIndex === null && !text) {
        if (!enterWalk()) return false;
      } else recall(list, walkUp(list, histIndex));
      return true;
    }
    if (e.key === 'ArrowDown' && histIndex !== null) {
      recall(list, walkDown(list, histIndex));
      return true;
    }
    return false;
  };

  /* ── paste → chip (Z-61) ──────────────────────────────────────────── */
  const onPasteText = (text: string): boolean => {
    if (props.readOnly || !isLongPaste(text)) return false;
    addComposerChip(key(), { kind: 'paste', label: pastedTextLabel(text), detail: pasteHead(text), markdown: text });
    return true;
  };

  /* ── attachments (Z-60 states, D-6 per-chat store) ───────────────── */
  const attachments = () => pendingAttachmentsFor(key());
  /** Pre-uploads what was picked (images shrink+encode, files over the blob
   *  channel) and says so when something does not fit. */
  const attachPicked = (files: readonly File[]) => {
    if (props.readOnly || !files.length) return;
    const { overLimit, refused } = attach(key(), hostId(), files);
    if (overLimit > 0) showActionNotice(`Up to ${MAX_ATTACHMENTS} attachments per message — ${overLimit} left out`);
    else if (refused.length === 1) showActionNotice(`${refused[0]!.name} is larger than 1 GB`);
    focusComposer(key());
  };
  /** Local files from the OS (a native paste, a drop on the desktop app):
   *  read into `File`s and attached like picked ones. */
  const attachLocal = async (files: ReadonlyArray<{ path: string; name: string; size: number; isDir: boolean }>): Promise<boolean> => {
    const usable = files.filter((f) => !f.isDir && f.size <= MAX_LOCAL_READ);
    const skipped = files.length - usable.length;
    if (skipped) showActionNotice(skipped === 1 ? `${files.find((f) => f.isDir || f.size > MAX_LOCAL_READ)!.name} can't be attached (a folder, or over 200 MB)` : `${skipped} items can't be attached (folders, or over 200 MB)`);
    if (!usable.length) return skipped > 0;
    attachPicked(await readLocalFiles(usable));
    return true;
  };
  /** UNI-256: WebKitGTK hides a clipboard image from the page — the core reads it. */
  const pasteNative = async (): Promise<boolean> => {
    if (props.readOnly) return false;
    const files = await clipboardFiles();
    if (!files.length) return false;
    return attachLocal(files);
  };
  /* ── drag & drop (web: DataTransfer files; desktop app: Tauri's window drop) ── */
  let wrap: HTMLDivElement | undefined;
  const [dropOver, setDropOver] = createSignal(false);
  let dragDepth = 0;
  const dragHasFiles = (e: DragEvent) => !!e.dataTransfer && Array.from(e.dataTransfer.types ?? []).includes('Files');
  const dropHandlers = {
    onDragEnter: (e: DragEvent) => {
      if (props.readOnly || !dragHasFiles(e)) return;
      e.preventDefault();
      dragDepth += 1;
      setDropOver(true);
    },
    onDragOver: (e: DragEvent) => {
      if (props.readOnly || !dragHasFiles(e)) return;
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
    },
    onDragLeave: () => {
      dragDepth = Math.max(0, dragDepth - 1);
      if (!dragDepth) setDropOver(false);
    },
    onDrop: (e: DragEvent) => {
      dragDepth = 0;
      setDropOver(false);
      if (props.readOnly || !dragHasFiles(e)) return;
      e.preventDefault();
      const files = [...(e.dataTransfer?.files ?? [])];
      if (files.length) attachPicked(files);
    },
  };
  onMount(() => {
    if (!isTauriDesktop()) return;
    void startNativeDrop();
    const off = registerDropZone({
      el: () => wrap,
      enabled: () => !props.readOnly,
      hover: setDropOver,
      drop: (paths) => {
        void describeLocal(paths)
          .then((files) => attachLocal(files))
          .catch((error: unknown) => {
            console.warn('[drop] reading the dropped files failed', error);
            showActionNotice("Couldn't read the dropped files");
          });
      },
    });
    onCleanup(off);
  });
  /** The image editor (D-5): annotate a pending image in place. */
  const [editing, setEditing] = createSignal<{ id: string; src: string; mime: string; name: string } | undefined>();
  const openEditor = (id: string) => {
    const image = attachments().find((a) => a.id === id);
    if (!image?.preview) return;
    setEditing({ id, src: image.preview, mime: image.mime, name: image.name });
  };
  let photoInput: HTMLInputElement | undefined;
  let cameraInput: HTMLInputElement | undefined;
  let fileInput: HTMLInputElement | undefined;

  /* ── the `+` menu (Z-63) ──────────────────────────────────────────── */
  const [plusAnchor, setPlusAnchor] = createSignal<DOMRect | null>(null);
  // Z-31: the `btw` side chat (WP-A10) — a pane tab on the desktop, a sheet on
  // the phone; the extension's own `editorReplace` panel is never opened.
  const openSideQuestion = (question?: string) => {
    openBtwTab({ hostId: hostId(), sessionId: props.session.sessionId(), cwd: cwd() }, question);
  };

  /* ── focus handoff (A7: quotes and Edit focus the field) ──────────── */
  createEffect(() => {
    const k = key();
    const cleanup = registerComposerFocus(k, () => editor?.focus());
    onCleanup(cleanup);
  });

  /* ── "Mention in chat" and `insertEditorToken` (P0-07 / P0-08) ────── */
  /** Inserts text at the caret (or the end), space-separated from its neighbours, as one undo step. */
  const insertAtCaret = (insert: string) => {
    if (props.readOnly || !insert) return;
    const text = draft().text;
    const at = Math.max(0, Math.min(editor?.getCaret() ?? text.length, text.length));
    const before = text.slice(0, at);
    const after = text.slice(at);
    const lead = before && !/\s$/.test(before) && !/^\s/.test(insert) ? ' ' : '';
    // A trailing space too, so typing goes on after the token (and a chip forms).
    const trail = !/^\s/.test(after) && !/\s$/.test(insert) ? ' ' : '';
    const caretAt = (before + lead + insert + trail).length;
    commit(before + lead + insert + trail + after, caretAt);
    setDismissed(null);
    queueMicrotask(() => {
      editor?.syncFromStore();
      editor?.setCaret(caretAt);
      setCaret(caretAt);
    });
  };
  // Files / Board / file-ref menus ("Mention in chat") and extensions' insertEditorToken
  // land in this composer while it is mounted (keyed like the draft store: host/session, draft:host).
  connectComposerMentions(key, insertAtCaret);
  connectEditorTokens(key, insertAtCaret);

  /* ── pi built-ins (WP-A25, G-15) ──────────────────────────────────── */
  /** The effects a built-in needs. The chat is always this session (a built-in
   *  is typed into a chat); the rest are app surfaces. */
  const builtinDeps = (): BuiltinDeps => {
    const chat = { hostId: hostId(), sessionId: props.session.sessionId() };
    const ctx = { hostId: chat.hostId, sessionId: chat.sessionId, title: '', cwd: cwd() || undefined };
    return {
      chat,
      openChatAction: (id) => {
        const row = chatActionsFor(ctx).find((entry) => entry.id === id);
        if (row) {
          row.action.run(ctx);
          return true;
        }
        // No ⋯ row plugged in yet: `/export` still exports (one surface, one call).
        if (id === 'export') {
          void import('../../lib/session/exportSession').then((m) => m.exportSession(chat.hostId, chat.sessionId));
          return true;
        }
        return false;
      },
      openImport: () => openImportSession(),
      openPickerCycleSet: () => {
        openModelPicker({ ...chat, focus: 'cycle' });
        return true;
      },
      openSettings: (section, anchor) => openSettingsAt(section, anchor),
      openHotkeys: () => setShortcutsOpen(true),
      reportBug: (url) => {
        openSettingsAt('about', ABOUT_ANCHORS['report-bug']);
        openExternal(url);
      },
      notice: showActionNotice,
      quit: () => {
        if (can('tray')) void call('quit_now', undefined, z.null());
        else showActionNotice('Quit is in the app window’s menu');
      },
      runInChat: (command) => {
        props.session.send(`/${command}`, { mode: 'prompt' });
        return true;
      },
    };
  };

  /** P0-02: the chat built-ins (`SESSION_BUILTINS`) — oino RPCs and app actions. */
  const sessionBuiltinDeps = (): SessionBuiltinDeps => {
    const s = props.session;
    const chat = () => ({ hostId: hostId(), sessionId: s.sessionId() });
    /** Opens one of this composer's own toolbar menus (the thinking / draft model chips). */
    const clickChip = (testId: string): boolean => {
      const el = wrap?.querySelector<HTMLButtonElement>(`[data-testid="${testId}"]`);
      if (!el || el.disabled) return false;
      el.click();
      return true;
    };
    const failed = (what: string) => (error: unknown) => showActionNotice(`${what}: ${errorText(error)}`);
    return {
      model: (query) => {
        if (!query) {
          if (props.draft) {
            if (!clickChip('model-chip')) showActionNotice('No model list yet');
          } else openModelPicker(chat());
          return;
        }
        void s
          .cmd<unknown>({ type: 'get_available_models' })
          .then((reply) => {
            const match = matchModel(parseModels(reply), query, s.state.model);
            if ('ambiguous' in match) {
              // D-13: same-named rows (two routes to one model) are told apart by provider · id.
              const shown = match.ambiguous.slice(0, 3);
              const names = shown
                .map((m) => (shown.filter((o) => modelName(o) === modelName(m)).length > 1 ? `${modelName(m)} (${m.provider} · ${m.id})` : `${modelName(m)} (${m.provider})`))
                .join(', ');
              showActionNotice(`“${query}” matches ${match.ambiguous.length} models — ${names}${match.ambiguous.length > 3 ? '…' : ''}. Type more, or /model to pick`);
              return;
            }
            if (!('hit' in match)) {
              showActionNotice(`No model matches “${query}”`);
              return;
            }
            const hit = match.hit;
            return s.cmd({ type: 'set_model', provider: hit.provider, modelId: hit.id }).then(() => showActionNotice(`Model: ${modelName(hit)}`));
          })
          .catch(failed('Could not switch the model'));
      },
      thinking: (level) => {
        if (!level) {
          if (!clickChip('thinking-chip')) showActionNotice('This model has no thinking levels');
          return;
        }
        const want = level.toLowerCase().replace(/^x-high$/, 'xhigh');
        void s
          .cmd<unknown>({ type: 'get_available_thinking_levels' })
          .then((reply) => {
            const levels = parseLevels(reply);
            if (levels.length && !levels.includes(want)) {
              showActionNotice(`Thinking levels here: ${levels.map(thinkingLabel).join(', ')}`);
              return;
            }
            return setThinkingLevel(s, want).then(() => showActionNotice(`Thinking: ${thinkingLabel(want)}`));
          })
          .catch(failed('Could not set the thinking level'));
      },
      compact: (focus) => {
        void s.cmd({ type: 'compact', ...(focus ? { customInstructions: focus } : {}) }).then(
          () => showActionNotice('Compacted'),
          (error: unknown) => {
            // D-08: a chat too short to compact is not a failure.
            if (/nothing to compact|too small|too short/i.test(errorText(error))) showActionNotice('Nothing to compact yet');
            else failed('Compaction failed')(error);
          },
        );
      },
      newChat: () => {
        if (props.draft) {
          focusComposer(key());
          return;
        }
        const here = cwd();
        appNavigate(`/new/${encodeURIComponent(hostId())}${here ? `?cwd=${encodeURIComponent(here)}` : ''}`);
      },
      resume: () => setPaletteOpen(true, 'sessions'),
      fork: () => {
        const ctx = { ...chat(), title: s.state.name ?? '', cwd: cwd() || undefined };
        const row = chatActionsFor(ctx).find((entry) => entry.id === 'fork');
        if (row) row.action.run(ctx);
        else void forkSession(ctx.hostId, ctx.sessionId);
      },
      tree: () => openTreeTab({ ...chat(), cwd: cwd() || undefined }),
      rename: (title) => {
        if (!title) {
          openRenameDialog({ ...chat(), name: s.state.name ?? '' });
          return;
        }
        void renameSession(hostId(), s.sessionId(), title.slice(0, NAME_MAX)).then(() => showActionNotice(`Renamed to “${title.slice(0, 60)}”`), failed('Could not rename'));
      },
      copyLast: () => {
        void s
          .cmd<{ text?: string | null }>({ type: 'get_last_assistant_text' })
          .then((reply) => {
            const text = reply && typeof reply.text === 'string' ? reply.text : '';
            if (!text.trim()) {
              showActionNotice('No reply to copy yet');
              return;
            }
            return copyText(text).then(() => showActionNotice('Copied the last reply'));
          })
          .catch(failed("Couldn't copy"));
      },
      sessionInfo: () => openSessionModal({ ...chat(), tab: 'info' }),
    };
  };

  /* ── send ─────────────────────────────────────────────────────────── */
  const send = (mode?: 'prompt' | 'steer' | 'followUp') => {
    if (props.readOnly || props.busy) return;
    // Z-60: nothing leaves until every attachment is ready.
    if (!attachmentsReady(key())) return;
    const current = draft();
    const text = withAttachmentMentions(key(), composerSendText(current));
    const images = current.images;
    if (!text.trim() && !images.length) return;
    const cmd = /^\/(\S+)(?:\s+([\s\S]*))?$/.exec(text.trim());
    // `/btw` opens the side question instead of sending to the main chat (Z-31).
    if (!props.draft && cmd && (cmd[1] === 'btw' || cmd[1] === 'unipi:btw')) {
      openSideQuestion(cmd[2]?.trim() || undefined);
      afterRoute(text);
      return;
    }
    // E-05: `/bg-tasks` opens the TUI's work tray, which the app doesn't have — its Running tab is the same list.
    if (!props.draft && cmd && (cmd[1] === 'bg-tasks' || cmd[1] === 'unipi:bg-tasks') && !cmd[2]?.trim()) {
      openSessionModal({ hostId: props.session.hostId, sessionId: props.session.sessionId(), tab: 'running' });
      afterRoute(text);
      return;
    }
    if (cmd && isAppBuiltin(cmd[1]!, props.session.state.commands)) {
      const name = cmd[1]!;
      // P0-02: every engine built-in runs here (an app action or its oino RPC);
      // an extension command of the same name wins (the session owns it).
      if (props.draft && !builtinWorksWithoutChat(name)) {
        showActionNotice(`/${name} works inside a chat — send a first message to start one`);
        return;
      }
      // WP-A25 (G-15): the app surfaces (settings, export, import…).
      if (builtinPlan(name) ? runBuiltin(name, builtinDeps()) : runSessionBuiltin(name, cmd[2], sessionBuiltinDeps())) {
        afterRoute(text);
        return;
      }
      showActionNotice(`/${name} isn't available here`);
      return;
    }
    // A `/name` neither the session nor the app knows never becomes a prompt.
    if (cmd && !isKnownCommand(cmd[1]!, props.session.state.commands)) {
      showActionNotice(`Unknown command /${cmd[1]}`);
      return;
    }
    if (!cmd && props.sendHold?.()) return;
    if (cmd) noteCommandRun(key(), text.trim().split('\n')[0]!.slice(1, 80));
    const item = props.session.send(text, { ...(mode ? { mode } : {}), ...(images.length ? { images } : {}) });
    promptStore().add(text);
    // A send that failed keeps no history entry (compare-and-rollback once
    // the outbox item settles).
    pendingRollback.set(item.ref, text);
    afterSend(text);
  };
  const pendingRollback = new Map<string, string>();
  createEffect(() => {
    for (const item of props.session.state.outbox) {
      if (!pendingRollback.has(item.ref)) continue;
      if (item.status === 'lost') {
        const text = pendingRollback.get(item.ref)!;
        pendingRollback.delete(item.ref);
        untrack(() => promptStore().rollback(text));
      } else if (item.status === 'acked') {
        pendingRollback.delete(item.ref);
      }
    }
  });
  /** A workflow control's command (`/permission …`, `/plan …`, G-6): sent as a
   *  prompt straight to the session — it never touches the draft or history. */
  const runCommand = (command: string) => {
    if (props.readOnly) return;
    props.session.send(command, { mode: 'prompt' });
    focusComposer(key());
  };
  /** After a send the draft starts over; one Ctrl/⌘+Z brings back what was sent. */
  const afterSend = (outgoing: string) => {
    clearComposer(key());
    history().reset(outgoing);
    setHistRev((n) => n + 1);
    setDismissed(null);
    histIndex = null;
    // F-05/D-11: the field is rebuilt from the (now empty) draft — Enter's default handling must not leave the text or a stray newline behind.
    queueMicrotask(() => editor?.syncFromStore());
    setTimeout(() => {
      if (!draft().text) editor?.syncFromStore();
    }, 0);
  };
  /**
   * A `/command` the app routes (`/model`, `/btw`, every pi built-in, WP-A25)
   * is not sent: the draft is cleared here, outside an edit event, so the field
   * is rebuilt from the draft — otherwise Enter's default handling leaves the
   * command text in the box.
   */
  const afterRoute = (outgoing: string) => {
    afterSend(outgoing);
    queueMicrotask(() => editor?.syncFromStore());
  };
  const stop = () => {
    if (!props.readOnly) void props.session.abort();
  };
  /** "Send now (stops run)" (Z-52): stop, then send the text as a prompt. */
  const sendNow = () => {
    if (props.readOnly || !attachmentsReady(key())) return;
    const current = draft();
    const text = withAttachmentMentions(key(), composerSendText(current));
    if (!text.trim() && !current.images.length) return;
    afterSend(text);
    void props.session
      .abort()
      .then(() => {
        props.session.send(text, { mode: 'prompt', ...(current.images.length ? { images: current.images } : {}) });
        promptStore().add(text);
      })
      .catch(() => undefined);
  };

  /* ── keys ─────────────────────────────────────────────────────────── */
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      if (slashItems().length || menuItems()?.length || argRows()?.length) {
        e.preventDefault();
        setDismissed(draft().text);
        if (!props.phone) e.stopPropagation();
        return;
      }
      // No menu: the dock root's escape-stop rule (WP-A11, Z-55) takes it.
      return;
    }
    if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
      const items = slashItems();
      const mentions = menuItems();
      const args = argRows();
      if (items.length || mentions?.length || args?.length) {
        e.preventDefault();
        const count = items.length || mentions?.length || args!.length;
        setSel((i) => (e.key === 'ArrowUp' ? (i <= 0 ? count - 1 : i - 1) : i + 1 >= count ? 0 : i + 1));
        return;
      }
      if (!e.shiftKey && historyKey(e, draft().text)) e.preventDefault();
      return;
    }
    if (e.key === 'Tab' && !e.shiftKey && argRows()?.length) {
      e.preventDefault();
      const args = argRows()!;
      pickArg(args[Math.min(sel(), args.length - 1)]!);
      return;
    }
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing && e.keyCode !== 229) {
      const items = slashItems();
      const mentions = menuItems();
      if (items.length) {
        const chosen = items[Math.min(sel(), items.length - 1)]!;
        // D-08: the whole command is already typed (`/compact`): Enter sends it, it doesn't complete it again.
        if (draft().text.trim() === `/${chosen.name}` && !commandNeedsTask(chosen)) {
          e.preventDefault();
          send(sendModeForEnter(e, running()));
          return;
        }
        e.preventDefault();
        pickCommand(chosen.name);
        return;
      }
      if (mentions?.length) {
        e.preventDefault();
        pickMention(mentions[Math.min(sel(), mentions.length - 1)]!);
        return;
      }
      const args = argRows();
      if (args?.length) {
        e.preventDefault();
        pickArg(args[Math.min(sel(), args.length - 1)]!);
        return;
      }
      // A touch keyboard's Enter is a newline; the send button sends.
      if (typeof matchMedia === 'function' && matchMedia('(pointer: coarse)').matches) return;
      e.preventDefault();
      send(sendModeForEnter(e, running()));
    }
  };

  const chips = () => draft().chips;
  const pending = () => pendingAttachmentsFor(key());
  const empty = () => !draft().text.trim() && !draft().quotes.length && !pending().length && !draft().chips.length;
  /** Z-60: Send waits for the attachments; a failed one keeps it closed. */
  const held = () => (/^\/\S/.test(draft().text.trim()) ? undefined : props.sendHold?.());
  const canSend = () => props.readOnly || (attachmentsReady(key()) && !props.busy && !held());
  onMount(() => {
    if (props.autofocus && !props.readOnly) queueMicrotask(() => editor?.focus());
  });

  return (
    <div
      class="tl-composer-wrap"
      data-testid="tl-composer-wrap"
      ref={wrap}
      onDragEnter={dropHandlers.onDragEnter}
      onDragOver={dropHandlers.onDragOver}
      onDragLeave={dropHandlers.onDragLeave}
      onDrop={dropHandlers.onDrop}
    >
      <Show when={slashItems().length}>
        <SlashMenu items={slashItems()} selected={Math.min(sel(), slashItems().length - 1)} onPick={pickCommand} onHover={setSel} />
      </Show>
      <Show when={argRows()?.length ? argMenu() : undefined}>
        {(menu) => (
          <ArgMenu
            command={menu().command}
            items={menu().items}
            selected={Math.min(sel(), menu().items.length - 1)}
            onPick={pickArg}
            onHover={setSel}
          />
        )}
      </Show>
      <Show when={menuItems()}>
        <MentionMenu items={menuItems()!} selected={Math.min(sel(), menuItems()!.length - 1)} onPick={pickMention} onHover={setSel} />
      </Show>
      <div
        class="tl-composer tl-composer-dock"
        data-testid="tl-composer"
        data-readonly={props.readOnly ? '' : undefined}
        data-draft={props.draft ? '' : undefined}
        data-drop={dropOver() ? '' : undefined}
        onMouseDown={(e) => {
          // A press on the card's padding lands in the field (the whole card reads as the input).
          if (e.target === e.currentTarget && !props.readOnly) {
            e.preventDefault();
            editor?.focus();
          }
        }}
      >
        <Show when={props.header}>
          <div class="comp-head" data-testid="composer-head">
            {props.header}
          </div>
        </Show>
        <Show when={draft().quotes.length > 0 || pending().length > 0 || chips().length > 0}>
          <div class="tl-composer-chips" data-testid="tl-composer-chips" role="list">
            <For each={draft().quotes}>
              {(quote) => (
                <span class="tl-chip tl-quote-chip" data-testid="tl-quote-chip" title={quote.text}>
                  <Quote size={12} />
                  <span class="tl-chip-label">{quote.text.replace(/\s+/g, ' ').trim().slice(0, 72)}</span>
                  <button type="button" class="tl-chip-x" aria-label="Remove quote" onClick={() => removeComposerQuote(key(), quote.id)}>
                    ✕
                  </button>
                </span>
              )}
            </For>
            <For each={chips()}>{(chip: ComposerChip) => <PastedTextChip chip={chip} onRemove={() => removeComposerChip(key(), chip.id)} />}</For>
            <For each={pending()}>
              {(attachment) => (
                <AttachmentChip
                  attachment={attachment}
                  onRemove={() => removeAttachment(key(), attachment.id)}
                  onRetry={() => retryAttachment(key(), hostId(), attachment.id)}
                  onEdit={attachment.kind === 'image' ? () => openEditor(attachment.id) : undefined}
                />
              )}
            </For>
          </div>
        </Show>
        <input
          ref={photoInput}
          type="file"
          accept="image/*"
          multiple
          hidden
          aria-label="Attach photos"
          onChange={(e) => {
            attachPicked([...(e.currentTarget.files ?? [])]);
            e.currentTarget.value = '';
          }}
        />
        <input
          ref={cameraInput}
          type="file"
          accept="image/*"
          capture="environment"
          hidden
          aria-label="Take a photo"
          onChange={(e) => {
            attachPicked([...(e.currentTarget.files ?? [])]);
            e.currentTarget.value = '';
          }}
        />
        <input
          ref={fileInput}
          type="file"
          multiple
          hidden
          aria-label="Attach a file"
          onChange={(e) => {
            attachPicked([...(e.currentTarget.files ?? [])]);
            e.currentTarget.value = '';
          }}
        />
        <ChipEditor
          class="tl-composer-field"
          value={draft().text}
          disabled={props.readOnly}
          placeholder={placeholder()}
          ariaLabel="Message"
          opts={tokenOpts}
          api={(api) => (editor = api)}
          onHistory={travel}
          onChange={(text, at, kind) => {
            commit(text, at, kind);
            setCaret(at);
            setDismissed(null);
          }}
          onCaret={setCaret}
          onKeyDown={onKeyDown}
          onPasteFiles={props.phone ? undefined : (files) => attachPicked(files)}
          onPasteNative={!props.phone && isTauriDesktop() ? pasteNative : undefined}
          onPasteText={onPasteText}
        />
        <Toolbar
          phone={props.phone}
          lead={
            <button
              type="button"
              class="composer-plus-btn"
              aria-label="Add"
              aria-expanded={plusAnchor() !== null}
              disabled={props.readOnly}
              onClick={(e) => setPlusAnchor((e.currentTarget as HTMLElement).getBoundingClientRect())}
              data-testid="composer-plus"
            >
              <Paperclip size={17} />
            </button>
          }
          end={
            <span class="tb-end-group">
              <Show when={!props.draft}>
                <ContextRing session={props.session} />
              </Show>
              <SendButton
                running={running()}
                disabled={props.readOnly}
                empty={empty()}
                blocked={!canSend()}
                blockedReason={props.busy ? 'Starting the chat…' : held() ? held() : 'Waiting for the attachments — Send unlocks when they are ready'}
                onSend={() => send()}
                onStop={stop}
                onSendNow={sendNow}
                options={!props.draft}
              />
            </span>
          }
        >
          <ToolbarSlot slot="permission">
            <PermissionControl session={props.session} onCommand={runCommand} disabled={props.readOnly} />
          </ToolbarSlot>
          <ToolbarSlot slot="plan">
            <PlanToggle session={props.session} onCommand={runCommand} disabled={props.readOnly} />
          </ToolbarSlot>
          <ToolbarSlot slot="think">
            <ThinkingChip session={props.session} disabled={props.readOnly} />
          </ToolbarSlot>
          <ToolbarSlot slot="model">{props.modelSlot ?? <ModelChip session={props.session} disabled={props.readOnly} />}</ToolbarSlot>
        </Toolbar>
      </div>
      <Show when={plusAnchor()}>
        {(anchor) => (
          <PlusMenu
            anchor={anchor()}
            onDismiss={() => setPlusAnchor(null)}
            onPhoto={() => photoInput?.click()}
            onCamera={isTauriDesktop() ? undefined : () => cameraInput?.click()}
            onFile={() => fileInput?.click()}
            onSideQuestion={props.draft ? undefined : () => openSideQuestion(draft().text.trim() || undefined)}
            onAddTask={() => addTaskFromChat(hostId(), cwd() || undefined)}
          />
        )}
      </Show>
      <Show when={editing()}>
        {(edit) => (
          <ImageEditor
            src={edit().src}
            mime={edit().mime}
            name={edit().name}
            onCancel={() => setEditing(undefined)}
            onDone={(blob) => {
              replaceImage(key(), edit().id, blob);
              setEditing(undefined);
              focusComposer(key());
            }}
          />
        )}
      </Show>
    </div>
  );
}

/** Ranks mention rows: leaf-name matches first, then short paths; ≤ 10 rows. */
function rankMentions(query: string, entries: ReadonlyArray<{ path: string; dir: boolean; leaf?: string }>): MentionItem[] {
  const q = query.toLowerCase();
  const items = entries.map((e) => {
    const clean = e.path.replace(/\/+$/, '');
    const leaf = e.leaf ?? clean.split('/').pop() ?? clean;
    return { value: clean, label: leaf, path: clean, dir: e.dir, leafHit: q ? leaf.toLowerCase().includes(q) : false, depth: clean.split('/').length };
  });
  items.sort((a, b) => Number(b.leafHit) - Number(a.leafHit) || a.depth - b.depth || a.label.localeCompare(b.label));
  return items.slice(0, 10).map(({ value, label, path, dir }) => ({ value, label, path, dir }));
}
