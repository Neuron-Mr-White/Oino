/**
 * Settings commands (owner5 #5/#7/#12): one runner for the capability pages'
 * Manage rows and the form engine's `action` fields, so a Settings action
 * never navigates away or silently sends a slash command.
 *
 * - background → `runInBackground`, the reply shows inline under the row text;
 * - sheet → `CommandSheet` renders the command's dialog steps and replies;
 * - chat → opens the chat explicitly (the row is labelled "Opens in chat ↗").
 *
 * An extension that is off shows inline on the clicked row with "Turn on".
 */
import { For, Show, createMemo, createSignal, onCleanup, onMount } from 'solid-js';
import { ArrowUpRight, Check, CircleAlert, Loader2 } from 'lucide-solid';
import { BoardSheet } from '../../components/board/BoardSheet';
import { Markdown } from '../../components/board/Markdown';
import { DialogPanel } from '../../components/dock/DialogPanel';
import { SlotModal } from '../slots/SlotModal';
import { sessionUi } from '../registries';
import { dialogResponse } from '../dialogs/adapter';
import { looksMarkdown } from '../../lib/reports';
import { appNavigate } from '../../lib/nav';
import { chatHref, runManageActionAnywhere, type ManageAction } from '../../lib/settings/manage';
import {
  commandMode,
  commandOwner,
  runInBackground,
  startSettingsCommand,
  type Reply,
} from '../../lib/settings/run';
import { listExtensions, setExtensionEnabled } from '../../settings/hostCmd';
import type { SessionHandle } from '../../lib/session/store';
import { desktopShell } from '../../lib/layout';
import { settingsHostId } from '../../settings/target';

export type CommandSpec = {
  /** Row / sheet title. */
  label: string;
  /** The extension command (and its arguments), without the slash. */
  command: string;
  hidden?: boolean;
  /** Extension id that owns it (else derived from the command). */
  extension?: string;
};

export type CommandState =
  | { status: 'idle' }
  | { status: 'running' }
  | { status: 'done'; replies: Reply[] }
  | { status: 'quiet' }
  | { status: 'no-chat' }
  | { status: 'off'; extension?: string }
  | { status: 'turned-on' }
  | { status: 'opened' };

/** The extension's display name for the "is off" line. */
function extName(id: string | undefined): string {
  if (!id) return 'That extension';
  const tail = id.replace(/^unipi\./, '').replace(/[-_]/g, ' ');
  return tail === 'skill registry' ? 'Skills' : tail === 'mcp' ? 'MCP' : tail[0]!.toUpperCase() + tail.slice(1);
}

/** One Settings command: its run state, the sheet flag and the run entry point. */
export function createSettingsCommand(spec: () => CommandSpec, hostId: () => string = () => settingsHostId()) {
  const [state, setState] = createSignal<CommandState>({ status: 'idle' });
  const [sheet, setSheet] = createSignal(false);
  const mode = () => commandMode(spec().command);
  const owner = () => spec().extension ?? commandOwner(spec().command);
  let alive = true;
  onCleanup(() => (alive = false));
  const set = (next: CommandState) => {
    if (alive) setState(next);
  };

  const run = async () => {
    if (state().status === 'running') return;
    const m = mode();
    if (m === 'sheet') {
      set({ status: 'idle' });
      setSheet(true);
      return;
    }
    set({ status: 'running' });
    if (m === 'chat') {
      const action: ManageAction = {
        id: spec().command,
        page: 'mcp',
        label: spec().label,
        description: '',
        command: spec().command,
        title: spec().label,
        group: 'Agent',
        hidden: spec().hidden,
      };
      const outcome = await runManageActionAnywhere(action, { navigate: appNavigate, hostId: hostId() });
      set(outcome === 'sent' ? { status: 'opened' } : outcome === 'missing' ? { status: 'off', extension: owner() } : { status: 'no-chat' });
      return;
    }
    const outcome = await runInBackground(spec().command, {
      hostId: hostId(),
      hidden: spec().hidden,
      // The row shows each line as it comes ("Syncing…" → "Synced 12 servers").
      onProgress: (replies) => set({ status: 'done', replies }),
    });
    if (outcome.kind === 'done') set({ status: 'done', replies: outcome.replies });
    else if (outcome.kind === 'missing') set({ status: 'off', extension: owner() });
    else set({ status: outcome.kind });
  };

  const turnOn = async () => {
    const id = owner();
    if (!id) return;
    set({ status: 'running' });
    const result = await setExtensionEnabled(id, true, hostId());
    set(result.ok ? { status: 'turned-on' } : { status: 'off', extension: id });
  };

  return { state, setState: set, sheet, setSheet, mode, owner, run, turnOn };
}

