/**
 * Chat composer (UNI-113): a growing field, `/` command suggestions above
 * it, Send ↔ Stop, a "+" menu (Photo/Camera/File/Side question/New
 * session/Resume), a send-mode chevron menu (Steer/Send now/After it
 * ends), the queue strip (messages waiting for pi — phone items editable),
 * and a context ring + model chip.
 *
 * UNI-157: a minimal photo-library attach button + image chip strip
 * (shrink → chip → inline/blob-upload on send, lib/chat/attachments.ts).
 * The "+" menu's Photo/Camera entries reuse the same `pickImages()`.
 *
 * UNI-159: send modes. Busy pi: tapping Send = steer (unchanged). The
 * chevron next to Send (and long-press) offers Steer / Send now / After it
 * ends. The queue strip shows pi's own queue AND the bridge's own "after
 * it ends" queue; every row is editable (edit inline, remove, send now;
 * reorder for bridge items).
 *
 * UNI-202: busy pi, non-empty draft: tapping Send opens a small inline
 * choice popover right at the send button (Steer / Queue pills) instead
 * of sending right away; it remembers which one was picked last for this
 * chat (lib/chat/sendChoice.ts) as the default highlight. The
 * chevron/long-press menu stays for the advanced options (now, explicit
 * followUp). Editing/removing a pi-owned queue row (`source:"tui"`) works
 * too — the bridge aborts-and-resends under the hood, invisible here.
 */
import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { ArrowDown, ArrowUp, ChevronDown, Clock, CornerDownRight, Ellipsis, File as FileIcon, Folder, Paperclip, Pencil, PenLine, Square, Trash2, X, Zap } from 'lucide-solid';
import { ImageEditor } from './ImageEditor';
import type { ArgCompletion, Chat, FileItem } from '../../lib/chat/store';
import { allCommands, applyAt, argQuery, atQuery, isKnownCommand, rankCommands, slashQuery, SOURCE_LABEL, TERMINAL_ONLY, wantsArgCompletions } from '../../lib/chat/commands';
import { lastSendChoice, rememberSendChoice, type SendChoice } from '../../lib/chat/sendChoice';
import {
  addPendingFile,
  addPendingImage,
  attachGenericFile,
  attachLocalFile,
  MAX_GENERIC_FILE_BYTES,
  newPendingFileId,
  newPendingImageId,
  pendingFileById,
  pendingFilesFor,
  removePendingFile,
  resolvePromptImages,
  updatePendingFile,
  type PendingImage,
} from '../../lib/chat/attachments';
import { attachmentToken, imageChipIds, imageToken, insertToken, pathCandidates, removeToken, resolveAttachmentTokens, resolveImageTokens, restoredImageText, tokenize, type Chip } from '../../lib/chat/chips';
import { PlusMenu, SendModeMenu } from './SessionMenus';
import { AnchoredMenu, MenuRow } from '../board/AnchoredMenu';
import { ChipEditor, type ChipEditorApi } from './ChipEditor';
import { desktopShell } from '../../lib/layout';
import { modKey } from '../../lib/shell';
import { isTauriDesktop, isTauriMobile } from '../../lib/platform';
import { isInlineImage, mimeForName, planLocalFiles, type LocalFile } from '../../lib/chat/paste';
import { clipboardFiles, readLocalFiles } from '../../lib/localFiles';
import { choiceKey, composerKey, splitDropped } from '../../lib/desktop';
import { composerHistory, type EditKind } from '../../lib/chat/undo';

/** What the chat layer can ask of the composer (UNI-117 desktop: Ctrl+L, drop files, Esc). */
export type ComposerApi = {
  focus(): void;
  /** Attach files (dropped on the chat / pasted): images inline, others uploaded as chips. */
  attachFiles(files: File[]): void;
  /** UNI-256 (desktop app): files from the OS by path (dropped on the window, copied in the file
   * manager): images read and sent inline, others uploaded from their path as chips. */
  attachLocal(files: LocalFile[]): void;
  /** UNI-290: replace the draft with a message's text (its `[image N]`s back as image chips for
   *  `images`, which may still be loading), focused with the caret at the end. */
  restore(text: string, images?: ReadonlyArray<Promise<File | undefined>>): void;
};

export function ContextRing(props: { percent: number | null | undefined }) {
  const p = () => Math.max(0, Math.min(100, props.percent ?? 0));
  const r = 6;
  const c = 2 * Math.PI * r;
  return (
    <svg class="ctx-ring" width="16" height="16" viewBox="0 0 16 16" data-high={p() >= 80 ? '' : undefined} aria-hidden="true">
      <circle cx="8" cy="8" r={r} class="ctx-ring-bg" />
      <circle cx="8" cy="8" r={r} class="ctx-ring-fg" stroke-dasharray={`${(c * p()) / 100} ${c}`} transform="rotate(-90 8 8)" />
    </svg>
  );
}

/** One row of the queue strip: pi's own steer/follow-up mirror OR the
 * bridge's own "after it ends" queue — both fully actionable (UNI-202):
 * edit inline, remove, send now. Reordering stays bridge-items-only (pi's
 * own steer/followUp relative order isn't the phone's to rearrange). */
