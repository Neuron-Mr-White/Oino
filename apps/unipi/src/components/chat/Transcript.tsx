/**
 * Chat transcript rows (UNI-113): user bubbles, assistant prose (markdown,
 * no bubble), folded process groups (thinking + tool calls), unipi cards,
 * compaction, errors, turn footers. deepseek-harness patterns, warm theme.
 */
import { createEffect, createMemo, createResource, createSignal, For, Match, onCleanup, Show, Switch } from 'solid-js';
import {
  Bot,
  Brain,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  ExternalLink,
  Eye,
  FilePen,
  FileText,
  FolderOpen,
  Globe,
  ListChecks,
  MessageCircleQuestion,
  Image as ImageIcon,
  Search,
  SquareTerminal,
  Sparkles,
  SquareSlash,
  Wrench,
  Database,
} from 'lucide-solid';
import { useNavigate } from '@solidjs/router';
import { openFilePath, resolvePathToken } from '../../lib/media/pathTokens';
import { BoardSheet } from '../board/BoardSheet';
import { knownPrefixes, splitTaskRefs } from '../../lib/board/portal';
import { openTaskFromChat } from '../../lib/session/open';
import { fileRefLabel, verifyCandidates, type FileRefHit } from '../../lib/chat/fileRefs';
import { toolBaseDir } from '../../lib/chat/fileRefScan';
import { attachFileRefGestures, handleFileRefClick } from '../../lib/chat/fileRefGestures';
import { openFileRef } from '../../lib/chat/fileRefOpen';
import { lookupPaths, pathStatUnavailable } from '../../lib/chat/pathStat';
import { FileRefText } from './FileRefText';
import { tokenize, type Chip, type Seg } from '../../lib/chat/chips';
import { setEditorTarget } from '../../lib/editorTarget';
import { openReviewViewer } from './ReviewViewer';
import type { JSX } from 'solid-js';
import {
  diffStat,
  formatDuration,
  processSummary,
  textOf,
  thinkingPreview,
  toolCategory,
  toolSubject,
  turnFileEditLine,
  type Entry,
  type ProcessItem,
  type Row,
  type ToolCategory,
  type ToolRun,
  type TurnFile,
} from '../../lib/chat/model';
import { renderBlocks } from '../../lib/chat/markdown';
import { allAdditions, editDiff, renderDiffHtml } from '../../lib/markdown';
import { attachmentsOf, type ImageRef } from '../../lib/chat/model';
import type { Chat } from '../../lib/chat/store';
import { MdBlock } from './MdBlock';
import { UnipiCard } from './Cards';
import { AttachmentChipRow, ImageChipRow, openImagePreviewer } from './ImageChip';

const ICON: Record<ToolCategory, (p: { size: number }) => JSX.Element> = {
  read: (p) => <FileText size={p.size} />,
  search: (p) => <Search size={p.size} />,
  edit: (p) => <FilePen size={p.size} />,
  run: (p) => <SquareTerminal size={p.size} />,
  web: (p) => <Globe size={p.size} />,
  ask: (p) => <MessageCircleQuestion size={p.size} />,
  agent: (p) => <Bot size={p.size} />,
  memory: (p) => <Database size={p.size} />,
  board: (p) => <ListChecks size={p.size} />,
  other: (p) => <Wrench size={p.size} />,
};

export function Prose(props: { text: string; live: boolean; stopped?: boolean; chat?: Chat }) {
  const blocks = createMemo(() => renderBlocks(props.text, props.live));
  return (
    <div class="chat-prose md" data-live={props.live ? '' : undefined}>
      <For each={blocks()}>{(html) => <MdBlock html={html} class="chat-block" chat={props.chat} />}</For>
      <Show when={props.stopped}>
        <span class="chat-stopped">Stopped</span>
      </Show>
    </div>
  );
}

/** Task refs (UNI-181) in a plain run, tappable. */
function TaskRefRun(props: { chat: Chat; text: string; prefixes?: Set<string> }) {
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
              openTaskFromChat(props.chat.hostId, value.ref, props.chat.state?.session?.cwd);
            }}
          >
            {value.ref}
          </span>
        )
      }
    </For>
  );
}