export type SettingsCommand = ReturnType<typeof createSettingsCommand>;

/** The inline result under a row's text: progress, the reply, or "is off — Turn on". */
export function CommandResult(props: { cmd: SettingsCommand; testId?: string }) {
  const [expanded, setExpanded] = createSignal(false);
  const st = () => props.cmd.state();
  const replies = () => {
    const s = st();
    return s.status === 'done' ? s.replies : [];
  };
  const body = () => replies().map((r) => r.text).join('\n\n');
  /** The newest reply's headline (a progress line is replaced by its result). */
  const firstLine = () => (replies().at(-1)?.text ?? '').split('\n').map((l) => l.trim()).find(Boolean) ?? '';
  const long = () => body().includes('\n') || body().length > 120;
  const bad = () => replies().some((r) => r.kind === 'error');
  return (
    <Show when={st().status !== 'idle'}>
      <div class="settings-cmd-result" data-status={st().status} data-bad={bad() || st().status === 'off' ? '' : undefined} role="status" data-testid={props.testId}>
        <Show when={st().status === 'running'}>
          <span class="settings-cmd-line"><Loader2 size={13} class="spin" /> Running…</span>
        </Show>
        <Show when={st().status === 'done'}>
          <span class="settings-cmd-line">
            <Show when={!bad()} fallback={<CircleAlert size={13} />}>
              <Check size={13} />
            </Show>
            <span class="min-w-0 truncate">{firstLine() || 'Done'}</span>
            <Show when={long()}>
              <button type="button" class="settings-link" onClick={() => setExpanded((v) => !v)} aria-expanded={expanded()}>
                {expanded() ? 'Hide' : 'Show all'}
              </button>
            </Show>
          </span>
          <Show when={expanded()}>
            <ReplyBody text={body()} />
          </Show>
        </Show>
        <Show when={st().status === 'quiet'}>
          <span class="settings-cmd-line"><Check size={13} /> Started — nothing to report yet.</span>
        </Show>
        <Show when={st().status === 'no-chat'}>
          <span class="settings-cmd-line"><CircleAlert size={13} /> Couldn’t reach the agent on this PC — try again in a moment.</span>
        </Show>
        <Show when={st().status === 'opened'}>
          <span class="settings-cmd-line"><ArrowUpRight size={13} /> Opened in the chat.</span>
        </Show>
        <Show when={st().status === 'turned-on'}>
          <span class="settings-cmd-line"><Check size={13} /> Turned on — it starts with your next chat.</span>
        </Show>
        <Show when={st().status === 'off'}>
          <span class="settings-cmd-line">
            <CircleAlert size={13} />
            <span class="min-w-0">{extName((st() as { extension?: string }).extension)} is off.</span>
            <Show when={props.cmd.owner()}>
              <button type="button" class="btn-secondary settings-btn settings-cmd-turnon" onClick={() => void props.cmd.turnOn()} data-testid="cmd-turn-on">
                Turn on
              </button>
            </Show>
          </span>
        </Show>
      </div>
    </Show>
  );
}

function ReplyBody(props: { text: string }) {
  return (
    <Show when={looksMarkdown(props.text)} fallback={<pre class="settings-cmd-body">{props.text}</pre>}>
      <div class="settings-cmd-body report-md">
        <Markdown text={props.text} />
      </div>
    </Show>
  );
}

/**
 * The Settings sheet a `sheet` command runs in: the command's dialog steps
 * (select / confirm / input / ask_user…) render here, its replies below;
 * nothing navigates. "Open in chat ↗" is the explicit way out.
 */