function QueueRow(props: {
  item: { id: string; text: string; mode: 'steer' | 'followUp' | 'after'; source: 'phone' | 'tui'; editable: boolean };
  index: number;
  count: number;
  chat: Chat;
  onHaptic: () => void;
}) {
  const [editing, setEditing] = createSignal(false);
  const [more, setMore] = createSignal<DOMRect | null>(null);
  const [draft, setDraft] = createSignal(props.item.text);
  const label = () => (props.item.mode === 'followUp' ? 'After this' : props.item.mode === 'after' ? 'After it ends' : 'Next');
  const reorderable = () => props.item.source === 'phone';
  const save = () => {
    const text = draft().trim();
    if (text && text !== props.item.text) props.chat.queueEdit(props.item.id, text);
    setEditing(false);
  };
  return (
    <div class="queue-row" data-editable={props.item.editable ? '' : undefined}>
      <div class="queue-row-main">
        <Clock size={13} class="shrink-0" />
        <span class="queue-mode">{label()}</span>
        <Show when={!editing()} fallback={
          <textarea
            class="queue-edit-field"
            rows={1}
            autofocus
            value={draft()}
            onInput={(e) => setDraft(e.currentTarget.value)}
            onBlur={save}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault();
                save();
              }
              if (e.key === 'Escape') {
                setDraft(props.item.text);
                setEditing(false);
              }
            }}
          />
        }>
          <span class="queue-text truncate" title={props.item.text}>
            {props.item.text}
          </span>
        </Show>
      </div>
      <Show when={props.item.editable && !editing()}>
        <div class="queue-actions">
          <Show when={reorderable() && props.count > 1}>
            <button
              type="button"
              class="queue-icon-btn"
              aria-label="Move up"
              disabled={props.index === 0}
              onClick={() => {
                props.onHaptic();
                props.chat.queueMove(props.item.id, props.index - 1);
              }}
            >
              <ArrowUp size={14} />
            </button>
          </Show>
          <button type="button" class="queue-icon-btn" aria-label="Edit" onClick={() => setEditing(true)}>
            <Pencil size={14} />
          </button>
          <button
            type="button"
            class="queue-icon-btn"
            aria-label="Remove"
            data-tone="danger"
            onClick={() => {
              props.onHaptic();
              props.chat.queueRemove(props.item.id);
            }}
          >
            <Trash2 size={14} />
          </button>
          <button
            type="button"
            class="queue-icon-btn"
            aria-label="More actions"
            aria-expanded={more() !== null}
            onClick={(e) => setMore((e.currentTarget as HTMLElement).getBoundingClientRect())}
          >
            <Ellipsis size={15} />
          </button>
        </div>
      </Show>
      <Show when={more()}>
        {(anchor) => (
          <AnchoredMenu anchor={anchor()} alignEnd width={248} label="Queued message" onDismiss={() => setMore(null)}>
            <MenuRow icon={<CornerDownRight size={18} />} label="Steer now" onSelect={() => (setMore(null), props.onHaptic(), props.chat.queuePromote(props.item.id, 'steer'))} />
            <MenuRow icon={<Zap size={18} />} label="Send now" hint={<span class="board-menu-note">stops run</span>} onSelect={() => (setMore(null), props.onHaptic(), props.chat.queuePromote(props.item.id, 'now'))} />
            <Show when={reorderable() && props.index < props.count - 1}>
              <MenuRow icon={<ArrowDown size={18} />} label="Move down" onSelect={() => (setMore(null), props.onHaptic(), props.chat.queueMove(props.item.id, props.index + 1))} />
            </Show>
          </AnchoredMenu>
        )}
      </Show>
    </div>
  );
}

/** UNI-202 Steer/Queue choice, reworked UNI-212: no longer a floating
 * popover over the send button (it covered the last transcript bubble) —
 * an in-flow bar that grows the composer stack, so the transcript above is
 * pushed up, never hidden. `defaultChoice` (remembered per chat,
 * lib/chat/sendChoice.ts) just decides which pill looks picked first. */
function SendChoiceBar(props: { defaultChoice: SendChoice; onDismiss: () => void; onPick: (choice: SendChoice) => void; keys?: boolean }) {
  return (
    <div class="send-choice-bar" role="group" aria-label="Send this message">
      <span class="send-choice-label">pi is working —</span>
      <div class="send-choice-row">
        <button type="button" class="send-choice-pill" data-picked={props.defaultChoice === 'steer' ? '' : undefined} onClick={() => props.onPick('steer')}>
          <CornerDownRight size={14} /> Steer
          <Show when={props.keys}>
            <kbd class="kbd send-choice-kbd">S</kbd>
          </Show>
        </button>
        <button type="button" class="send-choice-pill" data-picked={props.defaultChoice === 'after' ? '' : undefined} onClick={() => props.onPick('after')}>
          <Clock size={14} /> Queue
          <Show when={props.keys}>
            <kbd class="kbd send-choice-kbd">Q</kbd>
          </Show>
        </button>
      </div>
      <button type="button" class="send-choice-x" aria-label="Cancel" onClick={props.onDismiss}>
        <X size={14} />
      </button>
    </div>
  );
}

