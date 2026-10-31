/**
 * Draft view (Z-19, WP-A5): what `/new/:host?cwd=` shows — the empty-chat hero,
 * the folder + host pickers (`fs_roots` and the recent session cwds) and the
 * composer. The first prompt creates the session (`session_new`), the app
 * routes to `/s/:host/:id` and the prompt goes out once its stream is live
 * (`lib/session/firstPrompt.ts`). Replaces v1's `NewChatSheet` (its herdr/tmux
 * "Where" step went with herdr, D-14).
 */
import { createMemo, createResource, createSignal, For, Show } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { ArrowUp, Folder, Monitor } from 'lucide-solid';
import { useHostsOptional } from '../../hosts/context';
import { EmptyChat } from '../EmptyChat';
import { fsRoots } from '../../lib/fs';
import { folderChoices, newSession, shortPath } from '../../lib/session/newSession';
import { hubs } from '../../lib/session/hub';
import { queueFirstPrompt } from '../../lib/session/firstPrompt';
import { sessionHref } from '../shell/Sidebar';
import './draft.css';

export function DraftView(props: { hostId: string; cwd?: string }) {
  const navigate = useNavigate();
  const hosts = useHostsOptional();
  const [folder, setFolder] = createSignal<string | undefined>(props.cwd);
  const [text, setText] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<{ text: string; hint?: string } | undefined>();

  const hub = createMemo(() => hubs().peek(props.hostId));
  /** Recent cwds first (this host's sessions), then the host's `fs_roots`. */
  const [roots] = createResource(
    () => props.hostId,
    async (hostId) => {
      const res = await fsRoots(hostId);
      return res.ok ? res.value.roots : [];
    },
  );
  const folders = createMemo(() => {
    const cards = hub()?.state.loaded ? Object.values(hub()!.state.cards) : [];
    const live = folderChoices(cards, roots.latest ?? [], undefined);
    return [...new Set([folder(), ...live].filter((f): f is string => !!f))];
  });
  const hostName = () => {
    const h = hosts?.hosts().find((x) => x.id === props.hostId);
    return h && h.transport !== 'local' ? h.name : undefined;
  };
  const canSend = () => text().trim().length > 0 && !busy();
  const placeHint = () => {
    const f = folder();
    return f ? ` in ${shortPath(f)}` : '';
  };

  const pickHost = (id: string) => {
    if (id === props.hostId) return;
    navigate(`/new/${encodeURIComponent(id)}${folder() ? `?cwd=${encodeURIComponent(folder()!)}` : ''}`);
  };

  const send = async () => {
    if (!canSend()) return;
    const prompt = text().trim();
    const where = folder();
    setBusy(true);
    setError(undefined);
    const res = await newSession(props.hostId, where);
    if (!res.ok) {
      setBusy(false);
      setError({ text: res.error.message, hint: res.error.hint });
      return;
    }
    const id = res.value.session;
    queueFirstPrompt(props.hostId, id, { text: prompt });
    navigate(sessionHref(props.hostId, id));
  };

  return (
    <div class="draft" data-testid="draft-view">
      <div class="draft-hero">
        <EmptyChat cwd={folder()} hostName={hostName()} />
      </div>
      <div class="draft-pickers" data-testid="draft-pickers">
        <Show when={(hosts?.hosts().length ?? 0) > 1}>
          <div class="draft-chips" role="group" aria-label="Host" data-testid="draft-hosts">
            <For each={hosts?.hosts() ?? []}>
              {(h) => (
                <button
                  type="button"
                  class="draft-chip"
                  data-on={h.id === props.hostId ? '' : undefined}
                  aria-pressed={h.id === props.hostId}
                  onClick={() => pickHost(h.id)}
                >
                  <Monitor size={12} />
                  {h.transport === 'local' ? 'This computer' : h.name}
                </button>
              )}
            </For>
          </div>
        </Show>
        <Show when={folders().length > 0}>
          <div class="draft-chips" role="group" aria-label="Folder" data-testid="draft-folders">
            <For each={folders()}>
              {(f) => (
                <button
                  type="button"
                  class="draft-chip"
                  title={f}
                  data-on={f === folder() ? '' : undefined}
                  aria-pressed={f === folder()}
                  onClick={() => setFolder(f === folder() ? undefined : f)}
                >
                  <Folder size={12} />
                  {shortPath(f)}
                </button>
              )}
            </For>
          </div>
        </Show>
      </div>
      <form
        class="draft-form"
        data-testid="draft-form"
        onSubmit={(e) => {
          e.preventDefault();
          void send();
        }}
      >
        <textarea
          class="draft-input"
          rows={2}
          placeholder={`Message UniPi${placeHint()}…`}
          aria-label="First message"
          data-testid="draft-input"
          value={text()}
          onInput={(e) => setText(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey) {
              e.preventDefault();
              void send();
            }
          }}
        />
        <button type="submit" class="draft-send" disabled={!canSend()} aria-label="Send" data-testid="draft-send">
          <ArrowUp size={16} />
        </button>
      </form>
      <Show when={error()}>
        {(e) => (
          <p class="draft-error" role="alert" data-testid="draft-error">
            {e().text}
            <Show when={e().hint}>
              <span class="draft-error-hint">{e().hint}</span>
            </Show>
          </p>
        )}
      </Show>
    </div>
  );
}