/** Plain text with tappable task refs (UNI-181) and verified file refs
 * (UNI-204, `FileRefText`): the user's own messages. */
export function TaskRefText(props: { chat: Chat; text: string }) {
  const [prefixes] = createResource(() => props.chat?.hostId, knownPrefixes);
  return <FileRefText chat={props.chat} text={props.text} mode="prose" plain={(text) => <TaskRefRun chat={props.chat} text={text} prefixes={prefixes()} />} />;
}

/** A sent message's text as plain runs + inline chips (UNI-212): the same
 * tokenizer as the composer — attachments (`[file: name]@path`), a
 * leading `/skill:x` or `/command` from this session's list, pi's
 * expanded `<skill …>` block, and (UNI-285) an `[image N]` back to that
 * image. Whitespace between two file chips collapses
 * to one space, so a stack of attachments stays a compact wrap of pills
 * instead of one tall line each. */
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

function BubbleChip(props: { chat: Chat; chip: Chip; resolved?: { resolved: string; kind: 'file' | 'dir' }; images?: readonly ImageRef[] }) {
  const [failed, setFailed] = createSignal(false);
  const c = props.chip;
  const label = c.kind === 'file' ? c.name : c.kind === 'image' ? `image ${c.n}` : c.kind === 'path' ? fileRefLabel(c.path.replace(/\/+$/, '')).label + (props.resolved?.kind === 'dir' ? '/' : '') : c.kind === 'skill' ? c.name : `/${c.name}`;
  const path = c.kind === 'file' ? c.path : c.kind === 'path' ? c.path : undefined;
  const open = async (event: MouseEvent) => {
    event.stopPropagation();
    // UNI-285: `[image N]` opens that image in the previewer.
    if (c.kind === 'image') {
      const index = (c.n ?? 1) - 1;
      if (index >= 0 && index < (props.images?.length ?? 0)) void openImagePreviewer(props.chat, props.images!, index);
      return;
    }
    if (!path) return;
    setFailed(false);
    try {
      if (props.resolved) await openFileRef(props.chat, props.resolved);
      else await openFilePath(props.chat, resolvePathToken(path, props.chat.state?.session?.cwd));
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
  chat: Chat;
  text: string;
  images: ImageRef[];
  pending?: boolean;
  waiting?: boolean;
  harness?: { source?: string; title?: string };
  /** UNI-251: a typed `/command …` and the harness turn it opened. */
  command?: { name: string; tag?: string; body?: string };
  onRemoveWaiting?: () => void;
}) {
  const [open, setOpen] = createSignal(false);
  const [injectedOpen, setInjectedOpen] = createSignal(false);
  // UNI-203/212: `[file: name]@path` mentions, skills and commands render as
  // inline chips where they sit in the text, never as raw tokens. A command
  // row's own command always chips (UNI-251), even when this session's list
  // doesn't name it (yet).
  const commandNames = () => {
    const names = new Set((props.chat.state?.commands ?? []).filter((c) => c.source !== 'builtin').map((c) => c.name));
    if (props.command?.name) names.add(props.command.name);
    return names;
  };
  // UNI-204: `@path` mentions the PC confirms (paths_stat, cached per chat) become path chips.
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
    return bubbleSegments(props.text, commandNames(), known?.size ? (token) => known.has(token) : undefined);
  });
  const resolvedFor = (chip: Chip) => (chip.kind === 'path' ? mentions.latest?.get(chip.raw) : undefined);
  let bubbleEl: HTMLDivElement | undefined;
  createEffect(() => {
    if (!bubbleEl) return;
    onCleanup(attachFileRefGestures(bubbleEl, () => props.chat));
  });
  const bodyText = () => segs().map((x) => ('chip' in x ? ' ' : x.text)).join('');
  const long = () => bodyText().length > 600 || bodyText().split('\n').length > 12;
  return (
    <Show
      when={!props.harness}
      fallback={
        <button type="button" class="chat-harness" onClick={() => setOpen(!open())}>
          <span class="chat-harness-head">
            <Sparkles size={14} />
            <span class="chat-harness-source">{props.harness!.source ?? 'UniPi'}</span>
            <span class="truncate">{props.harness!.title}</span>
            <ChevronDown size={14} class="chat-chev" data-open={open() ? '' : undefined} />
          </span>
          <Show when={open()}>
            <div class="chat-harness-body md">
              <For each={renderBlocks(bodyText(), false)}>{(html) => <MdBlock html={html} />}</For>
            </div>
          </Show>
        </button>
      }
    >
      <div class="chat-user" data-pending={props.pending ? '' : undefined} data-waiting={props.waiting ? '' : undefined}>
        <div class="chat-bubble" ref={bubbleEl} data-clamped={long() && !open() ? '' : undefined} onClick={() => long() && setOpen(!open())}>
          <For each={segs()}>{(seg) => ('chip' in seg ? <BubbleChip chat={props.chat} chip={seg.chip} resolved={resolvedFor(seg.chip)} images={props.images} /> : <TaskRefText chat={props.chat} text={seg.text} />)}</For>
        </div>
        <ImageChipRow chat={props.chat} images={props.images} />
        <Show when={props.command?.tag}>
          <Show
            when={props.command?.body}
            fallback={
              <span class="chat-injected-chip" data-testid="chat-injected-chip">
                <Sparkles size={12} />
                <span class="truncate">{props.command!.tag}</span>
              </span>
            }
          >
            <button
              type="button"
              class="chat-injected-chip"
              data-testid="chat-injected-chip"
              aria-expanded={injectedOpen()}
              title="What UniPi added to this message"
              onClick={(e) => {
                e.stopPropagation();
                setInjectedOpen(!injectedOpen());
              }}
            >
              <Sparkles size={12} />
              <span class="truncate">{props.command!.tag}</span>
              <ChevronDown size={13} class="chat-chev" data-open={injectedOpen() ? '' : undefined} />
            </button>
            <Show when={injectedOpen()}>
              <div class="chat-injected-body md" data-testid="chat-injected-body" onClick={(e) => e.stopPropagation()}>
                <For each={renderBlocks(props.command!.body!, false)}>{(html) => <MdBlock html={html} />}</For>
              </div>
            </Show>
          </Show>
        </Show>
        <Show when={props.waiting}>
          <span class="chat-user-note">
            Waiting for connection…
            <Show when={props.onRemoveWaiting}>
              <button type="button" class="chat-outbox-remove" aria-label="Remove from outbox" onClick={() => props.onRemoveWaiting?.()}>
                ✕
              </button>
            </Show>
          </span>
        </Show>
        <Show when={props.pending && !props.waiting}>
          <span class="chat-user-note">Sending…</span>
        </Show>
      </div>
    </Show>
  );
}

