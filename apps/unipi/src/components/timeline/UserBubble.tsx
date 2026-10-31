/**
 * User bubble (WP-A6, moved+adapted from Transcript.tsx `UserBubble`; WP-A7
 * adds the fold, Edit and the row menu):
 *
 * - inline chips (`[file: name]@path`, `/commands`, skills, `[image N]`) via
 *   the same tokenizer, verified `@path` mentions (`paths_stat` through the
 *   `RefChat` adapter), task refs, image chips, pending/waiting outbox notes;
 * - **120 px fold** (Z-29, ZCode `v4-user-message-display`): body capped at
 *   120 px with a bottom fade; a centred chevron expands/collapses with a
 *   300 ms height transition (no motion when `prefers-reduced-motion`). The
 *   image/attachment chips sit *outside* the folded body, and the fold resets
 *   when the virtualizer reuses the row for another message;
 * - **Edit** (Z-28): a pencil on the user bubble (hover-revealed on the
 *   desktop, always visible on the phone) tree-jumps into the composer;
 * - **row menu** (Z-17/Z-27): right-click / long-press gives Copy, Quote,
 *   Fork and Edit for this message;
 * - **swipe right = Quote** on the phone (Z-30 keeps it).
 */
import { createEffect, createMemo, createResource, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { ChevronDown, ChevronUp, FileText, FolderOpen, Image as ImageIcon, Pencil, Sparkles, SquareSlash } from 'lucide-solid';
import type { RefChat } from '../../lib/chat/pathStat';
import { lookupPaths } from '../../lib/chat/pathStat';
import { fileRefLabel } from '../../lib/chat/fileRefs';
import { attachFileRefGestures } from '../../lib/chat/fileRefGestures';
import { openFileRef } from '../../lib/chat/fileRefOpen';
import { openFilePath, resolvePathToken } from '../../lib/media/pathTokens';
import { tokenize, type Chip, type Seg } from '../../lib/composer/chips';
import type { ImageRef } from '../../lib/chat/model';
import { knownPrefixes, splitTaskRefs } from '../../lib/board/portal';
import { openTaskFromChat } from '../../lib/session/open';
import { attachMessageMenu, attachQuoteSwipe, type MessageTarget } from '../../lib/timeline/actions';
import { FileRefText } from '../chat/FileRefText';
import { ImageChipRow, openImagePreviewer } from '../chat/ImageChip';

/** Longest collapsed body (Z-29; ZCode folds when `scrollHeight > 121`). */
export const FOLD_PX = 120;
/** Motion 120–200 ms ease-out (ui-bar.md Global). */
const FOLD_MS = 180;

/** A sent message's text as plain runs + inline chips (UNI-212): the same
 * tokenizer as the composer — attachments (`[file: name]@path`), a
 * leading `/skill:x` or `/command` from this session's list, pi's
 * expanded `<skill …>` block, and (UNI-285) an `[image N]` back to that
 * image. Whitespace between two file chips collapses to one space. */
export function bubbleSegments(text: string, commands: ReadonlySet<string>, verified?: (token: string) => boolean): Seg[] {
  const segs = tokenize(text, { commands, skillBlocks: true, verified, imageLabels: true });
  return segs.map((seg, i) => {
    if ('chip' in seg || seg.text.trim()) return seg;
    const prev = segs[i - 1];
    const next = segs[i + 1];
    const between = prev && 'chip' in prev && prev.chip.kind === 'file' && next && 'chip' in next && next.chip.kind === 'file';
    return between ? { text: ' ' } : seg;
  });
}

/** `@path` / `@"a path"` tokens in a sent message (UNI-204): the ones the PC confirms become path chips. */
export function mentionTokens(text: string): string[] {
  const out = new Set<string>();
  for (const m of text.matchAll(/(^|[\s(\[{])(@"[^"\n]+"|@[^\s"]+)/g)) {
    const token = m[2]!;
    if (token.length > 1 && !token.startsWith('@att:')) out.add(token);
  }
  return [...out];
}
const mentionPath = (token: string) => token.replace(/^@"?|"$/g, '');

/** Task refs (UNI-181) in a plain run, tappable. */
function TaskRefRun(props: { chat: RefChat; text: string; prefixes?: Set<string> }) {
  const parts = () => (props.prefixes ? splitTaskRefs(props.text, props.prefixes) : [props.text]);
  return (
    <For each={parts()}>
      {(value) =>
        typeof value === 'string' ? (
          value
        ) : (
          <span
            class="task-ref"
            role="link"
            data-task={value.ref}
            onClick={(event) => {
              event.stopPropagation();
              openTaskFromChat(props.chat.hostId, value.ref, props.chat.cwd?.());
            }}
          >
            {value.ref}
          </span>
        )
      }
    </For>
  );
}

/** Plain text with tappable task refs and verified file refs. */
function BubbleText(props: { chat: RefChat; text: string }) {
  const [prefixes] = createResource(() => props.chat.hostId, knownPrefixes);
  return <FileRefText chat={props.chat} text={props.text} mode="prose" plain={(text) => <TaskRefRun chat={props.chat} text={text} prefixes={prefixes()} />} />;
}

function BubbleChip(props: { chat: RefChat; chip: Chip; resolved?: { resolved: string; kind: 'file' | 'dir' }; images?: readonly ImageRef[] }) {
  const [failed, setFailed] = createSignal(false);
  const c = props.chip;
  const label = c.kind === 'file' ? c.name : c.kind === 'image' ? `image ${c.n}` : c.kind === 'path' ? fileRefLabel(c.path.replace(/\/+$/, '')).label + (props.resolved?.kind === 'dir' ? '/' : '') : c.kind === 'skill' ? c.name : `/${c.name}`;
  const path = c.kind === 'file' ? c.path : c.kind === 'path' ? c.path : undefined;
  const open = async (event: MouseEvent) => {
    event.stopPropagation();
    if (c.kind === 'image') {
      const index = (c.n ?? 1) - 1;
      if (index >= 0 && index < (props.images?.length ?? 0)) void openImagePreviewer(props.chat, props.images!, index);
      return;
    }
    if (!path) return;
    setFailed(false);
    try {
      if (props.resolved) await openFileRef(props.chat, props.resolved);
      else await openFilePath(props.chat, resolvePathToken(path, props.chat.cwd?.()));
    } catch {
      setFailed(true);
    }
  };
  const clickable = () => !!path || (c.kind === 'image' && !!props.images?.length);
  return (
    <span
      class="ichip"
      data-kind={c.kind}
      data-status={failed() ? 'error' : undefined}
      data-file-ref={props.resolved?.resolved}
      data-file-kind={props.resolved?.kind}
      role={clickable() ? 'link' : undefined}
      title={props.resolved?.resolved ?? path ?? c.raw.slice(0, 80)}
      aria-label={`${c.kind === 'command' ? 'command' : c.kind === 'skill' ? 'skill' : c.kind === 'image' ? 'open image' : 'open file'} ${label}`}
      onClick={clickable() ? (e) => void open(e) : undefined}
    >
      {c.kind === 'skill' ? <Sparkles size={13} /> : c.kind === 'command' ? <SquareSlash size={13} /> : c.kind === 'image' ? <ImageIcon size={13} /> : props.resolved?.kind === 'dir' ? <FolderOpen size={13} /> : <FileText size={13} />}
      <span class="ichip-label">{label}</span>
    </span>
  );
}

export function UserBubble(props: {
  chat: RefChat;
  text: string;
  images?: ImageRef[];
  /** 'sending': the host hasn't confirmed it yet; 'waiting': the stream is down (Z-50). */
  pending?: 'sending' | 'waiting';
  /** ✕ on a waiting outbox bubble. */
  onDismiss?: () => void;
  /** A steered message (Z-54) — smaller, marked. */
  steer?: boolean;
  commands: () => ReadonlySet<string>;
  /** Edit this message (Z-28): receives the pencil's anchor rect. */
  onEdit?: (anchor: DOMRect) => void;
  /** The row menu's payload for this message (Copy/Quote/Fork/Edit). */
  menu?: () => MessageTarget | undefined;
  /** Where Quote puts the chip (the session's composer key). */
  composerKey?: string;
}) {
  const [expanded, setExpanded] = createSignal(false);
  const [folds, setFolds] = createSignal(false);
  const [fullHeight, setFullHeight] = createSignal(FOLD_PX);
  const [bodyEl, setBodyEl] = createSignal<HTMLSpanElement>();
  const [rootEl, setRootEl] = createSignal<HTMLDivElement>();

  const [mentions] = createResource(
    () => {
      const tokens = mentionTokens(props.text);
      return tokens.length ? tokens : undefined;
    },
    async (tokens) => {
      const stats = await lookupPaths(props.chat, tokens.map(mentionPath));
      const out = new Map<string, { resolved: string; kind: 'file' | 'dir' }>();
      for (const t of tokens) {
        const s = stats.get(mentionPath(t));
        if (s && s.kind !== 'missing') out.set(t, { resolved: s.resolved, kind: s.kind });
      }
      return out;
    },
  );
  const segs = createMemo(() => {
    const known = mentions.latest;
    return bubbleSegments(props.text, props.commands(), known?.size ? (token) => known.has(token) : undefined);
  });
  const resolvedFor = (chip: Chip) => (chip.kind === 'path' ? mentions.latest?.get(chip.raw) : undefined);

  createEffect(() => {
    const el = rootEl();
    if (!el) return;
    const menu = attachMessageMenu(el, () => props.menu?.());
    const swipe = props.composerKey ? attachQuoteSwipe(el, () => props.text, () => props.composerKey!) : undefined;
    onCleanup(() => {
      menu();
      swipe?.();
    });
  });

  // Measure the body; reset to the collapsed state whenever the virtualizer
  // hands this row a different message (ZCode: reuse must not keep expansion).
  const measure = () => {
    const el = bodyEl();
    if (!el) return;
    setFullHeight(el.scrollHeight);
    setFolds(el.scrollHeight > FOLD_PX + 1);
  };
  createEffect(
    on(
      () => props.text,
      () => {
        setExpanded(false);
        requestAnimationFrame(measure);
      },
    ),
  );
  createEffect(() => {
    const el = bodyEl();
    if (!el) return;
    requestAnimationFrame(measure);
    if (typeof ResizeObserver === 'undefined') return;
    let frame = 0;
    const observer = new ResizeObserver(() => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(measure);
    });
    observer.observe(el);
    onCleanup(() => {
      cancelAnimationFrame(frame);
      observer.disconnect();
    });
  });

  const bodyStyle = () =>
    !folds()
      ? undefined
      : { 'max-height': expanded() ? `${fullHeight()}px` : `${FOLD_PX}px`, transition: `max-height ${FOLD_MS}ms ease-out` };

  return (
    <div
      class="chat-user tl-user"
      ref={setRootEl}
      data-pending={props.pending === 'sending' ? '' : undefined}
      data-waiting={props.pending === 'waiting' ? '' : undefined}
      data-steer={props.steer ? '' : undefined}
    >
      <Show when={props.steer}>
        <span class="tl-steer-tag">Steer</span>
      </Show>
      <div class="chat-bubble tl-bubble" data-folded={folds() && !expanded() ? '' : undefined}>
        <span class="tl-bubble-body" ref={setBodyEl} style={bodyStyle()}>
          <For each={segs()}>{(seg) => ('chip' in seg ? <BubbleChip chat={props.chat} chip={seg.chip} resolved={resolvedFor(seg.chip)} images={props.images} /> : <BubbleText chat={props.chat} text={seg.text} />)}</For>
        </span>
        <Show when={folds()}>
          <button
            type="button"
            class="tl-fold-btn"
            data-testid="tl-bubble-fold"
            aria-expanded={expanded()}
            aria-label={expanded() ? 'Collapse message' : 'Expand message'}
            title={expanded() ? 'Collapse message' : 'Show the whole message'}
            onClick={() => setExpanded(!expanded())}
          >
            <Show when={expanded()} fallback={<ChevronDown size={15} />}>
              <ChevronUp size={15} />
            </Show>
          </button>
        </Show>
      </div>
      <ImageChipRow chat={props.chat} images={props.images ?? []} />
      <Show when={props.onEdit}>
        <div class="tl-user-actions">
          <button
            type="button"
            class="tl-act"
            data-testid="tl-user-edit"
            aria-label="Edit message"
            data-tip="Edit"
            onClick={(event) => props.onEdit?.(event.currentTarget.getBoundingClientRect())}
          >
            <Pencil size={14} />
          </button>
        </div>
      </Show>
      <Show when={props.pending === 'waiting'}>
        <span class="chat-user-note">
          Waiting for connection…
          <Show when={props.onDismiss}>
            <button type="button" class="chat-outbox-remove" aria-label="Remove from outbox" onClick={() => props.onDismiss?.()}>
              ✕
            </button>
          </Show>
        </span>
      </Show>
      <Show when={props.pending === 'sending'}>
        <span class="chat-user-note">Sending…</span>
      </Show>
    </div>
  );
}
