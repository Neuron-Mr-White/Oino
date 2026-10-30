/**
 * "New chat" (UNI-260; WP-A1 placeholder until the new-chat draft, WP-A5 /
 * Z-19): Chat list + (phone + desktop) and the desktop palette. One sheet:
 * PC → folder (chips from the PC's session folders / Files roots, or
 * browse) → optional first message. Start asks the host (`session_new`);
 * the chat then opens and the first message goes out once it's connected.
 * The herdr/tmux "Where" step is gone with herdr (D-14).
 */
import { createEffect, createMemo, createSignal, For, on, Show, untrack } from 'solid-js';
import { Check, ChevronLeft, Folder, FolderOpen, Monitor } from 'lucide-solid';
import type { AppError } from '@unipi/protocol';
import { BoardSheet } from '../board/BoardSheet';
import { fsList, fsRoots } from '../../lib/fs';
import { chatLists, type ChatTarget } from '../../lib/session/list';
import { errorWords, folderChoices, newSession, shortPath } from '../../lib/session/newSession';

export type NewChatPc = { id: string; name: string };
export type NewChatStarted = { hostId: string; pcName: string; target: ChatTarget; first: string };

type Load = { roots: string[]; error?: AppError };

export function NewChatSheet(props: {
  open: boolean;
  onClose: () => void;
  pcs: NewChatPc[];
  hostId?: string;
  onStarted: (started: NewChatStarted) => void;
}) {
  const [hostId, setHostId] = createSignal<string | undefined>();
  const [step, setStep] = createSignal<'form' | 'pc' | 'browse'>('form');
  const [load, setLoad] = createSignal<Load | undefined>();
  const [loading, setLoading] = createSignal(false);
  const [cwd, setCwd] = createSignal<string | undefined>();
  const [first, setFirst] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<{ text: string; hint?: string } | undefined>();
  const pc = () => props.pcs.find((p) => p.id === hostId());
  const home = () => load()?.roots[0];
  const sessions = () => (hostId() ? (chatLists().get(hostId()!)?.data?.sessions ?? []) : []);
  const folders = createMemo(() => folderChoices(sessions(), load()?.roots ?? [], undefined));

  // Opening: the Chat tab's PC (or the first), fresh form.
  createEffect(
    on(
      () => props.open,
      (open) => {
        if (!open) return;
        setHostId(props.hostId ?? props.pcs[0]?.id);
        setStep(props.pcs.length > 1 && !props.hostId ? 'pc' : 'form');
        setFirst('');
        setError(undefined);
        setBusy(false);
      },
    ),
  );

  const refresh = async (id: string) => {
    setLoading(true);
    setLoad(undefined);
    setCwd(undefined);
    const roots = await fsRoots(id);
    if (untrack(hostId) !== id) return;
    setLoading(false);
    const l: Load = { roots: roots.ok ? roots.value.roots : [], error: roots.ok ? undefined : roots.error };
    setLoad(l);
    setCwd(folderChoices(untrack(sessions), l.roots)[0] ?? l.roots[0]);
  };
  createEffect(on([() => props.open, hostId], ([open, id]) => open && id && void refresh(id)));

  const canStart = () => !!cwd() && !busy() && !!hostId();

  const start = async () => {
    const id = hostId();
    const dir = cwd();
    if (!id || !dir || !canStart()) return;
    setBusy(true);
    setError(undefined);
    const res = await newSession(id, dir);
    setBusy(false);
    if (!res.ok) {
      setError(errorWords(res.error));
      return;
    }
    props.onStarted({ hostId: id, pcName: pc()?.name ?? '', target: res.value, first: first().trim() });
  };

  const footer = () => (
    <Show when={step() === 'form'}>
      <div class="dialog-actions new-chat-actions">
        <button type="button" class="btn btn-secondary" onClick={props.onClose} disabled={busy()}>
          Cancel
        </button>
        <button type="button" class="btn btn-primary" data-testid="new-chat-start" disabled={!canStart()} onClick={() => void start()}>
          {busy() ? 'Starting…' : 'Start chat'}
        </button>
      </div>
    </Show>
  );

  return (
    <BoardSheet open={props.open} onClose={() => (busy() ? undefined : step() === 'form' ? props.onClose() : setStep('form'))} label="New chat" testId="new-chat-sheet" class="new-chat-sheet" footer={footer()} guarded={busy() || first().length > 0}>
      <Show when={step() === 'pc'}>
        <p class="new-chat-lead">Which PC should run it?</p>
        <div class="settings-group rounded-card bg-surface-2/60">
          <For each={props.pcs}>
            {(p) => (
              <button type="button" class="picker-row" aria-current={p.id === hostId() ? 'true' : undefined} onClick={() => (setHostId(p.id), setStep('form'))}>
                <span class="settings-icon">
                  <Monitor size={18} />
                </span>
                <span class="min-w-0 flex-1 truncate text-left text-[16px] font-medium">{p.name}</span>
                <Show when={p.id === hostId()}>
                  <Check size={18} class="board-accent shrink-0" />
                </Show>
              </button>
            )}
          </For>
        </div>
      </Show>

      <Show when={step() === 'browse' && hostId()}>
        <FolderBrowser hostId={hostId()!} start={cwd() ?? home() ?? '~'} home={home()} onPick={(dir) => (setCwd(dir), setStep('form'))} onBack={() => setStep('form')} />
      </Show>

      <Show when={step() === 'form'}>
        <Show when={props.pcs.length > 1}>
          <button type="button" class="new-chat-pc" onClick={() => setStep('pc')} disabled={busy()}>
            <Monitor size={16} />
            <span class="truncate">{pc()?.name ?? 'Choose a PC'}</span>
            <span class="new-chat-pc-change">Change</span>
          </button>
        </Show>

        <Show when={loading()}>
          <div class="new-chat-skeleton" aria-busy="true" aria-label="Loading">
            <span />
            <span />
            <span />
          </div>
        </Show>

        <Show when={!loading() && load()}>
          {(l) => (
            <>
              <Show when={l().error}>
                <p class="new-chat-note warn" role="alert">
                  {l().error!.message}
                </p>
              </Show>
              <h3 class="section-label">Folder</h3>
              <div class="new-chat-folders">
                <For each={folders()}>
                  {(dir) => (
                    <button type="button" class="new-chat-folder" aria-pressed={cwd() === dir} disabled={busy()} onClick={() => setCwd(dir)} title={dir}>
                      <Folder size={14} />
                      <span class="truncate">{shortPath(dir, home())}</span>
                    </button>
                  )}
                </For>
                <Show when={cwd() && !folders().includes(cwd()!)}>
                  <button type="button" class="new-chat-folder" aria-pressed="true" title={cwd()}>
                    <Folder size={14} />
                    <span class="truncate">{shortPath(cwd()!, home())}</span>
                  </button>
                </Show>
                <button type="button" class="new-chat-folder browse" disabled={busy()} onClick={() => setStep('browse')}>
                  <FolderOpen size={14} />
                  Browse…
                </button>
              </div>

              <label class="section-label" for="new-chat-first">
                First message <span class="new-chat-optional">optional</span>
              </label>
              <textarea
                id="new-chat-first"
                class="new-chat-first"
                rows={3}
                placeholder="What should it do? Sent as soon as the session is ready."
                value={first()}
                disabled={busy()}
                onInput={(e) => setFirst(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
                    e.preventDefault();
                    void start();
                  }
                }}
              />
              <Show when={busy()}>
                <p class="new-chat-note" role="status" aria-live="polite" ref={(el) => queueMicrotask(() => el.scrollIntoView?.({ block: 'nearest' }))}>
                  Starting a session on {pc()?.name ?? 'the PC'}…
                </p>
              </Show>
              <Show when={error()}>
                {(e) => (
                  <div class="new-chat-note warn" role="alert" ref={(el) => queueMicrotask(() => el.scrollIntoView?.({ block: 'nearest' }))}>
                    <b>Couldn't start the chat.</b> {e().text}
                    <Show when={e().hint}>
                      <span class="block mt-1 text-text-muted">{e().hint}</span>
                    </Show>
                  </div>
                )}
              </Show>
            </>
          )}
        </Show>
      </Show>
    </BoardSheet>
  );
}