/** The file/dir a tool names in its arguments (read/edit/write `path`, grep/find/ls `path`) — the
 * candidate for a tappable tool subject (UNI-204), verified like any other ref. */
function toolArgPath(tool: ToolRun): string | undefined {
  const cat = toolCategory(tool.name);
  if (cat !== 'read' && cat !== 'edit' && cat !== 'search') return undefined;
  for (const k of ['path', 'file_path', 'filePath']) {
    const v = tool.args[k];
    if (typeof v === 'string' && v.trim() && !/\s/.test(v.trim())) return v.trim().replace(/^@/, '');
  }
  return undefined;
}

/** Verifies the tool's own path argument (one `paths_stat`, cached per chat). */
function useToolSubjectRef(tool: () => ToolRun, chat: () => Chat | undefined) {
  const [hit] = createResource(
    () => {
      const c = chat();
      const p = toolArgPath(tool());
      return c && p && tool().state !== 'preparing' ? { c, p } : undefined;
    },
    async ({ c, p }) => {
      const hit = (await verifyCandidates(c, [{ start: 0, end: p.length, path: p, explicit: p.startsWith('/') }]))[0];
      if (hit || !pathStatUnavailable(c) || toolCategory(tool().name) === 'search') return hit;
      // A pi without `paths_stat` (or no answer): link the subject unverified, resolved against the
      // session cwd — what the row did before UNI-204, so read/edit rows keep their Open.
      const resolved = resolvePathToken(p, c.state?.session?.cwd);
      return { start: 0, end: p.length, path: p, resolved, kind: p.endsWith('/') ? 'dir' : 'file' } satisfies FileRefHit;
    },
  );
  return () => hit.latest as FileRefHit | undefined;
}

