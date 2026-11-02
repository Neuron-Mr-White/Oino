/**
 * Draft view (Z-19, WP-A5): what `/new/:host?cwd=` shows — a calm start
 * screen: the time-of-day greeting, the SAME composer a chat has (attach,
 * permission, Plan, thinking, model, Send) centred under it with a workspace
 * row inside the card (folder picker, and a PC picker when more than one PC
 * is paired), and a faint Unicrab watermark behind.
 *
 * The composer talks to a local stand-in session (`./draftSession`); the
 * first prompt creates the real session in the chosen folder — "No
 * workspace" sends no folder and the host runs the chat in its own private
 * scratch folder (never the user's home) — replays the draft's
 * choices on it, sends the prompt (`./startSession`) and routes to the chat.
 */
import { createEffect, createMemo, createResource, createSignal, on, Show, untrack } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { useHostsOptional } from '../../hosts/context';
import { EmptyChat } from '../EmptyChat';
import { Unicrab } from '../Unicrab';
import { Composer } from '../composer/Composer';
import { fsRoots } from '../../lib/fs';
import { folderChoices } from '../../lib/session/newSession';
import { hubs } from '../../lib/session/hub';
import { clearComposer, restoreComposer } from '../../lib/composer/drafts';
import { LOCAL_HOST } from '../../lib/host/client';
import { layout } from '../../lib/layout';
import { sessionHref } from '../shell/Sidebar';
import { createDraftSession, draftKey } from './draftSession';
import { DraftModelChip } from './DraftModelChip';
import { startSession } from './startSession';
import { ConnectProviderCard } from './ConnectProviderCard';
import { HostPicker, WorkspacePicker, homeOf } from './WorkspacePicker';
import type { OutboxImage } from '../../lib/session/types';
import './draft.css';

/**
 * E-11: a draft restored after a failed start belongs to the workspace it was typed for
 * (draft key → that workspace). Entering a new chat in another workspace starts empty.
 */
const failedDraftCwd = new Map<string, string>();
function dropForeignFailedDraft(key: string, cwd: string | undefined): void {
  if (!failedDraftCwd.has(key)) return;
  if (failedDraftCwd.get(key) === (cwd ?? '')) return;
  failedDraftCwd.delete(key);
  clearComposer(key);
}

