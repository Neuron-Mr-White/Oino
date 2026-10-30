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
 * WP-A9's rules until WP-A11 refines them). Attachments beyond inline images
 * and the toolbar are WP-A13.
 */
import { createEffect, createMemo, createSignal, on, onCleanup, For, Show, untrack } from 'solid-js';
import { Image as ImageIcon, Paperclip, Quote } from 'lucide-solid';
import type { SessionHandle } from '../../lib/session/store';
import type { OutboxImage } from '../../lib/session/types';
import { allCommands, applyAt, atQuery, isKnownCommand, rankCommands, slashQuery } from '../../lib/chat/commands';
import { commandNames } from '../timeline/context';
import { fsList, fsSearch, fsStat } from '../../lib/fs';
import { layout } from '../../lib/layout';
import { panes, type PaneScope, type PaneView } from '../../lib/panes';
import { addTaskFromChat } from '../../lib/session/open';
import { followUpMode, sendModeForEnter } from '../../lib/composer/sendMode';
import { SendButton } from './SendButton';
import { showActionNotice } from '../../lib/timeline/actions';
import { blobToBase64, shrinkImage } from '../../lib/imageShrink';
import {
  addComposerChip,
  clearComposer,
  composerDraft,
  composerSendText,
  focusComposer,
  registerComposerFocus,
  removeComposerChip,
  removeComposerImage,
  removeComposerQuote,
  setComposerImages,
  setComposerText,
  type ComposerChip,
} from '../../lib/composer/drafts';
import { isLongPaste, pastedTextLabel } from '../../lib/composer/paste';
import { promptHistory, walkDown, walkUp } from '../../lib/composer/history';
import { composerHistory } from '../../lib/composer/undo';
import { pathCandidates } from '../../lib/composer/chips';
import { ChipEditor, type ChipEditorApi } from './ChipEditor';
import { SlashMenu } from './SlashMenu';
import { MentionMenu, type MentionItem } from './MentionMenu';
import { PlusMenu } from './PlusMenu';
import { PastedTextChip } from './PastedTextChip';
import './composer.css';

/** A paste's tooltip head (Z-61): the first lines, whitespace-collapsed. */
const pasteHead = (text: string): string => {
  const flat = text.replace(/\s+/g, ' ').trimEnd();
  return flat.length > 220 ? `${flat.slice(0, 219).trimEnd()}…` : flat;
};