/** Output of grep/find/ls is relative to the tool's `path` argument; bash/read output to the cwd. */
const outputBase = (tool: ToolRun) => (toolCategory(tool.name) === 'search' || tool.name === 'ls' ? toolBaseDir(tool.args.path) : undefined);

function ToolBody(props: { tool: ToolRun; cwd?: string; chat?: Chat }) {
  const t = () => props.tool;
  const cat = () => toolCategory(t().name);
  const out = () => t().output ?? t().live ?? '';
  const attachments = () => attachmentsOf(t().details);
  const edits = () => {
    const a = t().args;
    if (Array.isArray(a.edits)) return a.edits as Array<{ oldText?: string; newText?: string }>;
    if (typeof a.oldText === 'string' || typeof a.newText === 'string') return [{ oldText: a.oldText as string, newText: a.newText as string }];
    return [];
  };
  return (
    <div class="tool-body">
      <Switch
        fallback={
          <div class="tool-io">
            <div class="tool-io-sec">
              <span class="tool-io-label">In</span>
              <pre>{JSON.stringify(t().args, null, 2)}</pre>
            </div>
            <Show when={out()}>
              <div class="tool-io-sec" data-error={t().state === 'error' ? '' : undefined}>
                <span class="tool-io-label">Out</span>
                <pre>
                <FileRefText chat={props.chat} text={tail(out(), 4000)} mode="code" base={outputBase(t())} />
              </pre>
              </div>
            </Show>
          </div>
        }
      >
        <Match when={cat() === 'run'}>
          <div class="tool-term">
            <div class="tool-term-cmd">
              <span class="tool-term-prompt">$</span>
              <span>{String(t().args.command ?? t().name)}</span>
            </div>
            <Show when={out()}>
              <pre class="tool-term-out" data-error={t().state === 'error' ? '' : undefined}>
                <FileRefText chat={props.chat} text={tail(out(), 6000)} mode="code" />
              </pre>
            </Show>
          </div>
        </Match>
        <Match when={cat() === 'edit' && edits().length > 0}>
          <div class="tool-diff diffview-wrap" innerHTML={renderDiffHtml({ files: [{ hunks: edits().map((e) => editDiff(e.oldText ?? '', e.newText ?? '')) }] })} />
          <Show when={t().state === 'error' && out()}>
            <pre class="tool-err">
              <FileRefText chat={props.chat} text={tail(out(), 1200)} mode="code" />
            </pre>
          </Show>
        </Match>
        <Match when={cat() === 'edit' && t().name === 'write' && typeof t().args.content === 'string'}>
          <div class="tool-diff diffview-wrap" innerHTML={renderDiffHtml(allAdditions(String(t().args.content)))} />
        </Match>
        <Match when={(cat() === 'read' || cat() === 'search' || cat() === 'web' || cat() === 'memory') && out()}>
          <pre class="tool-read" data-error={t().state === 'error' ? '' : undefined}>
            <FileRefText chat={props.chat} text={head(out(), 3000)} mode="code" base={outputBase(t())} />
          </pre>
        </Match>
      </Switch>
      <Show when={props.chat && attachments().length > 0}>
        <AttachmentChipRow chat={props.chat!} attachments={attachments()} />
      </Show>
    </div>
  );
}

const tail = (s: string, n: number) => (s.length > n ? `…\n${s.slice(s.length - n)}` : s);
const head = (s: string, n: number) => (s.length > n ? `${s.slice(0, n)}\n…` : s);

