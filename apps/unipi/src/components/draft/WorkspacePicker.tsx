/**
 * The draft's workspace row (Z-19): where the new chat runs. A chip — folder icon, name, chevron —
 * that opens THE workspace picker (FolderPicker.tsx): one regular folder dialog with zoxide underneath
 * (desktop modal, full-screen sheet on the phone). "No workspace" runs the session in a private
 * scratch folder the host makes for that chat (never the user's home, where the agent could touch
 * everything). A PC chip sits beside it when more than one PC is paired.
 */
import { createMemo, createSignal, For, Show } from 'solid-js';
import { Check, ChevronDown, ChevronRight, Folder, FolderLock, Monitor } from 'lucide-solid';
import { AnchoredMenu } from '../board/AnchoredMenu';
import { layout } from '../../lib/layout';
import { shortPath, folderName } from '../../lib/session/newSession';
import { isOwnPcFile } from '../../lib/openLocal';
import { isHomeCwd } from '../../lib/session/selectors';
import { homeOf, parentOf } from './workspaceQuery';
import { FolderPicker } from './FolderPicker';

/** What "No workspace" means, in the chip tooltip and the menu row. */
export const NO_WORKSPACE_DETAIL = 'Runs in a private scratch folder';

/** Scratch locations are not projects: hidden from Recent unless the user opened them explicitly. */
const isTempPath = (p: string) => /^\/(var\/)?tmp(\/|$)/.test(p);
const OPENED_KEY = 'unipi.workspace.opened';
const explicitlyOpened = (): string[] => {
  try {
    return JSON.parse(localStorage.getItem(OPENED_KEY) ?? '[]') as string[];
  } catch {
    return [];
  }
};
const rememberOpened = (path: string) => {
  try {
    localStorage.setItem(OPENED_KEY, JSON.stringify([path, ...explicitlyOpened().filter((p) => p !== path)].slice(0, 20)));
  } catch {
    // best effort
  }
};

/** Re-exported: the home among a host's `fs_roots` lives with the path helpers. */
export { homeOf };

export type Item = { path: string; label: string; detail: string };

/** Re-exported: path helpers live with the pure picker rules. */
export { parentOf };

/** Oino's recent workspaces (owner 2026-11-03): newest first, no home, no scratch
 *  unless opened explicitly, at most 6. The picker merges them under zoxide's Frequent list. */
export function recentWorkspaces(recent: readonly string[], home: string | undefined, short: (p: string) => string): Item[] {
  const opened = explicitlyOpened();
  return recent
    // Home and the scratch folders are "No workspace", not projects.
    .filter((p) => p !== home && !isHomeCwd(p) && (!isTempPath(p) || opened.includes(p)))
    .slice(0, 6)
    .map((path) => ({ path, label: folderName(path), detail: short(path) }));
}

export function WorkspacePicker(props: {
  hostId: string;
  /** The chosen folder (`undefined` = No workspace). */
  value: string | undefined;
  onChange: (path: string | undefined) => void;
  recent: readonly string[];
  roots: readonly string[];
  home?: string;
  /** Hide the "No workspace" choice (e.g. Cron jobs require an explicit folder). */
  allowNone?: boolean;
  placeholder?: string;
  disabled?: boolean;
}) {
  const [open, setOpen] = createSignal(false);
  const pick = (path: string | undefined) => {
    if (path) rememberOpened(path);
    props.onChange(path);
    setOpen(false);
  };
  const label = () => (props.value ? folderName(props.value) : (props.placeholder ?? 'No workspace'));
  const short = (p: string) => shortPath(p, props.home);
  const recentItems = createMemo<Item[]>(() => recentWorkspaces(props.recent, props.home, short));
  const phone = () => !layout().desktop();

  return (
    <>
      <button
        type="button"
        class="ws-chip"
        aria-haspopup="dialog"
        aria-expanded={open()}
        title={props.value ? `Workspace: ${props.value}` : `No workspace — ${NO_WORKSPACE_DETAIL.toLowerCase()}`}
        disabled={props.disabled}
        onClick={() => setOpen(!open())}
        data-testid="workspace-chip"
      >
        <Show when={props.value} fallback={<FolderLock size={14} class="ws-chip-ic" />}>
          <Folder size={14} class="ws-chip-ic" />
        </Show>
        {/* Phone (§3.6, PH-30): a 56 px "Workspace" row — label, value, chevron. */}
        <Show when={phone()}>
          <span class="ws-chip-key">Workspace</span>
        </Show>
        <span class="ws-chip-label">{label()}</span>
        <Show when={phone()} fallback={<ChevronDown size={13} class="ws-chip-chev" />}>
          <ChevronRight size={18} class="ws-chip-chev" />
        </Show>
      </button>
      {/* ONE picker (desktop modal / phone full-screen sheet); zoxide is its engine. */}
      <Show when={open()}>
        <FolderPicker
          hostId={props.hostId}
          home={props.home}
          roots={props.roots}
          initial={props.value}
          recentItems={recentItems()}
          ownPc={isOwnPcFile(props.hostId)}
          allowNone={props.allowNone}
          onNone={props.allowNone !== false ? () => pick(undefined) : undefined}
          onClose={() => setOpen(false)}
          onUse={(picked) => pick(picked)}
        />
      </Show>
    </>
  );
}

/** The PC chip (only when more than one PC is paired). */
export function HostPicker(props: {
  hostId: string;
  hosts: ReadonlyArray<{ id: string; name: string; transport?: string }>;
  onPick: (id: string) => void;
  disabled?: boolean;
}) {
  const [anchor, setAnchor] = createSignal<DOMRect | undefined>();
  const nameOf = (h: { name: string; transport?: string }) => (h.transport === 'local' ? 'This computer' : h.name);
  const current = () => props.hosts.find((h) => h.id === props.hostId);
  return (
    <>
      <button
        type="button"
        class="ws-chip"
        aria-haspopup="menu"
        aria-expanded={!!anchor()}
        title="Which PC runs the chat"
        disabled={props.disabled}
        onClick={(e) => (anchor() ? setAnchor(undefined) : setAnchor((e.currentTarget as HTMLElement).getBoundingClientRect()))}
        data-testid="host-chip"
      >
        <Monitor size={14} class="ws-chip-ic" />
        <span class="ws-chip-label">{current() ? nameOf(current()!) : 'PC'}</span>
        <ChevronDown size={13} class="ws-chip-chev" />
      </button>
      <Show when={anchor()}>
        {(rect) => (
          <AnchoredMenu anchor={rect()} width={240} label="PC" onDismiss={() => setAnchor(undefined)}>
            <div class="ws-menu">
              <For each={props.hosts}>
                {(h) => (
                  <button
                    type="button"
                    role="menuitemradio"
                    aria-checked={h.id === props.hostId}
                    class="ws-row"
                    onClick={() => {
                      setAnchor(undefined);
                      props.onPick(h.id);
                    }}
                  >
                    <Monitor size={15} class="ws-row-ic" />
                    <span class="ws-row-text">
                      <span class="ws-row-label">{nameOf(h)}</span>
                    </span>
                    <Show when={h.id === props.hostId}>
                      <Check size={15} class="ws-row-check" />
                    </Show>
                  </button>
                )}
              </For>
            </div>
          </AnchoredMenu>
        )}
      </Show>
    </>
  );
}
