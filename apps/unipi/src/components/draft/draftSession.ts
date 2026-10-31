/**
 * The new-chat draft's stand-in session (Z-19): the draft page renders the
 * SAME composer a chat does, and the composer talks to a `SessionHandle`. No
 * session exists yet — the first prompt creates it (a fileless session would
 * show in every PC's list, so nothing is created early) — so this handle
 * answers locally:
 *
 * - the toolbar reads the host's defaults (`host_cmd get_settings`: the model
 *   and thinking level a new session starts with) and, for the `/` menu and
 *   the model list, a live chat of the same PC when one is open;
 * - the permission / Plan controls send their usual `/permission …` /
 *   `/plan …` commands — kept here as the draft's choices and replayed on the
 *   new session before the first prompt (`startSession.ts`);
 * - a thinking pick or a model pick is kept the same way;
 * - any other send is the first prompt → `onSubmit`.
 */
import { batch, createSignal } from 'solid-js';
import { createStore, produce } from 'solid-js/store';
import { z } from 'zod';
import { initialState } from '../../lib/session/reducer';
import { sessions, type SendOptions, type SessionHandle } from '../../lib/session/store';
import type { CommandInfo, OutboxImage, OutboxItem, SessionState } from '../../lib/session/types';
import type { RpcCommand } from '../../lib/host/client';
import { THINKING_ORDER } from '../../lib/model/thinking';
import { WORKFLOW_STATUS_TOPIC } from '../../lib/status/plan';
import { hostCmd, listExtensions } from '../../settings/hostCmd';
import { parseModels, type ModelInfo } from '../../lib/model/picker';
import { cachedCommands, cachedModels, rememberModels } from './hostCache';

export type DraftModel = { provider: string; modelId: string; name?: string };

/** What the user chose on the draft, replayed on the new session. */
export type DraftChoices = {
  model?: DraftModel;
  thinking?: string;
  permission?: string;
  plan?: boolean;
};

export type DraftSession = SessionHandle & {
  readonly choices: () => DraftChoices;
  /** A live chat on the same PC (borrowed for the model catalogue), if one is open. */
  peer(): SessionHandle | undefined;
  pickModel(model: DraftModel): void;
};

/** The levels offered before a chat knows its model's (pi's common set). */
const DEFAULT_LEVELS = THINKING_ORDER.filter((level) => level !== 'minimal' && level !== 'max');

const SettingsSchema = z.object({ settings: z.record(z.string(), z.unknown()).optional() }).passthrough();

const str = (v: unknown): string | undefined => (typeof v === 'string' && v.trim() ? v : undefined);

/** The draft composer's key: its text, images and history live under it. */
export const draftKey = (hostId: string) => `draft:${hostId}`;

let refSeq = 0;