export function Composer(props: { session: SessionHandle; cwd?: () => string | undefined; readOnly?: boolean; phone?: boolean }) {
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
    props.readOnly
      ? 'Read-only — this session is open elsewhere'
      : running()
        ? `Keep typing to ${followUpMode() === 'steer' ? 'steer' : 'queue'}…`
        : !hasHistory()
          ? props.phone
            ? 'Message pi…'
            : 'Message pi, / commands, @ files'
          : 'Ask for follow-up changes';

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
    return rankCommands(allCommands(props.session.state.commands), q.query, 12);
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
  const [sel, setSel] = createSignal(0);
  createEffect(() => {
    void slashItems().length;
    void mentionItems().length;
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

  /* ── images (in-memory draft images, D-6; A13 owns upload states) ─── */
  const attachImages = async (files: readonly File[]) => {
    const picked = files.filter((f) => f.type.startsWith('image/') || /\.(png|jpe?g|gif|webp|bmp|avif)$/i.test(f.name));
    const refused = files.length - picked.length;
    if (refused > 0) showActionNotice('File uploads need a newer build — images work today');
    if (!picked.length) return;
    const images: OutboxImage[] = [];
    for (const file of picked) {
      try {
        const shrunk = await shrinkImage(file);
        images.push({ mimeType: shrunk.mime, data: await blobToBase64(shrunk.blob) });
      } catch (error) {
        console.warn(`[attach] could not attach ${file.name}: ${String(error)}`);
      }
    }
    if (images.length) setComposerImages(key(), [...draft().images, ...images]);
    focusComposer(key());
  };
  let photoInput: HTMLInputElement | undefined;
  let cameraInput: HTMLInputElement | undefined;
  let fileInput: HTMLInputElement | undefined;

  /* ── the `+` menu (Z-63) ──────────────────────────────────────────── */
  const [plusAnchor, setPlusAnchor] = createSignal<DOMRect | null>(null);
  const openSideQuestion = (question?: string) => {
    // WP-A10's `btw` tab; opening it now means the entry works the day the
    // view plugs in (the pane shows "not available" until then).
    const scope: PaneScope = { hostId: hostId(), projectKey: cwd() || '~' };
    const view: PaneView = { sessionId: props.session.sessionId() };
    panes().open(scope, 'side', view, { kind: 'btw', ...(question ? { params: { question }, title: 'Side chat' } : {}) });
    layout().toggleSide(true);
  };

  /* ── focus handoff (A7: quotes and Edit focus the field) ──────────── */
  createEffect(() => {
    const k = key();
    const cleanup = registerComposerFocus(k, () => editor?.focus());
    onCleanup(cleanup);
  });

  /* ── send ─────────────────────────────────────────────────────────── */
  const send = (mode?: 'prompt' | 'steer' | 'followUp') => {
    if (props.readOnly) return;
    const current = draft();
    const text = composerSendText(current);
    const images = current.images;
    if (!text.trim() && !images.length) return;
    const cmd = /^\/(\S+)(?:\s+([\s\S]*))?$/.exec(text.trim());
    // `/btw` opens the side question instead of sending to the main chat (Z-31).
    if (cmd && (cmd[1] === 'btw' || cmd[1] === 'unipi:btw')) {
      openSideQuestion(cmd[2]?.trim() || undefined);
      afterSend(text);
      return;
    }
    // A `/name` neither the session nor the app knows never becomes a prompt.
    if (cmd && !isKnownCommand(cmd[1]!, props.session.state.commands)) {
      showActionNotice(`Unknown command /${cmd[1]}`);
      return;
    }
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
  /** After a send the draft starts over; one Ctrl/⌘+Z brings back what was sent. */
  const afterSend = (outgoing: string) => {
    clearComposer(key());
    history().reset(outgoing);
    setHistRev((n) => n + 1);
    setDismissed(null);
    histIndex = null;
  };
  const stop = () => {
    if (!props.readOnly) void props.session.abort();
  };
  /** "Send now (stops run)" (Z-52): stop, then send the text as a prompt. */
  const sendNow = () => {
    if (props.readOnly) return;
    const current = draft();
    const text = composerSendText(current);
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
      if (slashItems().length || menuItems()?.length) {
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
      if (items.length || mentions?.length) {
        e.preventDefault();
        const count = items.length || mentions!.length;
        setSel((i) => (e.key === 'ArrowUp' ? (i <= 0 ? count - 1 : i - 1) : i + 1 >= count ? 0 : i + 1));
        return;
      }
      if (!e.shiftKey && historyKey(e, draft().text)) e.preventDefault();
      return;
    }
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing && e.keyCode !== 229) {
      const items = slashItems();
      const mentions = menuItems();
      if (items.length) {
        e.preventDefault();
        pickCommand(items[Math.min(sel(), items.length - 1)]!.name);
        return;
      }
      if (mentions?.length) {
        e.preventDefault();
        pickMention(mentions[Math.min(sel(), mentions.length - 1)]!);
        return;
      }
      // A touch keyboard's Enter is a newline; the send button sends.
      if (typeof matchMedia === 'function' && matchMedia('(pointer: coarse)').matches) return;
      e.preventDefault();
      send(sendModeForEnter(e, running()));
    }
  };

  const chips = () => draft().chips;
  const empty = () => !draft().text.trim() && !draft().quotes.length && !draft().images.length && !draft().chips.length;

  return (
    <div class="tl-composer-wrap" data-testid="tl-composer-wrap">
      <Show when={slashItems().length}>
        <SlashMenu items={slashItems()} selected={Math.min(sel(), slashItems().length - 1)} onPick={pickCommand} onHover={setSel} />
      </Show>
      <Show when={menuItems()}>
        <MentionMenu items={menuItems()!} selected={Math.min(sel(), menuItems()!.length - 1)} onPick={pickMention} onHover={setSel} />
      </Show>
      <Show when={draft().quotes.length > 0 || draft().images.length > 0 || chips().length > 0}>
        <div class="tl-composer-chips" data-testid="tl-composer-chips">
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
          <For each={draft().images}>
            {(image, i) => (
              <span class="tl-chip" data-testid="tl-draft-image" title={`Image ${i() + 1}`}>
                <img class="tl-chip-thumb" src={`data:${image.mimeType};base64,${image.data}`} alt="" />
                <button type="button" class="tl-chip-x" aria-label={`Remove image ${i() + 1}`} onClick={() => removeComposerImage(key(), i())}>
                  ✕
                </button>
              </span>
            )}
          </For>
        </div>
      </Show>
      <div class="tl-composer" data-testid="tl-composer" data-readonly={props.readOnly ? '' : undefined}>
        <input
          ref={photoInput}
          type="file"
          accept="image/*"
          multiple
          hidden
          aria-label="Attach photos"
          onChange={(e) => {
            void attachImages([...(e.currentTarget.files ?? [])]);
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
            void attachImages([...(e.currentTarget.files ?? [])]);
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
            void attachImages([...(e.currentTarget.files ?? [])]);
            e.currentTarget.value = '';
          }}
        />
        <button
          type="button"
          class="composer-plus-btn"
          aria-label="Add"
          aria-expanded={plusAnchor() !== null}
          disabled={props.readOnly}
          onClick={(e) => setPlusAnchor((e.currentTarget as HTMLElement).getBoundingClientRect())}
        >
          <Paperclip size={16} />
        </button>
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
          onPasteFiles={props.phone ? undefined : (files) => void attachImages(files)}
          onPasteText={onPasteText}
        />
        <SendButton running={running()} disabled={props.readOnly} empty={empty()} onSend={() => send()} onStop={stop} onSendNow={sendNow} />
      </div>
      <Show when={plusAnchor()}>
        {(anchor) => (
          <PlusMenu
            anchor={anchor()}
            onDismiss={() => setPlusAnchor(null)}
            onPhoto={() => photoInput?.click()}
            onCamera={() => cameraInput?.click()}
            onFile={() => fileInput?.click()}
            onSideQuestion={() => openSideQuestion(draft().text.trim() || undefined)}
            onAddTask={() => addTaskFromChat(hostId(), cwd() || undefined)}
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