function ToolRow(props: { tool: ToolRun; cwd?: string; chat?: Chat }) {
  const [open, setOpen] = createSignal(false);
  const t = () => props.tool;
  const cat = () => toolCategory(t().name);
  const subject = () => toolSubject(t(), props.cwd);
  const stat = () => diffStat(t());
  const errLine = () => (t().state === 'error' ? (t().output ?? '').split('\n').find((l) => l.trim()) : undefined);
  const busy = () => t().state === 'running' || t().state === 'preparing';
  // UNI-204: the file/dir this tool names is tappable right in the row (opens it; long-press = menu),
  // once the PC confirms it exists. The rest of the row still expands/collapses.
  const subjectRef = useToolSubjectRef(t, () => props.chat);
  let rowEl: HTMLDivElement | undefined;
  createEffect(() => {
    if (!rowEl || !props.chat) return;
    onCleanup(attachFileRefGestures(rowEl, () => props.chat));
  });
  return (
    <div class="tool" data-state={t().state} ref={rowEl}>
      <button
        type="button"
        class="tool-row"
        aria-expanded={open()}
        disabled={t().state === 'preparing'}
        onClick={(event) => {
          if (rowEl && handleFileRefClick(event, rowEl, props.chat)) return;
          setOpen(!open());
        }}
      >
        <span class="tool-icon">{ICON[cat()]({ size: 15 })}</span>
        <span class="tool-name" classList={{ shimmer: busy() }}>
          {t().name}
        </span>
        <span class="tool-sep" />
        <Show
          when={!errLine() && t().state !== 'preparing' && subjectRef()}
          fallback={
            <span class="tool-subject" data-error={errLine() ? '' : undefined}>
              {errLine() ?? (t().state === 'preparing' ? 'preparing…' : subject())}
            </span>
          }
        >
          {(ref) => (
            <span class="tool-subject">
              <span class="file-ref" role="link" data-file-ref={ref().resolved} data-file-kind={ref().kind} title={ref().resolved} aria-label={`Open ${subject()}`}>
                {subject()}
              </span>
            </span>
          )}
        </Show>
        <Show when={stat()}>
          <span class="tool-stat">
            <span class="add">+{stat()!.add}</span> <span class="del">−{stat()!.del}</span>
          </span>
        </Show>
      </button>
      <Show when={open()}>
        <ToolBody tool={t()} cwd={props.cwd} chat={props.chat} />
      </Show>
    </div>
  );
}

function ThinkingRow(props: { text: string; live: boolean }) {
  const [open, setOpen] = createSignal(false);
  return (
    <div class="think">
      <button type="button" class="tool-row" aria-expanded={open()} onClick={() => setOpen(!open())}>
        <span class="tool-icon">
          <Brain size={15} />
        </span>
        <span class="tool-name" classList={{ shimmer: props.live }}>
          Thinking
        </span>
        <span class="tool-sep" />
        <span class="tool-subject fade">{thinkingPreview(props.text, props.live)}</span>
      </button>
      <Show when={open()}>
        <div class="think-body md">
          <For each={renderBlocks(props.text, props.live)}>{(html) => <MdBlock html={html} />}</For>
        </div>
      </Show>
    </div>
  );
}

function StepRow(props: { entry: Entry }) {
  const d = () => ((props.entry as { data?: Record<string, unknown> }).data ?? {}) as { kind?: string; name?: string; arg?: string; text?: string; output?: string; agent?: string };
  const tool = (): ToolRun => ({ id: props.entry.id, name: d().name ?? d().kind ?? 'step', args: { description: d().arg ?? d().text ?? '' }, state: 'ok', output: d().output });
  return (
    <div class="step">
      <ToolRow tool={tool()} />
    </div>
  );
}