export function createDraftSession(hostId: string, onSubmit: (text: string, images: OutboxImage[]) => void): DraftSession {
  const [state, setState] = createStore<SessionState>({ ...initialState(hostId, ''), conn: 'live' });
  const [choices, setChoices] = createSignal<DraftChoices>({});
  const mutate = (fn: (s: SessionState) => void) => setState(produce(fn));

  /** A live chat on this PC: its commands and model list are this PC's too. */
  const peer = (): SessionHandle | undefined => {
    const reg = sessions();
    const keys = reg.keys().filter((k) => k.startsWith(`${hostId}/`)).reverse();
    for (const key of keys) {
      const handle = reg.peek(hostId, key.slice(hostId.length + 1));
      if (handle && handle.state.conn === 'live') return handle;
    }
    return undefined;
  };

  // The host's defaults: the model + thinking level a new session starts with.
  void hostCmd({ kind: 'get_settings' }, SettingsSchema, hostId)
    .then((res) => {
      if (!res.ok) return;
      const s = res.value.settings ?? {};
      const provider = str(s.defaultProvider);
      const id = str(s.defaultModel);
      const level = str(s.defaultThinkingLevel);
      batch(() => {
        if (id && !choices().model) setState('model', { provider, id, name: id });
        if (!choices().thinking) setState('thinkingLevel', level ?? 'medium');
      });
    })
    .catch(() => undefined);

  // The `/` menu: a live chat's commands, else the ones a chat on this PC
  // listed last (P0-09), else the workflow controls alone when this PC runs
  // the extension (the permission / Plan chips need them).
  const fromPeer = peer()?.state.commands;
  const remembered = cachedCommands(hostId);
  if (fromPeer?.length) setState('commands', [...fromPeer] as CommandInfo[]);
  else if (remembered?.length) setState('commands', remembered);
  else
    void listExtensions(hostId)
      .then((res) => {
        if (!res.ok) return;
        const workflow = res.value.find((e) => e.id === 'unipi.workflow' && e.enabled);
        if (!workflow || state.commands.length) return;
        setState('commands', [
          { name: 'permission', description: 'Permission mode: ask | auto | full', source: 'extension' },
          { name: 'plan', description: 'Plan mode: on | off', source: 'extension' },
        ]);
      })
      .catch(() => undefined);

  const setWorkflow = (patch: { permissionMode?: string; planMode?: boolean }) => {
    mutate((s) => {
      const prev = (s.topics[WORKFLOW_STATUS_TOPIC]?.data ?? { planMode: false }) as { planMode: boolean; permissionMode?: string };
      s.topics[WORKFLOW_STATUS_TOPIC] = { data: { ...prev, ...patch }, sticky: true, publisher: 'draft' };
    });
  };

  const fakeItem = (text: string, images?: OutboxImage[]): OutboxItem => ({
    ref: `draft-${++refSeq}`,
    text,
    ...(images?.length ? { images } : {}),
    mode: 'prompt',
    at: Date.now(),
    status: 'sending',
  });

  const handle: DraftSession = {
    key: draftKey(hostId),
    hostId,
    sessionId: () => '',
    state,
    turns: () => [],
    send(text: string, opts: SendOptions = {}) {
      const perm = /^\/permission\s+(ask|auto|full)\s*$/.exec(text.trim());
      if (perm) {
        setChoices((c) => ({ ...c, permission: perm[1] }));
        setWorkflow({ permissionMode: perm[1] });
        return fakeItem(text);
      }
      const plan = /^\/plan\s+(on|off)\s*$/.exec(text.trim());
      if (plan) {
        setChoices((c) => ({ ...c, plan: plan[1] === 'on' }));
        setWorkflow({ planMode: plan[1] === 'on' });
        return fakeItem(text);
      }
      const item = fakeItem(text, opts.images);
      onSubmit(text, opts.images ?? []);
      return item;
    },
    resend() {},
    dismiss() {},
    abort: async () => {},
    async cmd<T = unknown>(cmd: RpcCommand): Promise<T> {
      if (cmd.type === 'get_available_thinking_levels') {
        const p = peer();
        const same = p && p.state.model?.id === state.model?.id && p.state.model?.provider === state.model?.provider;
        if (same) {
          const reply = await p.cmd<unknown>(cmd).catch(() => undefined);
          if (reply) return reply as T;
        }
        return { levels: [...DEFAULT_LEVELS] } as T;
      }
      if (cmd.type === 'get_available_models') {
        const p = peer();
        const reply = p ? await p.cmd<unknown>(cmd).catch(() => undefined) : undefined;
        const models: ModelInfo[] = reply ? parseModels(reply) : [];
        if (models.length) {
          rememberModels(hostId, models);
          return { models } as T;
        }
        return { models: cachedModels(hostId) ?? [] } as T;
      }
      if (cmd.type === 'set_model' && typeof cmd.provider === 'string' && typeof cmd.modelId === 'string') {
        const known = (cachedModels(hostId) ?? []).find((m) => m.provider === cmd.provider && m.id === cmd.modelId);
        handle.pickModel({ provider: cmd.provider, modelId: cmd.modelId, ...(known?.name ? { name: known.name } : {}) });
        return null as T;
      }
      if (cmd.type === 'set_thinking_level' && typeof cmd.level === 'string') {
        setChoices((c) => ({ ...c, thinking: cmd.level as string }));
        setState('thinkingLevel', cmd.level);
        return null as T;
      }
      throw { code: 'E_UNSUPPORTED', message: 'Start the chat first', retryable: false };
    },
    subscribeTopics() {},
    getEntries: async () => [],
    retry() {},
    takeBack() {},
    acquire: () => () => {},
    close() {},
    choices,
    peer,
    pickModel(model) {
      setChoices((c) => ({ ...c, model }));
      setState('model', { provider: model.provider, id: model.modelId, name: model.name ?? model.modelId });
    },
  };
  return handle;
}