/** Folder browser over `fs_list` (dirs only): up, open, "Use this folder". */
function FolderBrowser(props: { hostId: string; start: string; home?: string; onPick: (dir: string) => void; onBack: () => void }) {
  const [dir, setDir] = createSignal(props.start);
  const [entries, setEntries] = createSignal<string[] | undefined>();
  const [failed, setFailed] = createSignal<string | undefined>();
  createEffect(
    on(dir, async (d) => {
      setEntries(undefined);
      setFailed(undefined);
      const res = await fsList(props.hostId, d);
      if (dir() !== d) return;
      if (!res.ok) {
        setFailed(res.error.message);
        setEntries([]);
        return;
      }
      setEntries(res.value.entries.filter((e) => e.kind === 'dir' && !e.name.startsWith('.')).map((e) => e.name).sort((a, b) => a.localeCompare(b)));
    }),
  );
  const parent = () => {
    const d = dir().replace(/\/+$/, '');
    const i = d.lastIndexOf('/');
    return i > 0 ? d.slice(0, i) : '/';
  };
  return (
    <div class="new-chat-browse" data-testid="new-chat-browse">
      <div class="new-chat-browse-head">
        <button type="button" class="ghost-icon-button" aria-label="Back" onClick={props.onBack}>
          <ChevronLeft size={20} />
        </button>
        <span class="new-chat-browse-path truncate font-mono" title={dir()}>
          {shortPath(dir(), props.home)}
        </span>
      </div>
      <div class="new-chat-browse-list">
        <Show when={dir() !== '/'}>
          <button type="button" class="new-chat-browse-row" onClick={() => setDir(parent())}>
            <FolderOpen size={16} />
            <span>..</span>
          </button>
        </Show>
        <Show when={entries() === undefined}>
          <p class="new-chat-hint">Loading…</p>
        </Show>
        <Show when={failed()}>
          <p class="new-chat-note warn">{failed()}</p>
        </Show>
        <Show when={entries()?.length === 0 && !failed()}>
          <p class="new-chat-hint">No folders here.</p>
        </Show>
        <For each={entries() ?? []}>
          {(n) => (
            <button type="button" class="new-chat-browse-row" onClick={() => setDir(`${dir().replace(/\/+$/, '')}/${n}`)}>
              <Folder size={16} />
              <span class="truncate">{n}</span>
            </button>
          )}
        </For>
      </div>
      <div class="dialog-actions">
        <button type="button" class="btn btn-primary" data-testid="new-chat-use-folder" onClick={() => props.onPick(dir())}>
          Use this folder
        </button>
      </div>
    </div>
  );
}