export function DraftView(props: { hostId: string; cwd?: string }) {
  const navigate = useNavigate();
  const hosts = useHostsOptional();
  /** `/new/local` (the first-run route) names This computer by alias: use its real id. */
  const hostId = createMemo(() => {
    const list = hosts?.hosts() ?? [];
    if (list.some((h) => h.id === props.hostId)) return props.hostId;
    if (props.hostId === LOCAL_HOST) return list.find((h) => h.transport === 'local')?.id ?? props.hostId;
    return props.hostId;
  });
  dropForeignFailedDraft(draftKey(hostId()), props.cwd);
  createEffect(on(() => [hostId(), props.cwd] as const, ([h, c]) => dropForeignFailedDraft(draftKey(h), c), { defer: true }));
  const [folder, setFolder] = createSignal<string | undefined>(props.cwd);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<{ text: string; hint?: string } | undefined>();

  // Untracked: `ensure` may open the hub (a registry write) — the memo follows the host only.
  const hub = createMemo(on(hostId, (id) => untrack(() => hubs().ensure(id))));
  const [roots] = createResource(hostId, async (id) => {
    const res = await fsRoots(id);
    return res.ok ? res.value.roots : [];
  });
  const home = createMemo(() => homeOf(roots.latest ?? []));
  /** This PC's recent session folders (newest first). */
  const recent = createMemo(() => {
    const cards = hub()?.state.loaded ? Object.values(hub()!.state.cards) : [];
    const sorted = [...cards].sort((a, b) => (b.updated_at ?? 0) - (a.updated_at ?? 0));
    return folderChoices(sorted, [], undefined);
  });
  const hostName = () => {
    const h = hosts?.hosts().find((x) => x.id === hostId());
    return h && h.transport !== 'local' ? h.name : undefined;
  };
  const multiHost = () => (hosts?.hosts().length ?? 0) > 1;

  const pickHost = (id: string) => {
    if (id === hostId()) return;
    navigate(`/new/${encodeURIComponent(id)}${folder() ? `?cwd=${encodeURIComponent(folder()!)}` : ''}`);
  };

  const submit = async (text: string, images: OutboxImage[], choices: ReturnType<ReturnType<typeof createDraftSession>['choices']>) => {
    const id = hostId();
    setError(undefined);
    // E-06: nothing here can run a model (or the picked model's key was rejected) — do not start a chat that fails on its first turn.
    const ready = draft()?.readiness();
    const model = draft()?.state.model;
    const deadKey = ready?.known && ready.any && model?.provider ? ready.providers.find((p) => p.id === model.provider && p.ready === false) : undefined;
    if ((ready?.known && !ready.any) || deadKey) {
      restoreComposer(draftKey(id), { text, images });
      failedDraftCwd.set(draftKey(id), props.cwd ?? '');
      setError(
        deadKey
          ? { text: `${deadKey.name || deadKey.id} isn't working${deadKey.reason ? ` (${deadKey.reason})` : ''}.`, hint: 'Pick another model, or fix the key in Settings › Providers.' }
          : { text: 'No model provider is connected yet.', hint: 'Connect one in Settings › Providers, then send again.' },
      );
      return;
    }
    setBusy(true);
    // No workspace = no cwd: the host makes the chat's private scratch folder (never `$HOME`).
    const cwd = folder();
    const res = await startSession(id, { cwd, text, images, choices });
    if (!res.ok) {
      setBusy(false);
      setError({ text: String(res.error.message), hint: res.error.hint });
      // The message comes back: nothing typed is lost to a failed start.
      restoreComposer(draftKey(id), { text, images });
      failedDraftCwd.set(draftKey(id), props.cwd ?? '');
      return;
    }
    failedDraftCwd.delete(draftKey(id));
    // PH-17: the draft becomes the chat — Back from it goes to Chats, not the stale draft.
    navigate(sessionHref(id, res.sessionId), { replace: true });
  };

  // One stand-in per PC, built untracked: it peeks the session registry (a
  // chat closing as the route changes must not rebuild the composer mid-dispose).
  const draft = createMemo(
    on(hostId, (id) =>
      untrack(() => {
        const session = createDraftSession(id, (text, images) => void submit(text, images, session.choices()));
        return session;
      }),
    ),
  );

  /** F-01: why Send is held — the typed text is never sent nor cleared meanwhile. */
  const sendHold = (): string | undefined => {
    const ready = draft()?.readiness();
    if (ready?.known && !ready.any) return 'Connect a provider first — your message stays here';
    const model = draft()?.state.model;
    const dead = ready?.known && model?.provider ? ready.providers.find((p) => p.id === model.provider && p.ready === false) : undefined;
    return dead ? `${dead.name || dead.id} isn't working — pick another model` : undefined;
  };

  return (
    <div class="draft" data-testid="draft-view">
      <div class="draft-mark" aria-hidden="true">
        <Unicrab state="idle" size={560} class="draft-mark-crab" />
      </div>
      <div class="draft-center">
        <EmptyChat cwd={folder()} hostName={hostName()} variant="start" />
        <Show when={draft()?.readiness()?.known && !draft()?.readiness()?.any}>
          <ConnectProviderCard />
        </Show>
        <div class="draft-composer" data-testid="draft-composer" data-pending={draft()?.ready() ? undefined : ''}>
          <Show when={draft()} keyed>
            {(session) => (
              <Composer
                session={session}
                draft
                sendHold={sendHold}
                busy={busy()}
                autofocus={layout().desktop()}
                phone={!layout().desktop()}
                cwd={() => folder()}
                placeholder={busy() ? 'Starting the chat…' : undefined}
                modelSlot={<DraftModelChip session={session} disabled={busy()} />}
                header={
                  <>
                    <WorkspacePicker
                      hostId={hostId()}
                      value={folder()}
                      onChange={setFolder}
                      recent={recent()}
                      roots={roots.latest ?? []}
                      home={home()}
                      disabled={busy()}
                    />
                    <Show when={multiHost()}>
                      <HostPicker hostId={hostId()} hosts={hosts?.hosts() ?? []} onPick={pickHost} disabled={busy()} />
                    </Show>
                  </>
                }
              />
            )}
          </Show>
        </div>
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
    </div>
  );
}
