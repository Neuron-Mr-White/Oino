/**
 * Chat transport (UNI-113) — the one seam between the Chat UI and a PC's
 * pi sessions (docs/m5/PROTOCOL.md).
 *
 * - Tauri: `chat_list` / `chat_open` (Channel) / `chat_send` / `chat_close`
 *   → iroh → unipi-host → `~/.unipi/bridge/<pid>.sock`.
 * - Browser dev / tests: `setChatTransport()` injects a fake (the capture
 *   harness replays a recorded session through it).
 */
import { Channel, invoke } from '@tauri-apps/api/core';
import { isTauri } from '../platform';

export type AgentStatus = 'idle' | 'working' | 'blocked' | 'done' | 'unknown';

export type ChatPane = {
  id: string;
  focused: boolean;
  title?: string;
  terminal_title?: string;
  cwd?: string;
  agent?: { name: string; status: AgentStatus; session?: { kind: string; value: string } };
  chat?: { pid: number; session_name?: string; cwd?: string; waiting?: { kind: string; title?: string; since: number }; running?: boolean; pending_work?: string; lh_mode?: string; lh_paused?: string };
};
export type ChatTab = { id: string; label: string; number: number; focused: boolean; agent_status: AgentStatus; panes: ChatPane[] };
export type ChatWorkspace = {
  id: string;
  label: string;
  number: number;
  focused: boolean;
  agent_status: AgentStatus;
  tab_count: number;
  pane_count: number;
  tabs: ChatTab[];
};
export type ChatHerdrSession = { name: string; running: boolean; default: boolean; workspaces: ChatWorkspace[]; error?: string };
/** UNI-260: tmux panes in chat_list (hosts ≥ proto 1.12.0). */
export type ChatTmuxPane = { id: string; index: number; active: boolean; cwd: string; pid: number; title: string; current_command?: string; agent?: ChatPane['agent']; chat?: ChatPane['chat'] };
export type ChatTmuxSession = { id: string; name: string; windows: Array<{ id: string; index: number; name: string; active: boolean; panes: ChatTmuxPane[] }> };
export type ChatListData = {
  herdr: { available: boolean; reason?: string; sessions?: ChatHerdrSession[] };
  tmux?: { available: boolean; reason?: string; sessions?: ChatTmuxSession[] };
  protected?: string[];
};

export type ChatStreamState = 'open' | 'closed' | 'error';

export interface ChatConnection {
  send(line: string): Promise<void>;
  close(): void;
}

export interface ChatTransport {
  list(hostId: string): Promise<ChatListData>;
  open(
    hostId: string,
    pid: number,
    onLine: (line: string) => void,
    onState: (state: ChatStreamState, message?: string, code?: string) => void,
  ): ChatConnection;
}

type StreamMsg =
  | { kind: 'open' }
  | { kind: 'line'; line: string }
  | { kind: 'closed'; message?: string }
  | { kind: 'error'; message: string; code?: string };

const errorText = (error: unknown) => String((error as { message?: string })?.message ?? error);

export const tauriChatTransport: ChatTransport = {
  async list(hostId) {
    return (await invoke('chat_list', { hostId })) as ChatListData;
  },
  open(hostId, pid, onLine, onState) {
    const channel = new Channel<StreamMsg>();
    let streamId: string | undefined;
    let closed = false;
    const queue: string[] = [];
    channel.onmessage = (message) => {
      if (message.kind === 'line') onLine(message.line);
      else if (message.kind === 'open') onState('open');
      else onState(message.kind, message.message, message.kind === 'error' ? message.code : undefined);
    };
    const ready = invoke('chat_open', { hostId, pid, onEvent: channel })
      .then((id) => {
        streamId = String(id);
        if (closed) void invoke('chat_close', { streamId }).catch(() => {});
        for (const line of queue.splice(0)) void invoke('chat_send', { streamId, line }).catch(() => {});
        return streamId;
      })
      .catch((error) => {
        onState('error', errorText(error), (error as { code?: string })?.code);
        return undefined;
      });
    return {
      async send(line) {
        if (!streamId) {
          queue.push(line);
          await ready;
          return;
        }
        await invoke('chat_send', { streamId, line });
      },
      close() {
        closed = true;
        if (streamId) void invoke('chat_close', { streamId }).catch(() => {});
      },
    };
  },
};

/** No PC (browser without an injected fake). */
const noTransport: ChatTransport = {
  async list() {
    return { herdr: { available: false, reason: 'Chat needs the app (no PC in the browser).' } };
  },
  open(_h, _p, _l, onState) {
    queueMicrotask(() => onState('error', 'Chat needs the app.'));
    return { send: async () => {}, close: () => {} };
  },
};

let override: ChatTransport | undefined;
export function setChatTransport(transport: ChatTransport | undefined): void {
  override = transport;
}
export function chatTransport(): ChatTransport {
  if (override) return override;
  const injected = typeof window !== 'undefined' ? (window as { __UNIPI_CHAT__?: ChatTransport }).__UNIPI_CHAT__ : undefined;
  if (injected) return injected;
  return isTauri() ? tauriChatTransport : noTransport;
}
