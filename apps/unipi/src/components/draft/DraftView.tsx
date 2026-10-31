/**
 * Draft view (Z-19, WP-A5): what `/new/:host?cwd=` shows — a calm start
 * screen: the time-of-day greeting, the SAME composer a chat has (attach,
 * permission, Plan, thinking, model, Send) centred under it with a workspace
 * row inside the card (folder picker, and a PC picker when more than one PC
 * is paired), and a faint Unicrab watermark behind.
 *
 * The composer talks to a local stand-in session (`./draftSession`); the
 * first prompt creates the real session in the chosen folder — "No
 * workspace" is the user's home folder on that PC — replays the draft's
 * choices on it, sends the prompt (`./startSession`) and routes to the chat.
 */
import { createMemo, createResource, createSignal, on, Show, untrack } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { useHostsOptional } from '../../hosts/context';
import { EmptyChat } from '../EmptyChat';
import { Unicrab } from '../Unicrab';
import { Composer } from '../composer/Composer';
import { fsRoots } from '../../lib/fs';
import { folderChoices } from '../../lib/session/newSession';
import { hubs } from '../../lib/session/hub';
import { restoreComposer } from '../../lib/composer/drafts';
import { LOCAL_HOST } from '../../lib/host/client';
import { layout } from '../../lib/layout';
import { sessionHref } from '../shell/Sidebar';
import { createDraftSession, draftKey } from './draftSession';
import { DraftModelChip } from './DraftModelChip';
import { startSession } from './startSession';
import { HostPicker, WorkspacePicker, homeOf } from './WorkspacePicker';
import type { OutboxImage } from '../../lib/session/types';
import './draft.css';

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
    setBusy(true);
    setError(undefined);
    // No workspace = the home folder on that PC (asked again if the roots are still on their way).
    let cwd = folder() ?? home();
    if (!cwd) {
      const r = await fsRoots(id);
      cwd = r.ok ? homeOf(r.value.roots) : undefined;
    }
    const res = await startSession(id, { cwd, text, images, choices });
    if (!res.ok) {
      setBusy(false);
      setError({ text: String(res.error.message), hint: res.error.hint });
      // The message comes back: nothing typed is lost to a failed start.
      restoreComposer(draftKey(id), { text, images });
      return;
    }
    navigate(sessionHref(id, res.sessionId));
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

  return (
    <div class="draft" data-testid="draft-view">
      <div class="draft-mark" aria-hidden="true">
        <Unicrab state="idle" size={560} class="draft-mark-crab" />
      </div>
      <div class="draft-center">
        <EmptyChat cwd={folder()} hostName={hostName()} variant="start" />
        <div class="draft-composer" data-testid="draft-composer">
          <Show when={draft()} keyed>
            {(session) => (
              <Composer
                session={session}
                draft
                busy={busy()}
                autofocus
                phone={!layout().desktop()}
                cwd={() => folder() ?? home()}
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
