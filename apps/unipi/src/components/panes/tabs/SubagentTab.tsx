/**
 * The `subagent` side-pane tab (WP-A10, Z-42): the child transcript of one
 * nested agent, read-only, in the unipi light row look. Opened from a
 * subagent step row or a completion card (one place, not two).
 *
 * Data: `_oino/agents` (`AgentInfo` tree: state, role, owner) plus the
 * backlog `_oino/agent_messages{agentId}`; while the child runs the tab
 * re-reads both (it is a live view — the extension's `agent_event`s are not
 * fanned out to tabs, so the tab follows the same throttle-on-seq rule as
 * the Session modal's Running tab). Stop is `_oino/agent_abort{agentId}`, the
 * same command the Running tab sends.
 */
import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { Bot, Square } from 'lucide-solid';
import { textOf } from '../../../lib/chat/model';
import { sessions, type SessionHandle } from '../../../lib/session/store';
import type { AgentMessage, Part } from '../../../lib/session/types';
import { setPaneComponent, type PaneTabProps } from '../../../lib/panes/kinds';
import { SubagentTimeline } from './SubagentTimeline';
import { tabBaseOf } from './sessionTabs';
import './tabs.css';

type AgentInfo = { id: string; label?: string; role?: string; owner?: string; state?: string; depth?: number };
type AgentsReply = { agents?: AgentInfo[]; follow?: boolean };
type MessagesReply = { messages?: AgentMessage[] };

/** How often a running child is re-read (the transcript is a live view). */
export const SUBAGENT_POLL_MS = 1_500;

const str = (v: unknown): string | undefined => (typeof v === 'string' && v ? v : undefined);
const obj = (v: unknown): Record<string, unknown> | undefined => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : undefined);

/** The agents of the session tree (`_oino/agents`). */
export async function fetchAgents(s: SessionHandle): Promise<AgentInfo[]> {
  const reply = await s.cmd<AgentsReply>({ type: '_oino/agents' }).catch(() => undefined);
  return Array.isArray(reply?.agents) ? reply!.agents!.filter((a) => a && typeof a.id === 'string') : [];
}

/** One child's transcript (`_oino/agent_messages`). */
export async function fetchAgentMessages(s: SessionHandle, agentId: string): Promise<AgentMessage[]> {
  const reply = await s.cmd<MessagesReply>({ type: '_oino/agent_messages', agentId }).catch(() => undefined);
  return Array.isArray(reply?.messages) ? reply!.messages!.filter((m) => m && typeof m.role === 'string') : [];
}

/**
 * A finished subagent run is gone from the host's agent tree, but its record id still names its
 * transcript in the subagents extension (`subagents.transcript{id}` → task / text / tool items,
 * read from the child session file). [running-tab]: the Running tab's "Open transcript" on a
 * finished run (and the timeline card's) opens by record id.
 */
export function messagesFromTranscript(reply: unknown): AgentMessage[] {
  const items = (reply as { items?: unknown } | null | undefined)?.items;
  if (!Array.isArray(items)) return [];
  const out: AgentMessage[] = [];
  items.forEach((raw, i) => {
    const it = obj(raw);
    if (!it) return;
    if (it.kind === 'task') out.push({ role: 'user', content: str(it.text) ?? '' } as AgentMessage);
    else if (it.kind === 'text') out.push({ role: 'assistant', content: [{ type: 'text', text: str(it.text) ?? '' }] } as AgentMessage);
    else if (it.kind === 'tool') {
      const id = `rec-${i}`;
      out.push({ role: 'assistant', content: [{ type: 'toolCall', id, name: str(it.name) ?? 'tool', arguments: { command: str(it.arg) ?? '' } }] } as AgentMessage);
      if (it.running !== true) out.push({ role: 'toolResult', toolCallId: id, content: str(it.output) ?? '', ...(it.isError === true ? { isError: true } : {}) } as AgentMessage);
    }
  });
  const report = str((reply as { report?: unknown }).report);
  if (report && !out.some((m) => m.role === 'assistant' && Array.isArray(m.content) && (m.content as Part[]).some((p) => p.type === 'text' && (p as { text?: unknown }).text === report))) {
    out.push({ role: 'assistant', content: [{ type: 'text', text: report }] } as AgentMessage);
  }
  return out;
}

/** A run's transcript by its record id (the subagents extension); empty when it does not know the id. */
export async function fetchRecordTranscript(s: SessionHandle, id: string): Promise<AgentMessage[]> {
  const reply = await s.cmd<unknown>({ type: '_oino/service_call', target: 'unipi.subagents', name: 'subagents.transcript', payload: { id } }).catch(() => undefined);
  return messagesFromTranscript(reply);
}