export function ProcessGroup(props: { items: ProcessItem[]; live: boolean; cwd?: string; chat?: Chat }) {
  const [open, setOpen] = createSignal(false);
  const summary = createMemo(() => processSummary(props.items, props.live, props.cwd));
  const lead = createMemo<ToolCategory | 'think'>(() => {
    const last = [...props.items].reverse().find((i) => i.kind === 'tool');
    return last && last.kind === 'tool' ? toolCategory(last.tool.name) : 'think';
  });
  const anyError = () => props.items.some((i) => i.kind === 'tool' && i.tool.state === 'error');
  return (
    <div class="proc" data-live={props.live ? '' : undefined} data-open={open() ? '' : undefined}>
      <button type="button" class="proc-head" aria-expanded={open()} onClick={() => setOpen(!open())}>
        <span class="proc-icon">
          <Show when={lead() !== 'think'} fallback={<Brain size={15} />}>
            {ICON[lead() as ToolCategory]({ size: 15 })}
          </Show>
        </span>
        <span class="proc-title" classList={{ shimmer: props.live }}>
          {summary().title}
        </span>
        <Show when={summary().detail}>
          <span class="proc-detail" data-error={anyError() && !props.live ? '' : undefined}>
            {summary().detail}
          </span>
        </Show>
        <ChevronRight size={15} class="proc-chev" />
      </button>
      <Show when={open()}>
        <div class="proc-body">
          <For each={props.items}>
            {(item) => (
              <Switch>
                <Match when={item.kind === 'thinking' && item}>{(it) => <ThinkingRow text={it().text} live={it().live} />}</Match>
                <Match when={item.kind === 'tool' && item}>{(it) => <ToolRow tool={it().tool} cwd={props.cwd} chat={props.chat} />}</Match>
                <Match when={item.kind === 'step' && item}>{(it) => <StepRow entry={it().entry} />}</Match>
              </Switch>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}

export function TurnEnd(props: { ms?: number; cost?: number; stopped?: boolean; files?: TurnFile[]; chat?: Chat }) {
  return (
    <>
      <div class="turn-end">
        <span>{props.stopped ? 'Stopped' : props.ms !== undefined ? `Done in ${formatDuration(props.ms)}` : 'Done'}</span>
        <Show when={props.cost && props.cost >= 0.0005}>
          <span>· ${props.cost!.toFixed(props.cost! < 0.1 ? 3 : 2)}</span>
        </Show>
      </div>
      <Show when={props.files && props.files.length > 0}>
        <ChangesCard files={props.files!} chat={props.chat} />
      </Show>
    </>
  );
}

/** "Changes this turn" (UNI-156): every file edited/written, folded to +N −M.
 * A file with several edits (several edit/write calls merged by path, or
 * one edit with several `edits[]`) lists each as its own sub-row
 * ("edit 2 · +a −d") that expands on its own (UNI-192); a single-edit file
 * behaves as before. "Open in editor" (UNI-155) jumps straight to the
 * edit's first changed line; "Review" opens the full-screen read-only
 * viewer at that edit. */
function ChangesCard(props: { files: TurnFile[]; chat?: Chat }) {
  const navigate = useNavigate();
  const openInEditor = (file: TurnFile, editIndex = 0) => {
    const hostId = props.chat?.hostId;
    if (!hostId) return;
    const line = turnFileEditLine(file.edits[editIndex] ?? file.edits[0]!);
    // Tool args often carry a relative path (`src/a.ts`) or `~/…`; resolve
    // against the session cwd before the host sees it (UNI-178).
    const cwd = props.chat?.state?.session?.cwd;
    setEditorTarget({ hostId, path: resolvePathToken(file.path, cwd), line });
    // The Show-all drawer (BoardSheet, a Corvu portal) must close *before*
    // the editor route mounts (UNI-192 bug): left open, it sits over the
    // editor at a higher z-index and swallows Back, so "Open" looked like
    // it did nothing.
    setSheetOpen(false);
    navigate('/files/edit');
  };
  const review = (file: TurnFile, editIndex = 0) => {
    setSheetOpen(false);
    openReviewViewer(file, editIndex);
  };
  // `path` for a single-edit file's own row, `path#index` for one of several
  // sub-rows (UNI-192: each edit expands on its own).
  const [expanded, setExpanded] = createSignal<string | undefined>(undefined);
  const [sheetOpen, setSheetOpen] = createSignal(false);
  let sheetEl: HTMLDivElement | undefined;
  // Opened from a tapped file: bring that file into view in the drawer.
  createEffect(() => {
    if (!sheetOpen()) return;
    const key = expanded();
    if (!key) return;
    const path = key.split('#')[0];
    queueMicrotask(() => [...(sheetEl?.querySelectorAll<HTMLElement>('.changes-file') ?? [])].find((el) => el.dataset.path === path)?.scrollIntoView?.({ block: 'start' }));
  });
  const totals = createMemo(() => props.files.reduce((acc, f) => ({ add: acc.add + f.add, del: acc.del + f.del }), { add: 0, del: 0 }));
  const editDiffHtml = (file: TurnFile, editIndex: number) => {
    const e = file.edits[editIndex];
    if (!e) return '';
    return renderDiffHtml({ files: [{ oldPath: file.path, newPath: file.path, hunks: [editDiff(e.oldText, e.newText)] }] });
  };
  // The inline list renders one row per file, *plus* one row per edit for
  // any file with several (UNI-192) — so "3 files" can still be 13 rows
  // (UNI-297: a turn with 2 files but 10 edits between them rendered fully
  // expanded because `clipped()` only ever looked at `files.length`).
  // Count actual rendered rows instead.
  const rowCount = createMemo(() => props.files.reduce((n, f) => n + (f.edits.length > 1 ? 1 + f.edits.length : 1), 0));
  const clipped = () => rowCount() > 3;
  // In the clipped ~2.5-row list an inline diff would open where it can't
  // be seen or scrolled (UNI-183): tapping a file there opens the drawer
  // with that file expanded instead.
  const toggle = (key: string, inSheet: boolean) => {
    if (!inSheet && clipped()) {
      setExpanded(key);
      setSheetOpen(true);
      return;
    }
    setExpanded((cur) => (cur === key ? undefined : key));
  };
  const ActionButtons = (file: TurnFile, editIndex: number) => (
    <Show when={props.chat}>
      <button type="button" class="changes-edit-review" aria-label={`Review ${file.path}`} onClick={() => review(file, editIndex)}>
        <Eye size={14} />
      </button>
      <button type="button" class="changes-file-open" aria-label={`Open ${file.path} in editor`} onClick={() => openInEditor(file, editIndex)}>
        <ExternalLink size={14} />
      </button>
    </Show>
  );
  const EditRow = (file: TurnFile, editIndex: number, inSheet: boolean) => {
    const key = `${file.path}#${editIndex}`;
    const e = () => file.edits[editIndex]!;
    // The real file line (UNI-297), when known — never a line computed
    // from the edit's own oldText/newText snippet: that number is right
    // for the snippet but not for the file it points "Open in editor" at.
    const line = () => e().realLine;
    return (
      <div class="changes-edit" data-edit-key={key}>
        <div class="changes-edit-row">
          <button type="button" class="changes-edit-row-main" aria-expanded={expanded() === key} onClick={() => toggle(key, inSheet)}>
            <span class="changes-edit-label">
              edit {editIndex + 1}
              <Show when={line() !== undefined}> · L{line()}</Show>
            </span>
            <span class="tool-stat">
              <span class="add">+{e().add}</span> <span class="del">−{e().del}</span>
            </span>
          </button>
          {ActionButtons(file, editIndex)}
        </div>
        <Show when={expanded() === key && (inSheet || !clipped())}>
          <div class="tool-diff diffview-wrap" innerHTML={editDiffHtml(file, editIndex)} />
        </Show>
      </div>
    );
  };
  const FileRow = (file: TurnFile, inSheet = false) => (
    <div class="changes-file" data-path={file.path}>
      <Show
        when={file.edits.length > 1}
        fallback={
          <div class="changes-file-row">
            <button type="button" class="changes-file-row-main" aria-expanded={expanded() === file.path} onClick={() => toggle(file.path, inSheet)}>
              <span class="changes-file-path truncate">{file.path}</span>
              <span class="tool-stat">
                <span class="add">+{file.add}</span> <span class="del">−{file.del}</span>
              </span>
            </button>
            {ActionButtons(file, 0)}
          </div>
        }
      >
        <div class="changes-file-head">
          <span class="changes-file-path truncate">{file.path}</span>
          <span class="tool-stat">
            <span class="add">+{file.add}</span> <span class="del">−{file.del}</span>
          </span>
        </div>
        <For each={file.edits}>{(_e, i) => EditRow(file, i(), inSheet)}</For>
      </Show>
      <Show when={file.edits.length <= 1 && expanded() === file.path && (inSheet || !clipped())}>
        <div class="tool-diff diffview-wrap" innerHTML={editDiffHtml(file, 0)} />
      </Show>
    </div>
  );
  return (
    <div class="changes-card">
      <div class="changes-head">
        <span>
          Changes this turn · {props.files.length} {props.files.length === 1 ? 'file' : 'files'}
        </span>
        <span class="tool-stat">
          <span class="add">+{totals().add}</span> <span class="del">−{totals().del}</span>
        </span>
        <Show when={clipped()}>
          <button type="button" class="changes-show-all" data-testid="changes-show-all" onClick={() => setSheetOpen(true)}>
            Show all
          </button>
        </Show>
      </div>
      {/* Collapsed: ~2.5 rows, the half row hints there's more (UNI-177). */}
      <div class="changes-list" classList={{ clipped: clipped() }}>
        <For each={props.files}>{(file) => FileRow(file)}</For>
      </div>
      <BoardSheet
        open={sheetOpen()}
        onClose={() => setSheetOpen(false)}
        label={`Changes this turn · ${props.files.length} ${props.files.length === 1 ? 'file' : 'files'}`}
        testId="changes-sheet"
      >
        <div class="changes-sheet" ref={(el) => (sheetEl = el)}>
          <For each={props.files}>{(file) => FileRow(file, true)}</For>
        </div>
      </BoardSheet>
    </div>
  );
}

export function Compaction(props: { entry: Entry }) {
  const [open, setOpen] = createSignal(false);
  const e = () => props.entry as { summary?: string; tokensBefore?: number };
  return (
    <div class="compaction">
      <button type="button" class="compaction-head" onClick={() => setOpen(!open())}>
        <span class="compaction-line" />
        <span>Context compacted{e().tokensBefore ? ` · ${Math.round(e().tokensBefore! / 1000)}k tokens before` : ''}</span>
        <span class="compaction-line" />
      </button>
      <Show when={open() && e().summary}>
        <div class="compaction-body md">
          <For each={renderBlocks(e().summary!, false)}>{(html) => <MdBlock html={html} />}</For>
        </div>
      </Show>
    </div>
  );
}

export function RowView(props: { row: Row; cwd?: string; chat: Chat; onRemoveWaiting?: (key: string) => void }) {
  const r = () => props.row;
  return (
    <Switch>
      <Match when={r().kind === 'user' && (r() as Extract<Row, { kind: 'user' }>)}>
        {(u) => <UserBubble chat={props.chat} text={u().text} images={u().images} harness={u().harness} command={u().command} pending={u().pending} waiting={u().waiting} onRemoveWaiting={u().waiting ? () => props.onRemoveWaiting?.(u().key) : undefined} />}
      </Match>
      <Match when={r().kind === 'prose' && (r() as Extract<Row, { kind: 'prose' }>)}>{(p) => <Prose text={p().text} live={p().live} stopped={p().stopped} chat={props.chat} />}</Match>
      <Match when={r().kind === 'process' && (r() as Extract<Row, { kind: 'process' }>)}>
        {(p) => <ProcessGroup items={p().items} live={p().live} cwd={props.cwd} chat={props.chat} />}
      </Match>
      <Match when={r().kind === 'error' && (r() as Extract<Row, { kind: 'error' }>)}>
        {(e) => (
          <div class="chat-error" role="alert">
            <CircleAlert size={16} class="shrink-0" />
            <span>{e().text}</span>
          </div>
        )}
      </Match>
      <Match when={r().kind === 'card' && (r() as Extract<Row, { kind: 'card' }>)}>{(c) => <UnipiCard entry={c().entry} chat={props.chat} />}</Match>
      <Match when={r().kind === 'compaction' && (r() as Extract<Row, { kind: 'compaction' }>)}>{(c) => <Compaction entry={c().entry} />}</Match>
      <Match when={r().kind === 'turn_end' && (r() as Extract<Row, { kind: 'turn_end' }>)}>{(t) => <TurnEnd ms={t().ms} cost={t().cost} stopped={t().stopped} files={t().files} chat={props.chat} />}</Match>
    </Switch>
  );
}

export { textOf };