export function Composer(props: {
  chat: Chat;
  onHaptic: (kind?: 'tap' | 'tick') => void;
  onModel: () => void;
  onCommand: (name: string, args?: string) => boolean;
  /** UNI-308: `/name` is neither a session command nor one the app handles. */
  onUnknownCommand?: (name: string) => void;
  draft: string;
  setDraft: (text: string) => void;
  onSideQuestion: (question?: string) => void;
  onNewSession: () => void;
  /** UNI-181: + menu "Board task…". */
  onAddTask?: () => void;
  /** UNI-117: hands the chat layer focus/attach controls. */
  api?: (api: ComposerApi) => void;
}) {
  const s = () => props.chat.state;
  const chatKey = () => `${props.chat.hostId}:${props.chat.session}`;
  let editor: ChipEditorApi | undefined;
  const [caret, setCaret] = createSignal(0);
  const [dismissed, setDismissed] = createSignal<string | null>(null);
  const running = () => s().run.running;

  // "File" (any type, ≤ 1 GB; big ones streamed, UNI-278): uploaded right away via the blob channel,
  // tracked per chat (`pendingFilesFor`, UNI-203) and — UNI-212 — shown as
  // an inline chip at the caret where it was attached: the draft holds a
  // `[file: name]@att:<id>` token (lib/chat/chips.ts), swapped for the
  // host path at send.
  let fileInput: HTMLInputElement | undefined;
  const pendingFiles = () => pendingFilesFor(chatKey());
  const fileState = (id: string) => pendingFileById(chatKey(), id);

  // UNI-270: undo/redo. The field's DOM is rebuilt under the browser (chips, paste, picks), so its
  // own undo stack is useless — the draft history lives here, per chat (lib/chat/undo.ts). Every
  // draft change made here goes through `commit`; one made outside (a quote, restored queue
  // messages, an @-mention from Files) is caught by the effect below as a step of its own.
  const history = () => composerHistory(chatKey());
  const [histRev, setHistRev] = createSignal(0);
  const commit = (text: string, at: number, kind: EditKind = 'other') => {
    history().record(text, at, kind);
    setHistRev((n) => n + 1);
    props.setDraft(text);
  };
  createEffect(() => {
    const text = props.draft;
    const h = history();
    if (h.present?.text === text) return;
    h.record(text, text.length, 'other');
    setHistRev((n) => n + 1);
  });
  const travel = (op: 'undo' | 'redo') => {
    const st = op === 'undo' ? history().undo() : history().redo();
    if (!st) return;
    setHistRev((n) => n + 1);
    setDismissed(null);
    setText(st.text, st.caret, false);
  };

  // Chips: commands from this session's list; `@path`s pi's own finder
  // confirmed exist (picked from the @ menu, or verified below).
  const commandNames = createMemo(() => new Set(s().commands.filter((c) => c.source !== 'builtin').map((c) => c.name)));
  const [verified, setVerified] = createSignal<ReadonlySet<string>>(new Set(), { equals: false });
  const markVerified = (token: string) => {
    const next = new Set(verified());
    next.add(token);
    setVerified(next);
  };
  const tokenOpts = () => ({ commands: commandNames(), verified: (t: string) => verified().has(t), requireTrailingSpace: true });
  const segs = createMemo(() => tokenize(props.draft, tokenOpts()));
  const fileChips = createMemo(() => segs().flatMap((x) => ('chip' in x && x.chip.kind === 'file' && x.chip.attId ? [x.chip] : [])));
  // Nothing sendable: no text once unfinished uploads are left out, no images.
  const empty = () => !resolveAttachmentTokens(props.draft, fileState).text.trim() && images().length === 0;
  const uploading = () => fileChips().some((c) => fileState(c.attId!)?.status === 'uploading');
  // UNI-285: an image chip's label is its position among the draft's image
  // chips (= attach order); the token itself carries no number, so the chip
  // and the text sent to pi number them the same way.
  const imageLabel = (chip: Chip): string | undefined => {
    if (chip.kind !== 'image' || !chip.imgId) return undefined;
    const n = imageChipIds(props.draft).indexOf(chip.imgId);
    return n < 0 ? undefined : `image ${n + 1}`;
  };

  // A typed `@path` (not picked from the menu) chips once the bridge's
  // finder confirms it exists — one lookup per finished token.
  const asked = new Set<string>();
  createEffect(
    on(
      () => props.draft,
      (text) => {
        const t = setTimeout(() => {
          for (const token of pathCandidates(text)) {
            if (asked.has(token) || verified().has(token)) continue;
            asked.add(token);
            const query = token.slice(1).replace(/"/g, '');
            const dir = query.endsWith('/');
            void props.chat.files(query).then((items) => {
              const want = query.replace(/\/$/, '');
              if (items.some((i) => i.value.replace(/^@"?|"$/g, '').replace(/\/$/, '') === want && i.dir === dir)) markVerified(token);
            });
          }
        }, 350);
        onCleanup(() => clearTimeout(t));
      },
    ),
  );

  const slash = createMemo(() => slashQuery(props.draft, caret()));
  const suggestions = createMemo(() => {
    const q = slash();
    if (!q || dismissed() === props.draft) return [];
    return rankCommands(allCommands(s().commands), q.query, 400);
  });

  // `@` → files from pi's working directory (pi's own finder), debounced;
  // a reply for an older query is dropped.
  const at = createMemo(() => atQuery(props.draft, caret()));
  const [files, setFiles] = createSignal<FileItem[]>([]);
  const [filesFor, setFilesFor] = createSignal<string | null>(null);
  let filesSeq = 0;
  createEffect(
    on(
      () => (at() && dismissed() !== props.draft ? at()!.query : null),
      (query) => {
        const seq = ++filesSeq;
        if (query === null) {
          setFiles([]);
          setFilesFor(null);
          return;
        }
        const t = setTimeout(() => {
          void props.chat.files(query).then((items) => {
            if (seq !== filesSeq) return;
            setFiles(items);
            setFilesFor(query);
          });
        }, 140);
        onCleanup(() => clearTimeout(t));
      },
    ),
  );
  const fileMenu = () => (at() && filesFor() !== null ? files() : []);

  // UNI-276: `/command <arg>` → the command's own argument suggestions (as in the terminal).
  const argQ = createMemo(() => argQuery(props.draft, caret()));
  const [args, setArgs] = createSignal<{ key: string; items: ArgCompletion[] }>({ key: '', items: [] });
  createEffect(
    on(
      () => {
        const q = argQ();
        return q && !at() && dismissed() !== props.draft && s().commands.some((c) => c.name === q.command && wantsArgCompletions(c.source)) ? `${q.command}\u0000${q.prefix}` : null;
      },
      (key) => {
        if (key === null) return setArgs({ key: '', items: [] });
        const [command, prefix] = key.split('\u0000') as [string, string];
        const t = setTimeout(() => {
          void props.chat.argCompletions?.(command, prefix).then((items) => {
            if (argQ() && `${argQ()!.command}\u0000${argQ()!.prefix}` === key) setArgs({ key, items: items.filter((x) => x.value.trim() || x.label) });
          });
        }, 120);
        onCleanup(() => clearTimeout(t));
      },
    ),
  );
  const argMenu = () => {
    const q = argQ();
    // An item equal to what's typed adds nothing; an empty value ("<prompt>") is only a hint.
    return q && args().key === `${q.command}\u0000${q.prefix}` ? args().items.filter((x) => x.value.trim() !== q.prefix.trim() && x.value !== '') : [];
  };
  const pickArg = (item: ArgCompletion) => {
    const q = argQ();
    if (!q) return;
    const next = props.draft.slice(0, q.start) + item.value + props.draft.slice(caret());
    props.onHaptic('tick');
    setText(next, q.start + item.value.length);
  };

  // Pending image attachments (UNI-157): shrunk on pick, chips shown above
  // the field, resolved to inline base64 / blob-upload path at send time.
  // UNI-285: each one also inserts an inline `[image]@img:<id>` chip at the
  // caret, numbered by its position here (the token itself carries no
  // number, so renumbering after a remove needs no text rewrite).
  const [images, setImages] = createSignal<PendingImage[]>([]);
  const [attaching, setAttaching] = createSignal(0);
  let photoInput: HTMLInputElement | undefined;
  let cameraInput: HTMLInputElement | undefined;
  const pickImages = async (fileList: FileList | null) => {
    // `files` is a live FileList tied to the <input>: the caller clears
    // `input.value` right after calling this, which clears `files` too, so
    // its length is read once up front (not again in `finally`).
    const picked = [...(fileList ?? [])];
    if (!picked.length) return;
    // The ids are minted first so the chips go in at the caret immediately,
    // in the same order the images are appended — chip N is image N.
    const ids = picked.map(() => newPendingImageId());
    let text = props.draft;
    let pos = editor?.getCaret() ?? caret();
    for (const id of ids) {
      const next = insertToken(text, pos, imageToken(id));
      text = next.text;
      pos = next.caret;
    }
    commit(text, pos);
    queueMicrotask(() => editor?.setCaret(pos));
    setAttaching((n) => n + picked.length);
    // A file we cannot read (a 0-byte paste, a decode failure) must not leave
    // its chip behind pointing at nothing: settle each image on its own and
    // take the chips of the failed ones back out of the draft.
    const settled = await Promise.all(picked.map((f, i) => addPendingImage(f, ids[i]!).then((image) => image, (error: unknown) => ({ error }))));
    setAttaching((n) => Math.max(0, n - picked.length));
    const added: PendingImage[] = [];
    const failed: string[] = [];
    settled.forEach((result, i) => {
      if ('error' in result) {
        console.warn(`[attach] could not attach ${picked[i]!.name}: ${String(result.error)}`);
        failed.push(ids[i]!);
      } else added.push(result);
    });
    if (added.length) setImages((prev) => [...prev, ...added]);
    if (failed.length) dropTokens(failed);
  };
  // UNI-227: tap an attached image to draw on it, add arrows or crop it before sending.
  const [editing, setEditing] = createSignal<PendingImage | undefined>();
  const applyEdit = (id: string, blob: Blob) => {
    setImages((prev) =>
      prev.map((p) => {
        if (p.id !== id) return p;
        URL.revokeObjectURL(p.previewUrl);
        return { ...p, blob, mime: blob.type || p.mime, previewUrl: URL.createObjectURL(blob) };
      }),
    );
    setEditing(undefined);
  };
  /** Takes these images' `[image]@img:<id>` chips out of the draft (every
   *  occurrence), recorded as one step in the undo history. */
  const dropTokens = (ids: readonly string[]) => {
    let text = props.draft;
    let changed = false;
    for (const id of ids) {
      const token = imageToken(id);
      let at = text.indexOf(token);
      while (at !== -1) {
        const next = removeToken(text, at, token);
        text = next.text;
        changed = true;
        at = text.indexOf(token, next.caret);
      }
    }
    if (changed) commit(text, text.length);
  };
  const removeImage = (id: string) => {
    // The chip goes with the image (UNI-285), or a later edit would show a
    // strip thumbnail with nothing referring to it.
    dropTokens([id]);
    setImages((prev) => {
      const target = prev.find((p) => p.id === id);
      if (target) URL.revokeObjectURL(target.previewUrl);
      return prev.filter((p) => p.id !== id);
    });
  };

  const pickFiles = async (fileList: FileList | null) => {
    const picked = [...(fileList ?? [])];
    if (!picked.length) return;
    // Tokens go in at the caret first (the chips show their upload state
    // right away), then the uploads run.
    let text = props.draft;
    let pos = editor?.getCaret() ?? caret();
    const ids = picked.map((file) => {
      const id = newPendingFileId();
      const next = insertToken(text, pos, attachmentToken(file.name, id));
      text = next.text;
      pos = next.caret;
      return id;
    });
    commit(text, pos);
    queueMicrotask(() => editor?.setCaret(pos));
    await Promise.all(picked.map((file, i) => attachGenericFile(chatKey(), props.chat.hostId, file, ids[i])));
  };
  // The draft is the truth: an upload whose token left the text (✕, Backspace, deleted with a
  // selection, the draft cleared) is dropped too — unless Undo/Redo can still bring the token back
  // (UNI-270), then it stays so the chip comes back working. Same rule for a pending image
  // (UNI-285): its `[image]@img:<id>` chip is what references it.
  createEffect(() => {
    histRev();
    const ids = new Set(fileChips().map((c) => c.attId));
    for (const text of history().reachable()) for (const c of tokenize(text)) if ('chip' in c && c.chip.kind === 'file' && c.chip.attId) ids.add(c.chip.attId);
    for (const f of pendingFiles()) if (!ids.has(f.id)) removePendingFile(chatKey(), f.id);
  });
  createEffect(() => {
    histRev();
    const ids = new Set<string>();
    for (const seg of segs()) if ('chip' in seg && seg.chip.kind === 'image' && seg.chip.imgId) ids.add(seg.chip.imgId);
    for (const text of history().reachable()) for (const c of tokenize(text)) if ('chip' in c && c.chip.kind === 'image' && c.chip.imgId) ids.add(c.chip.imgId);
    const keep = images().filter((p) => ids.has(p.id));
    if (keep.length !== images().length) setImages(keep);
  });

  const send = (mode: 'auto' | 'steer' | 'followUp' | 'now' | 'after' = 'auto') => {
    const draft = props.draft;
    const pending = images();
    // UNI-212: attachment tokens become the UNI-203 wire mention in place
    // (`[file: name]@<host path>`); one still uploading / failed stays in the
    // draft (its chip keeps its state) and isn't sent.
    const resolved = resolveAttachmentTokens(draft, fileState);
    // UNI-285: each image chip becomes `[image N]`, N matching the order the
    // images are sent in, so pi can be told which is which.
    const withImages = resolveImageTokens(resolved.text, pending);
    const text = withImages.text.trim();
    if (!text && !pending.length) return;
    const cmd = /^\/(\S+)(?:\s+([\s\S]*))?$/.exec(text);
    // /btw and /unipi:btw open the side-question sheet instead of sending to the main chat.
    if (cmd && (cmd[1] === 'btw' || cmd[1] === 'unipi:btw')) {
      props.onSideQuestion(cmd[2]?.trim() || undefined);
      sent(text, '');
      return;
    }
    if (cmd && props.onCommand(cmd[1]!, cmd[2]?.trim())) {
      sent(text, '');
      return;
    }
    // UNI-308: a `/name` the session doesn't list and the app doesn't handle
    // must never silently become a plain prompt — say so instead.
    if (cmd && !isKnownCommand(cmd[1]!, s().commands)) {
      props.onUnknownCommand?.(cmd[1]!);
      return;
    }
    props.onHaptic('tap');
    setImages([]);
    const outgoing = text;
    for (const id of resolved.sentIds) removePendingFile(chatKey(), id);
    // Send the images in the order their chips appear in the text, so
    // "[image N]" is always the Nth image pi receives.
    const ordered = withImages.ids.flatMap((id) => pending.filter((p) => p.id === id));
    if (!ordered.length) {
      props.chat.send(outgoing, mode);
    } else {
      void resolvePromptImages(props.chat.hostId, ordered)
        .then((imgs) => props.chat.send(outgoing, mode, imgs))
        .finally(() => ordered.forEach((p) => URL.revokeObjectURL(p.previewUrl)));
    }
    sent(outgoing, resolved.kept.join(' ') + (resolved.kept.length ? ' ' : ''));
  };
  /** After a send the history starts over: one Ctrl/⌘+Z brings back what was sent (attachments as
   * their uploaded host paths, so sending it again works), nothing before it. */
  const sent = (outgoing: string, rest: string) => {
    history().reset(outgoing);
    commit(rest, rest.length);
  };

  // Long-press Send (while pi works) opens the send-mode menu (now / explicit
  // followUp — the advanced options). A plain tap while busy, with text to
  // send, opens the in-flow Steer/Queue choice bar (UNI-202/212).
  const [modeMenu, setModeMenu] = createSignal<DOMRect | null>(null);
  const [plusMenu, setPlusMenu] = createSignal<DOMRect | null>(null);
  const [choiceOpen, setChoiceOpen] = createSignal(false);
  // The highlighted pill while the choice is open (keyboard: ←/→ move it, Enter picks it).
  const [choiceHi, setChoiceHi] = createSignal<SendChoice>('steer');
  createEffect(on(choiceOpen, (o) => o && setChoiceHi(lastSendChoice(props.chat.hostId, props.chat.session))));
  // The run ended while the choice was open: nothing to steer — the bar goes.
  createEffect(on(running, (r) => !r && setChoiceOpen(false)));
  createEffect(on(() => props.draft, (d) => !d.trim() && setChoiceOpen(false)));
  let press: ReturnType<typeof setTimeout> | undefined;
  let longFired = false;
  let sendBtn: HTMLButtonElement | undefined;
  const down = () => {
    longFired = false;
    if (!running() || empty()) return;
    press = setTimeout(() => {
      longFired = true;
      props.onHaptic('tick');
      setModeMenu(sendBtn?.getBoundingClientRect() ?? null);
    }, 450);
  };
  const up = () => {
    if (press) clearTimeout(press);
    press = undefined;
  };
  onCleanup(up);
  const choose = (choice: SendChoice) => {
    rememberSendChoice(props.chat.hostId, props.chat.session, choice);
    setChoiceOpen(false);
    send(choice);
  };

  const setText = (text: string, at: number, record = true) => {
    if (record) commit(text, at);
    else props.setDraft(text);
    queueMicrotask(() => {
      editor?.setCaret(at);
      setCaret(at);
    });
  };
  const pick = (name: string) => {
    const q = slash();
    if (!q) return;
    const next = `${props.draft.slice(0, q.start)}/${name} ${props.draft.slice(caret())}`.replace(/\s+$/, ' ');
    props.onHaptic('tick');
    setText(next, q.start + name.length + 2);
  };

  const pickFile = (f: FileItem) => {
    const q = at();
    if (!q) return;
    const next = applyAt(props.draft, caret(), q, f.value, f.dir);
    // Picked from pi's own finder: it exists — chip it.
    if (!f.dir) markVerified(f.value);
    props.onHaptic('tick');
    setText(next.text, next.caret);
  };

  // Combined queue strip: pi's own (source:"tui") + the bridge's own
  // ("after it ends", source:"phone", editable). Older bridges send no
  // `queue{}` items for a phone followUp — the pending bubble covers that.
  const queueItems = createMemo(() => {
    if (s().queue.length) return s().queue;
    return s()
      .pending.filter((p): p is typeof p & { mode: 'steer' | 'followUp' } => p.mode === 'steer' || p.mode === 'followUp')
      .map((p) => ({ id: p.ref, text: p.text, mode: p.mode, source: 'phone' as const, editable: false }));
  });

  // UNI-117: images go to the image strip (inline), anything else uploads as an attachment chip.
  const attachFiles = (files: File[]) => {
    const { images: imgs, others } = splitDropped(files);
    if (imgs.length) void pickImages(imgs as unknown as FileList);
    if (others.length) void pickFiles(others as unknown as FileList);
  };
  // UNI-256: files from the OS by path (desktop app: a drop, files copied in the file manager, a
  // screenshot on the clipboard — the core saved it to a temp PNG).
  const attachLocal = (files: LocalFile[]) => {
    const plan = planLocalFiles(files, MAX_GENERIC_FILE_BYTES);
    const imgs = plan.read.filter((f) => isInlineImage(f.name));
    const others = plan.read.filter((f) => !isInlineImage(f.name));
    if (imgs.length) {
      setAttaching((n) => n + imgs.length);
      void readLocalFiles(imgs)
        .then((read) => pickImages(read as unknown as FileList))
        .catch((error) => console.warn('[attach] reading images failed', error))
        .finally(() => setAttaching((n) => Math.max(0, n - imgs.length)));
    }
    const chips = [...others.map((file) => ({ file, reason: undefined as string | undefined })), ...plan.refused];
    if (!chips.length) return;
    let text = props.draft;
    let pos = editor?.getCaret() ?? caret();
    const ids = chips.map(({ file }) => {
      const id = newPendingFileId();
      const next = insertToken(text, pos, attachmentToken(file.name, id));
      text = next.text;
      pos = next.caret;
      return id;
    });
    commit(text, pos);
    queueMicrotask(() => editor?.setCaret(pos));
    chips.forEach(({ file, reason }, i) => {
      const mime = mimeForName(file.name);
      if (reason) {
        addPendingFile(chatKey(), { id: ids[i]!, name: file.name, mime, size: file.size, status: 'error' });
        updatePendingFile(chatKey(), ids[i]!, { error: reason });
      } else void attachLocalFile(chatKey(), props.chat.hostId, { path: file.path, name: file.name, size: file.size, mime }, ids[i]);
    });
  };
  /** Ctrl+V with nothing the page could see (WebKitGTK): the clipboard's files / screenshot. */
  const pasteNative = async (): Promise<boolean> => {
    const files = await clipboardFiles();
    if (!files.length) return false;
    attachLocal(files);
    return true;
  };
  /** UNI-290 "Start from here": the message comes back as the whole draft, images re-attached. */
  const restore = (text: string, pending: ReadonlyArray<Promise<File | undefined>> = []) => {
    const ids = pending.map(() => newPendingImageId());
    const next = restoredImageText(text, ids);
    commit(next, next.length);
    setDismissed(null);
    queueMicrotask(() => {
      editor?.setCaret(next.length);
      setCaret(next.length);
    });
    if (!pending.length) return;
    setAttaching((n) => n + pending.length);
    void Promise.all(
      pending.map((p, i) =>
        p
          .then((file) => (file ? addPendingImage(file, ids[i]!) : undefined))
          .catch((error: unknown) => {
            console.warn(`[restore] could not bring an image back: ${String(error)}`);
            return undefined;
          }),
      ),
    ).then((settled) => {
      setAttaching((n) => Math.max(0, n - pending.length));
      const added = settled.filter((x): x is PendingImage => !!x);
      if (added.length) setImages((prev) => [...prev, ...added]);
      const failed = ids.filter((_, i) => !settled[i]);
      if (failed.length) dropTokens(failed);
    });
  };
  props.api?.({ focus: () => editor?.focus(), attachFiles, attachLocal, restore });

  /** Desktop shell keys (lib/desktop.ts composerKey): Enter sends, Ctrl/⌘+Enter = Steer/Queue choice
   * while pi works, Esc stops a run; S/Q/←/→/Enter/Esc drive the open choice bar. */
  const onDesktopKey = (e: KeyboardEvent): boolean => {
    if (choiceOpen()) {
      const c = choiceKey(e, choiceHi());
      if (!c) return false;
      e.preventDefault();
      e.stopPropagation();
      if (c.close) setChoiceOpen(false);
      if (c.highlight) setChoiceHi(c.highlight);
      if (c.pick) choose(c.pick);
      return true;
    }
    if (suggestions().length || fileMenu().length) return false;
    const action = composerKey(e, { running: !!running(), empty: empty() });
    if (!action) return false;
    e.preventDefault();
    e.stopPropagation();
    if (action === 'stop') props.chat.abort();
    else if (action === 'choose') setChoiceOpen(true);
    else if (action === 'send-default') choose(lastSendChoice(props.chat.hostId, props.chat.session));
    else send();
    return true;
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Escape' && (suggestions().length || fileMenu().length || argMenu().length)) {
      e.preventDefault();
      setDismissed(props.draft);
      // Desktop: Esc closes the menu only, not the chat too.
      if (desktopShell()) e.stopPropagation();
      return;
    }
    if (desktopShell() && onDesktopKey(e)) return;
    // Hardware keyboard: Enter sends, Shift+Enter is a new line.
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing && e.keyCode !== 229) {
      const touch = typeof matchMedia === 'function' && matchMedia('(pointer: coarse)').matches;
      if (touch) return;
      e.preventDefault();
      if (suggestions().length) pick(suggestions()[0]!.name);
      else if (fileMenu().length) pickFile(fileMenu()[0]!);
      else if (running() && !empty()) setChoiceOpen(true);
      else send();
    }
  };

  return (
    <div class="composer-seat">
      <Show when={fileMenu().length > 0 && !suggestions().length}>
        <div class="slash-menu" role="listbox" aria-label="Files">
          <For each={fileMenu()}>
            {(f) => (
              <button
                type="button"
                role="option"
                class="slash-row"
                onPointerDown={(e) => e.preventDefault()}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => pickFile(f)}
              >
                {f.dir ? <Folder size={15} class="file-icon dir" /> : <FileIcon size={15} class="file-icon" />}
                <span class="slash-name">{f.label}</span>
                <span class="slash-desc file-path">{f.path}</span>
              </button>
            )}
          </For>
        </div>
      </Show>
      <Show when={argMenu().length > 0 && !suggestions().length && !fileMenu().length}>
        <div class="slash-menu" role="listbox" aria-label="Arguments" data-testid="arg-menu">
          <For each={argMenu()}>
            {(a) => (
              <button
                type="button"
                role="option"
                class="slash-row"
                onPointerDown={(e) => e.preventDefault()}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => pickArg(a)}
              >
                <span class="slash-name">{a.label}</span>
                <span class="slash-desc">{a.description ?? ''}</span>
              </button>
            )}
          </For>
        </div>
      </Show>
      <Show when={suggestions().length > 0}>
        <div class="slash-menu" role="listbox" aria-label="Commands">
          <For each={suggestions()}>
            {(c) => (
              <button
                type="button"
                role="option"
                class="slash-row"
                onPointerDown={(e) => e.preventDefault()}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => pick(c.name)}
              >
                <span class="slash-name">/{c.name}</span>
                <span class="slash-desc">{c.description ?? ''}</span>
                <span class="slash-src" data-src={TERMINAL_ONLY.has(c.name) && c.source === 'builtin' ? 'terminal' : c.source}>
                  {TERMINAL_ONLY.has(c.name) && c.source === 'builtin' ? 'terminal' : SOURCE_LABEL[c.source]}
                </span>
              </button>
            )}
          </For>
        </div>
      </Show>
      <Show when={images().length > 0 || attaching() > 0}>
        <div class="composer-image-strip" aria-label="Attached images">
          <For each={images()}>
            {(image) => (
              <div class="composer-image-chip">
                <button type="button" class="composer-image-edit" aria-label={`Edit ${image.name}`} title="Draw, add arrows or crop" onClick={() => (props.onHaptic('tick'), setEditing(image))}>
                  <img src={image.previewUrl} alt="" />
                  <span class="composer-image-pen" aria-hidden="true">
                    <PenLine size={11} />
                  </span>
                </button>
                <button type="button" class="composer-image-remove" aria-label={`Remove ${image.name}`} onClick={() => removeImage(image.id)}>
                  <X size={12} />
                </button>
              </div>
            )}
          </For>
          <Show when={attaching() > 0}>
            <div class="composer-image-chip composer-image-loading" aria-label="Preparing image" />
          </Show>
        </div>
      </Show>
      <Show when={editing()} keyed>
        {(im) => <ImageEditor src={im.previewUrl} mime={im.mime} name={im.name} onCancel={() => setEditing(undefined)} onDone={(blob) => applyEdit(im.id, blob)} />}
      </Show>
      <Show when={queueItems().length > 0}>
        <div class="queue-strip" aria-label="Waiting for pi">
          <For each={queueItems()}>{(q, i) => <QueueRow item={q} index={i()} count={queueItems().length} chat={props.chat} onHaptic={() => props.onHaptic('tick')} />}</For>
        </div>
      </Show>
      <Show when={choiceOpen()}>
        <SendChoiceBar defaultChoice={desktopShell() ? choiceHi() : lastSendChoice(props.chat.hostId, props.chat.session)} keys={desktopShell()} onDismiss={() => setChoiceOpen(false)} onPick={choose} />
      </Show>
      <div class="chat-composer">
        <ChipEditor
          class="chat-field"
          value={props.draft}
          placeholder={running() ? (desktopShell() ? `Steer pi… (${modKey()}+Enter: steer or queue)` : 'Steer pi… (hold send for more options)') : 'Message pi, / commands, @ files'}
          ariaLabel="Message"
          opts={tokenOpts}
          chipState={(chip) => {
            if (chip.kind !== 'file' || !chip.attId) return undefined;
            const f = fileState(chip.attId);
            return f ? { status: f.status, error: f.error } : { status: 'error', error: 'Attachment lost — remove and attach again' };
          }}
          chipLabel={imageLabel}
          api={(api) => (editor = api)}
          onHistory={travel}
          onChange={(text, at, kind) => {
            commit(text, at, kind);
            setCaret(at);
            setDismissed(null);
          }}
          onCaret={setCaret}
          onKeyDown={onKeyDown}
          onPasteFiles={isTauriMobile() ? undefined : attachFiles}
          onPasteNative={isTauriDesktop() ? pasteNative : undefined}
        />
        <Show when={uploading()}>
          <p class="composer-upload-note">Uploading… a file sends once it's done.</p>
        </Show>
        <div class="chat-composer-row">
          <input
            ref={photoInput}
            type="file"
            accept="image/*"
            multiple
            hidden
            aria-label="Attach photos"
            onChange={(e) => {
              void pickImages(e.currentTarget.files);
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
              void pickImages(e.currentTarget.files);
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
              void pickFiles(e.currentTarget.files);
              e.currentTarget.value = '';
            }}
          />
          <button
            type="button"
            class="composer-plus-btn"
            aria-label="Add"
            aria-expanded={plusMenu() !== null}
            onClick={(e) => {
              props.onHaptic('tick');
              setPlusMenu((e.currentTarget as HTMLElement).getBoundingClientRect());
            }}
          >
            <Paperclip size={16} />
          </button>
          <button type="button" class="model-chip" onClick={() => props.onModel()} aria-label="Model and thinking">
            <ContextRing percent={s().run.context?.percent} />
            <span class="tabular-nums">{s().run.context?.percent != null ? `${Math.round(s().run.context!.percent!)}%` : '–'}</span>
            <span class="model-chip-name truncate">{s().fusionStatus ? `Fusion · ${s().fusionStatus!.leadName || '…'}` : (s().run.model?.name ?? s().run.model?.id ?? 'model')}</span>
            <Show when={s().run.model?.reasoning && s().run.thinking}>
              <span class="model-chip-think">{s().run.thinking}</span>
            </Show>
          </button>
          <span class="flex-1" />
          <Show
            when={running() && empty()}
            fallback={
              <div class="send-group">
                <button
                  ref={sendBtn}
                  type="button"
                  class="send-btn"
                  aria-label={running() ? 'Steer (hold: more options)' : 'Send'}
                  aria-expanded={running() ? choiceOpen() : undefined}
                  disabled={empty() || s().conn !== 'live'}
                  onPointerDown={down}
                  onPointerUp={up}
                  onPointerLeave={up}
                  onPointerCancel={up}
                  onContextMenu={(e) => e.preventDefault()}
                  onClick={() => {
                    if (longFired) {
                      longFired = false;
                      return;
                    }
                    // UNI-202: busy + something to send — one more tap: the
                    // Steer/Queue choice, instead of silently steering.
                    if (running() && !empty()) {
                      setChoiceOpen((v) => !v);
                      return;
                    }
                    send();
                  }}
                >
                  <ArrowUp size={20} strokeWidth={2.4} />
                </button>
                <button
                  type="button"
                  class="send-chevron-btn"
                  aria-label="Send options"
                  disabled={empty() || s().conn !== 'live'}
                  onClick={(e) => {
                    props.onHaptic('tick');
                    setModeMenu((e.currentTarget as HTMLElement).getBoundingClientRect());
                  }}
                >
                  <ChevronDown size={14} />
                </button>
              </div>
            }
          >
            <button
              type="button"
              class="send-btn stop"
              aria-label="Stop"
              disabled={s().conn !== 'live'}
              onClick={() => {
                props.onHaptic('tick');
                props.chat.abort();
              }}
            >
              <Square size={14} fill="currentColor" />
            </button>
          </Show>
        </div>
      </div>
      <Show when={plusMenu()}>
        {(anchor) => (
          <PlusMenu
            anchor={anchor()}
            onDismiss={() => setPlusMenu(null)}
            onPhoto={() => photoInput?.click()}
            onCamera={() => cameraInput?.click()}
            onFile={() => fileInput?.click()}
            onSideQuestion={() => props.onSideQuestion(resolveAttachmentTokens(props.draft, () => undefined).text.trim() || undefined)}
            onNewSession={props.onNewSession}
            onAddTask={props.onAddTask}
          />
        )}
      </Show>
      <Show when={modeMenu()}>
        {(anchor) => <SendModeMenu anchor={anchor()} onDismiss={() => setModeMenu(null)} running={running()} onPick={(mode) => (setChoiceOpen(false), send(mode))} />}
      </Show>
    </div>
  );
}