/**
 * The child transcript (shared by the desktop tab and the phone sheet): the main chat's own
 * timeline components over the child's messages, read only, bottom-locked.
 */
export function SubagentTranscript(props: { session: SessionHandle; agentId: string; cwd?: string; messages: readonly AgentMessage[]; running: boolean }) {
  return <SubagentTimeline session={props.session} agentId={props.agentId} cwd={props.cwd} messages={props.messages} running={props.running} />;
}

export function SubagentTab(props: PaneTabProps) {
  const base = () => tabBaseOf(props.tab.params, { hostId: props.hostId, sessionId: props.sessionId });
  const agentId = () => (typeof props.tab.params?.agentId === 'string' ? (props.tab.params.agentId as string) : '');
  const handle = () => (base().sessionId ? sessions().ensure(base().hostId, base().sessionId) : undefined);
  const [agents, setAgents] = createSignal<AgentInfo[]>([]);
  const [messages, setMessages] = createSignal<AgentMessage[]>([]);
  const [loaded, setLoaded] = createSignal(false);
  const [error, setError] = createSignal<string | undefined>();

  const read = async () => {
    const s = handle();
    const id = agentId();
    if (!s || !id) return;
    try {
      const [tree, msgs] = await Promise.all([fetchAgents(s), fetchAgentMessages(s, id)]);
      setAgents(tree);
      // Not (or no longer) in the host tree: a subagent record id — its transcript from the extension.
      setMessages(msgs.length === 0 && !tree.some((a) => a.id === id) ? await fetchRecordTranscript(s, id) : msgs);
      setError(undefined);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoaded(true);
    }
  };
  // A child that runs keeps writing: re-read on stream progress (throttled)
  // and on a slow tick, like the Running tab (Z-42 live transcript).
  let last = 0;
  const agent = createMemo(() => agents().find((a) => a.id === agentId()));
  const running = () => agent()?.state === 'running';
  createEffect(() => {
    // Re-read when the session stream advances (a child event bumps the seq).
    void handle()?.state.seq;
    if (!loaded()) return;
    const now = Date.now();
    if (now - last < SUBAGENT_POLL_MS / 2) return;
    last = now;
    void read();
  });
  // First read as soon as the stream is live (a tab may open while the
  // session is still connecting and `cmd` rejects until then).
  createEffect(() => {
    if (handle()?.state.conn !== 'live') return;
    if (loaded()) return;
    void read();
  });
  // perf-app: polls only while the run is running (no idle wake-ups once it settled).
  createEffect(() => {
    if (!running()) return;
    const timer = setInterval(() => void read(), SUBAGENT_POLL_MS);
    onCleanup(() => clearInterval(timer));
  });

  const name = () => agent()?.label ?? (typeof props.tab.params?.name === 'string' ? (props.tab.params.name as string) : 'Subagent');
  const stop = () => {
    const s = handle();
    if (!s) return;
    void s
      .cmd({ type: '_oino/agent_abort', agentId: agentId() })
      .then(() => read())
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  };

  return (
    <div class="stab" data-testid="subagent-tab" data-agent={agentId()}>
      <header class="stab-head">
        <Bot size={15} class="stab-icon" />
        <span class="stab-title truncate" title={name()}>{name()}</span>
        <Show when={agent()?.state}>
          <span class="stab-state" data-state={agent()!.state} data-testid="stab-state">
            {agent()!.state}
          </span>
        </Show>
        <span class="grow" />
        <Show when={running()}>
          <button type="button" class="stab-stop" onClick={stop} data-testid="stab-stop">
            <Square size={12} />
            <span>Stop</span>
          </button>
        </Show>
      </header>
      <Show when={agent()}>
        <p class="stab-meta">
          {[agent()!.role, agent()!.owner, `${(agent()!.depth ?? 0) || 1} deep`].filter(Boolean).join(' · ')}
        </p>
      </Show>
      <div class="stab-body">
        <Show when={error()}>
          <p class="stab-error">{error()}</p>
        </Show>
        <Show when={loaded()} fallback={<p class="stab-note">Loading the transcript…</p>}>
          <Show
            when={messages().length > 0}
            fallback={
              <p class="stab-note">
                {agent() ? 'This subagent has not written anything yet.' : 'This subagent is no longer in the session tree.'}
              </p>
            }
          >
            <Show when={handle()}>
              {(h) => <SubagentTranscript session={h()} agentId={agentId()} cwd={typeof props.tab.params?.cwd === 'string' ? (props.tab.params.cwd as string) : undefined} messages={messages()} running={running()} />}
            </Show>
          </Show>
        </Show>
      </div>
    </div>
  );
}

setPaneComponent('subagent', SubagentTab);
