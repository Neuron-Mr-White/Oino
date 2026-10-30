/**
 * "New chat" (UNI-260): Chat list + (phone + desktop) and the desktop palette.
 * One sheet, three steps on one page: PC → where (an existing herdr
 * workspace gets a new tab; or a new workspace; tmux on tmux-only PCs) →
 * folder (chips from running pis / Files roots, or browse) → optional first
 * message. Start asks the host (`chat_new`), which starts the PC's pi
 * launcher and waits for its bridge; the chat then opens and the first
 * message goes out once it's connected.
 */
import { createEffect, createMemo, createSignal, For, on, Show, untrack } from 'solid-js';
import { Check, ChevronLeft, Folder, FolderOpen, Monitor, Plus, SquareTerminal, LayoutGrid } from 'lucide-solid';
import { type AppError, type TreeData } from '@unipi/protocol';
import { MuxTreeResultSchema } from '../../lib/schemas';
import { BoardSheet } from '../board/BoardSheet';
import { call } from '../../lib/ipc';
import { fsList, fsRoots } from '../../lib/fs';
import {
  chatConfig,
  errorWords,
  folderChoices,
  folderName,
  isOlderHost,
  newChatSession,
  shortPath,
  whereOptions,
  type ChatNewData,
  type WhereOption,
} from '../../lib/chat/newChat';
import { desktopShell } from '../../lib/layout';

export type NewChatPc = { id: string; name: string };
export type NewChatStarted = { hostId: string; pcName: string; data: ChatNewData; title: string; label: string; first: string };