export function CommandSheet(props: { cmd: SettingsCommand; spec: CommandSpec; hostId?: string }) {
  const hostId = () => props.hostId ?? settingsHostId();
  const [handle, setHandle] = createSignal<SessionHandle | undefined>();
  const [replies, setReplies] = createSignal<Reply[]>([]);
  const [phase, setPhase] = createSignal<'starting' | 'running' | 'no-chat' | 'off'>('starting');
  let release: (() => void) | undefined;
  let closed = false;
  onMount(() => {
    void startSettingsCommand(props.spec.command, {
      hostId: hostId(),
      hidden: props.spec.hidden,
      onReply: (reply) => setReplies((list) => [...list, reply]),
    }).then((started) => {
      if (!started.ok) {
        setPhase(started.reason === 'missing' ? 'off' : 'no-chat');
        return;
      }
      if (closed) {
        started.release();
        return;
      }
      release = started.release;
      setHandle(started.handle);
      setPhase('running');
    });
  });
  const close = () => {
    closed = true;
    release?.();
    release = undefined;
    props.cmd.setSheet(false);
  };
  onCleanup(() => {
    closed = true;
    release?.();
  });
  const registry = createMemo(() => {
    const h = handle();
    return h ? sessionUi(h.hostId, h.sessionId()) : undefined;
  });
  const dialogs = () => registry()?.dialogs() ?? [];
  const overlays = () => registry()?.stack('overlay')() ?? [];
  const answer = (id: string, value: unknown) => {
    const h = handle();
    if (!h) return;
    registry()?.dismissDialog(id);
    void h.cmd(dialogResponse(id, value)).catch((error: unknown) => console.warn('[settings] answer failed', id, error));
  };
  const waiting = () => phase() === 'running' && dialogs().length === 0 && replies().length === 0;
  const openInChat = () => {
    const h = handle();
    close();
    if (h) appNavigate(chatHref(h.hostId, h.sessionId()));
  };
  return (
    <BoardSheet open onClose={close} label={props.spec.label} class="settings-cmd-sheet" testId="settings-cmd-sheet"
      footer={
        <div class="settings-cmd-foot">
          <Show when={handle()}>
            <button type="button" class="settings-link" onClick={openInChat} data-testid="settings-cmd-open-chat">
              Open in chat <ArrowUpRight size={13} />
            </button>
          </Show>
          <span class="flex-1" />
          <button type="button" class="btn-secondary settings-btn" onClick={close} data-testid="settings-cmd-done">
            {dialogs().length ? 'Cancel' : 'Done'}
          </button>
        </div>
      }
    >
      <div class="settings-cmd-sheet-body">
        <Show when={phase() === 'starting' || waiting()}>
          <p class="settings-cmd-line settings-cmd-wait"><Loader2 size={14} class="spin" /> {phase() === 'starting' ? 'Starting…' : `Waiting for ${props.spec.label}…`}</p>
        </Show>
        <Show when={phase() === 'no-chat'}>
          <p class="settings-cmd-line" data-bad=""><CircleAlert size={14} /> Couldn’t reach the agent on this PC — try again in a moment.</p>
        </Show>
        <Show when={phase() === 'off'}>
          <p class="settings-cmd-line" data-bad="">
            <CircleAlert size={14} /> {extName(props.cmd.owner())} is off.
            <Show when={props.cmd.owner()}>
              <button type="button" class="btn-secondary settings-btn" onClick={() => void props.cmd.turnOn().then(close)}>Turn on</button>
            </Show>
          </p>
        </Show>
        <Show when={dialogs().length > 0 && handle()}>
          <div class="settings-cmd-dialogs">
            <DialogPanel dialogs={dialogs()} onAnswer={answer} phone={!desktopShell()} hostId={handle()!.hostId} sessionId={handle()!.sessionId()} />
          </div>
        </Show>
        <For each={replies()}>{(reply) => <ReplyBody text={reply.text} />}</For>
      </div>
      <Show when={handle() && overlays().length > 0}>
        <SlotModal stack={overlays()} target={{ scope: 'session', hostId: handle()!.hostId, sessionId: handle()!.sessionId() }} phone={!desktopShell()} />
      </Show>
    </BoardSheet>
  );
}

/** Whether an extension is off on this host (`undefined` while unknown). */
export function createExtensionStates(hostId: () => string = () => settingsHostId()) {
  const [states, setStates] = createSignal<Record<string, boolean> | undefined>();
  const refresh = async () => {
    const result = await listExtensions(hostId());
    if (result.ok) setStates(Object.fromEntries(result.value.map((e) => [e.id, e.enabled])));
  };
  void refresh();
  return { enabled: (id: string | undefined) => (id ? states()?.[id] : undefined), refresh };
}

