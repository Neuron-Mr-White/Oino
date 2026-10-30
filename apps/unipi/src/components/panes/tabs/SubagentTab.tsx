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
import { renderBlocks } from '../../../lib/chat/markdown';
import { textOf } from '../../../lib/chat/model';
import { sessions, type SessionHandle } from '../../../lib/session/store';
import type { AgentMessage, Part } from '../../../lib/session/types';
import { setPaneComponent, type PaneTabProps } from '../../../lib/panes/kinds';
import { MdBlock } from '../../chat/MdBlock';
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

/** The text of a message's content (a string, or its text parts joined). */
const messageText = (m: AgentMessage): string => textOf(m.content as string | Part[] | undefined);

/** The tool result of a child's call, matched by `toolCallId`. */
function resultOf(messages: readonly AgentMessage[], callId: string): { text: string; isError?: boolean } | undefined {
  for (const m of messages) {
    if (m.role !== 'toolResult' || m.toolCallId !== callId) continue;
    return { text: messageText(m), ...(m.isError ? { isError: true } : {}) };
  }
  return undefined;
}

const callArgs = (part: Part): string => {
  const args = obj(part.arguments) ?? obj(part.args) ?? {};
  const pick = str(args.path) ?? str(args.command) ?? str(args.pattern) ?? str(args.query) ?? str(args.task) ?? str(args.prompt);
  return pick ?? '';
};

const tail = (text: string, n: number): string => {
  const t = text.trim();
  return t.length > n ? `…${t.slice(-n)}` : t;
};

/** The child transcript column (shared by the desktop tab and the phone sheet). */
export function SubagentTranscript(props: { messages: readonly AgentMessage[] }) {
  return (
    <div class="stab-msgs">
      <For each={props.messages}>
        {(m) => {
          if (m.role === 'toolResult') return null; // rendered inside its call
          if (m.role === 'user') {
            return (
              <div class="stab-msg" data-role="user" data-testid="stab-user">
                <span class="stab-role">Task</span>
                <p class="stab-text">{messageText(m)}</p>
              </div>
            );
          }
          if (m.role === 'bashExecution') {
            return <pre class="stab-out" data-testid="stab-bash">{messageText(m)}</pre>;
          }
          if (m.role !== 'assistant') {
            const label = str(obj(m.details)?.title) ?? str(m.customType) ?? m.role;
            return (
              <div class="stab-msg" data-role="note">
                <span class="stab-role">{label}</span>
              </div>
            );
          }
          const parts = Array.isArray(m.content) ? (m.content as Part[]) : [];
          return (
            <div class="stab-msg" data-role="assistant" data-testid="stab-assistant">
              <For each={parts}>
                {(p) => {
                  if (p.type === 'thinking') {
                    const t = String((p as { thinking?: unknown }).thinking ?? '');
                    return (
                      <details class="stab-think">
                        <summary>Thought</summary>
                        <p class="stab-think-text">{t.trim()}</p>
                      </details>
                    );
                  }
                  if (p.type === 'toolCall') {
                    const id = str(p.id) ?? '';
                    const name = str(p.name) ?? 'tool';
                    const res = resultOf(props.messages, id);
                    return (
                      <div class="stab-tool" data-state={res?.isError ? 'error' : res ? 'ok' : 'running'} data-testid="stab-tool">
                        <span class="stab-tool-name">{name}</span>
                        <span class="stab-tool-subject truncate">{callArgs(p)}</span>
                        <Show when={res}>
                          <pre class="stab-out" data-error={res!.isError ? '' : undefined}>{tail(res!.text, 1200)}</pre>
                        </Show>
                      </div>
                    );
                  }
                  if (p.type === 'text') {
                    const text = String((p as { text?: unknown }).text ?? '');
                    if (!text.trim()) return null;
                    return (
                      <div class="stab-prose md">
                        <For each={renderBlocks(text, false)}>{(html) => <MdBlock html={html} />}</For>
                      </div>
                    );
                  }
                  return null;
                }}
              </For>
            </div>
          );
        }}
      </For>
    </div>
  );
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
      setMessages(msgs);
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
  onMount(() => {
    const timer = setInterval(() => {
      if (running()) void read();
    }, SUBAGENT_POLL_MS);
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
            <SubagentTranscript messages={messages()} />
          </Show>
        </Show>
      </div>
    </div>
  );
}

setPaneComponent('subagent', SubagentTab);