type Load = { tree?: TreeData; roots: string[]; herdr: boolean; tmux: boolean; launcher: string; older: boolean; error?: AppError };

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
  const [where, setWhere] = createSignal<string | undefined>();
  const [cwd, setCwd] = createSignal<string | undefined>();
  const [name, setName] = createSignal('');
  const [nameTouched, setNameTouched] = createSignal(false);
  const [first, setFirst] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<{ text: string; hint?: string } | undefined>();
  const pc = () => props.pcs.find((p) => p.id === hostId());
  const home = () => load()?.roots[0];

  const options = createMemo<WhereOption[]>(() => {
    const l = load();
    return l ? whereOptions(l.tree, { herdr: l.herdr, tmux: l.tmux }) : [];
  });
  const picked = () => options().find((o) => o.id === where());
  const folders = createMemo(() => folderChoices(load()?.tree, load()?.roots ?? [], undefined));

  // Opening: the Chat tab's PC (or the first), fresh form.
  createEffect(
    on(
      () => props.open,
      (open) => {
        if (!open) return;
        setHostId(props.hostId ?? props.pcs[0]?.id);
        setStep(props.pcs.length > 1 && !props.hostId ? 'pc' : 'form');
        setFirst('');
        setName('');
        setNameTouched(false);
        setError(undefined);
        setBusy(false);
      },
    ),
  );

  const refresh = async (id: string) => {
    setLoading(true);
    setLoad(undefined);
    setWhere(undefined);
    setCwd(undefined);
    const [cfg, tree, roots] = await Promise.all([chatConfig(id), call('mux_tree', { hostId: id, relay: 'n0' }, MuxTreeResultSchema), fsRoots(id)]);
    if (untrack(hostId) !== id) return;
    setLoading(false);
    const older = !cfg.ok && isOlderHost(cfg.error);
    const l: Load = {
      tree: tree.ok ? tree.value.tree : undefined,
      roots: roots.ok ? roots.value.roots : [],
      herdr: cfg.ok ? cfg.value.herdr : !!(tree.ok && tree.value.tree.herdr.available),
      tmux: cfg.ok ? cfg.value.tmux : false,
      launcher: cfg.ok ? cfg.value.launcher : 'pi',
      older,
      error: !cfg.ok && !older ? cfg.error : !tree.ok ? tree.error : undefined,
    };
    setLoad(l);
    const opts = whereOptions(l.tree, { herdr: l.herdr, tmux: l.tmux });
    // Default: a new workspace when herdr is there (the chat gets its own place), else the first option.
    const preferred = opts.find((o) => o.target.kind === 'herdr_new_workspace') ?? opts[0];
    setWhere(preferred?.id);
    setCwd(folderChoices(l.tree, l.roots)[0] ?? l.roots[0]);
  };
  createEffect(on([() => props.open, hostId], ([open, id]) => open && id && void refresh(id)));

  // An existing place brings its own folder; a new one keeps the chosen folder.
  createEffect(
    on(where, () => {
      const o = picked();
      if (o?.cwd && !o.needsName) setCwd(o.cwd);
    }),
  );
  // The new place's name follows the folder until the user types one.
  createEffect(() => {
    const dir = cwd();
    if (!nameTouched()) setName(dir ? folderName(dir) : '');
  });

  const canStart = () => !!picked() && !!cwd() && !busy() && !load()?.older && (!picked()!.needsName || name().trim().length > 0);

  const start = async () => {
    const o = picked();
    const id = hostId();
    const dir = cwd();
    if (!o || !id || !dir || !canStart()) return;
    setBusy(true);
    setError(undefined);
    const label = name().trim().slice(0, 128);
    const target =
      o.target.kind === 'herdr_new_workspace'
        ? { ...o.target, label: label || folderName(dir) }
        : o.target.kind === 'tmux_new_session'
          ? { kind: 'tmux_new_session' as const, name: label || folderName(dir) }
          : o.target;
    const res = await newChatSession(id, target, dir);
    setBusy(false);
    if (!res.ok) {
      setError(errorWords(res.error));
      return;
    }
    const groupLabel = o.needsName ? label || folderName(dir) : o.title;
    props.onStarted({ hostId: id, pcName: pc()?.name ?? '', data: res.value, title: folderName(dir), label: groupLabel, first: first().trim() });
  };

  const footer = () => (
    <Show when={step() === 'form'}>
      <div class="dialog-actions new-chat-actions">
        <button type="button" class="btn btn-secondary" onClick={props.onClose} disabled={busy()}>
          Cancel
        </button>
        <button type="button" class="btn btn-primary" data-testid="new-chat-start" disabled={!canStart()} onClick={() => void start()}>
          {busy() ? 'Starting pi…' : 'Start chat'}
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
              <Show when={l().older}>
                <p class="new-chat-note warn" role="alert">
                  {pc()?.name ?? 'This PC'} runs an older UniPi that can't start chats. Update UniPi on the PC (re-run the installer), then try again.
                </p>
              </Show>
              <Show when={!l().older && options().length === 0}>
                <p class="new-chat-note warn" role="alert">
                  {l().error ? `${l().error!.message}` : `Neither herdr nor tmux is running on ${pc()?.name ?? 'this PC'}.`} Start herdr (or install tmux) on the PC to start chats from here.
                </p>
              </Show>
              <Show when={!l().older && options().length > 0}>
                <h3 class="section-label">Where</h3>
                <div class="new-chat-where" role="radiogroup" aria-label="Where">
                  <For each={options()}>
                    {(o) => (
                      <button
                        type="button"
                        role="radio"
                        class="new-chat-where-row"
                        aria-checked={where() === o.id}
                        disabled={busy()}
                        onClick={() => setWhere(o.id)}
                      >
                        <span class="new-chat-where-icon">{o.needsName ? <Plus size={16} /> : o.target.kind.startsWith('tmux') ? <SquareTerminal size={16} /> : <LayoutGrid size={16} />}</span>
                        <span class="min-w-0 flex-1 text-left">
                          <span class="new-chat-where-title truncate">{o.title}</span>
                          <span class="new-chat-where-sub truncate">
                            {o.sub}
                            <Show when={o.cwd}> · {shortPath(o.cwd!, home())}</Show>
                          </span>
                        </span>
                        <Show when={where() === o.id}>
                          <Check size={18} class="board-accent shrink-0" />
                        </Show>
                      </button>
                    )}
                  </For>
                </div>

                <Show when={picked()?.needsName}>
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
                  <label class="section-label" for="new-chat-name">
                    {picked()?.target.kind === 'tmux_new_session' ? 'Session name' : 'Workspace name'}
                  </label>
                  <input
                    id="new-chat-name"
                    type="text"
                    class="files-search-input new-chat-input"
                    maxLength={128}
                    value={name()}
                    disabled={busy()}
                    onInput={(e) => (setNameTouched(true), setName(e.currentTarget.value))}
                  />
                </Show>
                <Show when={picked() && !picked()!.needsName}>
                  <p class="new-chat-note">
                    Opens a new {picked()!.target.kind === 'tmux_window' ? 'window' : 'tab'} in <b>{picked()!.title}</b>
                    <Show when={cwd()}> at <span class="font-mono">{shortPath(cwd()!, home())}</span></Show>.
                  </p>
                </Show>

                <label class="section-label" for="new-chat-first">
                  First message <span class="new-chat-optional">optional</span>
                </label>
                <textarea
                  id="new-chat-first"
                  class="new-chat-first"
                  rows={3}
                  placeholder="What should pi do? Sent as soon as it's ready."
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
                <p class="new-chat-hint">
                  Runs <span class="font-mono">{l().launcher}</span> on {pc()?.name ?? 'the PC'}
                  {desktopShell() ? ' (change it in Settings → PCs).' : ' · Settings → PCs to change.'}
                </p>
                <Show when={busy()}>
                  <p class="new-chat-note" role="status" aria-live="polite" ref={(el) => queueMicrotask(() => el.scrollIntoView?.({ block: 'nearest' }))}>
                    Starting pi on {pc()?.name ?? 'the PC'}… this can take a few seconds.
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
