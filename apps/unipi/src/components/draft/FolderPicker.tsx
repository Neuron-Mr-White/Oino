/**
 * THE workspace picker (owner spec 2026-11-19): one regular "Open folder" dialog — a location
 * box on top, one list of folders, Up / Backspace to the parent, Enter or double-click to enter a
 * folder, a primary "Use this folder" and Cancel — with zoxide as the engine underneath.
 *
 *  - The box is focused on open. Type a word (or several) and the list is `zoxide query <words>`
 *    on that PC (frecency, every folder zoxide knows), then Oino's recents and the matching folders
 *    right here; Enter opens the top one as the workspace.
 *  - Type something that looks like a path (`~/`, `/`, `./`, `../`) and the box navigates: the folder
 *    it names is listed, filtered by the last segment, and Tab completes it.
 *  - Empty box: "Frequent" (zoxide's top folders, merged with Oino's recent workspaces; plain
 *    "Recent" when zoxide isn't installed) and under it the folders of the current folder.
 *  - ↑/↓ select across the whole list. Enter opens a Frequent/search row as the workspace, or enters
 *    a browsed folder. Ctrl/Cmd+Enter always uses the highlighted folder (else the current one).
 *    Esc clears the box, then closes.
 *
 * Same component on the phone (and ≤520 px): a full-screen sheet, 48 px rows, a tap does the row's
 * default action (open a Frequent/search folder, enter a browsed one), the PC's zoxide arrives via the
 * host's `fs_zoxide` (the host only returns folders this phone may list).
 */
import { Portal } from 'solid-js/web';
import { createEffect, createMemo, createResource, createSignal, For, Index, on, onCleanup, onMount, Show } from 'solid-js';
import { ChevronRight, Folder, FolderSearch, Search, ArrowUp, X } from 'lucide-solid';
import { fsList, fsRoots } from '../../lib/fs';
import { zoxideQuery } from '../../lib/zoxide';
import { layout } from '../../lib/layout';
import { isTauriDesktop, platform } from '../../lib/platform';
import { pickFolderNative } from '../../lib/openLocal';
import { useHostsOptional } from '../../hosts/context';
import { wrapTab } from '../../lib/focusTrap';
import { shortPath } from '../../lib/session/newSession';
import { Button, Sheet } from '../phone';
import { MiddleText } from '../MiddleText';
import {
  homeOf,
  ancestorRows,
  isPathQuery,
  keyAction,
  mergeFrequent,
  parentOf,
  rankDirs,
  rankQuickRows,
  splitDisplay,
  splitPathQuery,
  tabComplete,
  type QuickRow,
} from './workspaceQuery';
import './folderPicker.css';

/** D-20: a folder the PC doesn't let the app see reads calmly (no raw scope message, no CLI hint). */
export const calmFsError = (message: string): string =>
  /outside the allowed|allowed folders|E_SCOPE|scope/i.test(message)
    ? 'Unipi may only browse your home folder and the folders this PC allows. Go back to one of them.'
    : message;

const ZOXIDE_HINT = 'Tip: install zoxide on this PC and search ranks the folders you use most.';
const DEBOUNCE_MS = 110;
const FREQUENT_MAX = 8;

const [quickOpen, setQuickOpen] = createSignal(false);
/** The app-wide layer's open state (the Mod+O `workspace.open` shortcut / "Open workspace…" command). */
export const workspaceQuickOpen = quickOpen;
export function openWorkspaceQuick(open = true): void {
  setQuickOpen(open);
}

type Listing = { dir: string; names: string[]; error?: string };
type Section = 'frequent' | 'match' | 'recent' | 'near' | 'dir';

export function FolderPicker(props: {
  hostId: string;
  home?: string;
  roots?: readonly string[];
  /** The picker's current folder: browsing starts there. */
  initial?: string;
  /** Oino's recent workspaces on this host (the picker's `recentWorkspaces`). */
  recentItems: ReadonlyArray<{ path: string; label: string; detail?: string }>;
  /** Show "No workspace" (runs in a private scratch folder). */
  allowNone?: boolean;
  onNone?: () => void;
  /** This computer: the footer offers the native dialog. */
  ownPc?: boolean;
  /** Dialog title (default "Choose a workspace") and the verb of the primary button (default "Use"). */
  title?: string;
  actionLabel?: string;
  /** The caller's own failure (e.g. the Board refusing a folder), shown above the list. */
  error?: string;
  /** The caller is working on the picked folder: the primary button is off. */
  busy?: boolean;
  onClose: () => void;
  /** Use a folder (button, Enter/double-click on a Frequent/search row, Ctrl/Cmd+Enter, system dialog). */
  onUse: (path: string) => void;
}) {
  const hosts = useHostsOptional();
  const phone = () => !layout().desktop();
  const pcName = createMemo(() => {
    const h = hosts?.hosts().find((x) => x.id === props.hostId);
    return h ? (h.transport === 'local' ? 'This computer' : h.name) : undefined;
  });
  /** The native dialog needs the desktop app AND this computer's host (the transport decides, not only the prop). */
  const canNative = () => {
    if (!isTauriDesktop()) return false;
    const h = hosts?.hosts().find((x) => x.id === props.hostId);
    return !!props.ownPc || h?.transport === 'local';
  };

  // ── where we are ──
  // `fs_roots` lists $HOME first (fs.rs `roots`), so a home outside /home/<user> (a NAS, a container) still resolves `~`.
  const knownHome = () => props.home ?? homeOf(props.roots ?? []) ?? props.roots?.[0];
  const [fetchedRoots] = createResource(
    () => (knownHome() ? false : props.hostId),
    async (id) => {
      const res = await fsRoots(id);
      return res.ok ? res.value.roots : [];
    },
  );
  const home = createMemo(() => knownHome() ?? homeOf(fetchedRoots.latest ?? []) ?? fetchedRoots.latest?.[0]);
  const short = (p: string) => shortPath(p, home());
  const [start, setStart] = createSignal(props.initial || knownHome() || props.roots?.[0] || '/');
  const [dir, setDir] = createSignal(start());
  let moved = !!props.initial;
  const [navigated, setNavigated] = createSignal(false);
  // The home arrived late (fs_roots): start there, unless the user already went somewhere.
  createEffect(() => {
    const h = home();
    if (h && !moved) {
      setStart(h);
      setDir(h);
    }
  });

  const [query, setQuery] = createSignal('');
  const mode = createMemo<'browse' | 'search' | 'path'>(() => (isPathQuery(query()) ? 'path' : query().trim() ? 'search' : 'browse'));
  const place = createMemo(() => (mode() === 'path' ? splitPathQuery(query(), home(), dir()) : undefined));
  /** The folder whose children the list shows (typed path, else the browsed folder). */
  const here = () => place()?.dir ?? dir();

  // ── listings: one fs_list per folder, cached for this picker's session ──
  const cache = new Map<string, Promise<Listing>>();
  const list = (target: string, hidden: boolean): Promise<Listing> => {
    const key = `${hidden ? 1 : 0}${target}`;
    let hit = cache.get(key);
    if (!hit) {
      hit = fsList(props.hostId, target, hidden).then((res): Listing => {
        if (!res.ok) return { dir: target, names: [], error: calmFsError(String(res.error.message)) };
        return {
          dir: target,
          names: res.value.entries
            .filter((e) => (e.kind === 'dir' || e.kind === 'symlink') && (hidden || !e.name.startsWith('.')))
            .map((e) => e.name)
            .sort((a, b) => a.localeCompare(b)),
        };
      });
      cache.set(key, hit);
    }
    return hit;
  };
  const listKey = createMemo(() => {
    const p = place();
    if (mode() === 'path') return p ? `${p.prefix.startsWith('.') ? 1 : 0}${p.dir}` : undefined;
    return `0${dir()}`;
  });
  const [listing] = createResource(listKey, (key) => list(key.slice(1), key[0] === '1'));
  /** The latest listing: the previous rows stay on screen until the next folder's arrive (no blink). */
  const shown = () => listing.latest;
  const children = createMemo<QuickRow[]>(() => {
    const l = shown();
    if (!l || l.error || (mode() === 'path' && !place())) return [];
    return rankDirs(l.dir, mode() === 'path' ? (place()?.prefix ?? '') : '', l.names);
  });
  const allChildren = createMemo<QuickRow[]>(() => {
    const l = shown();
    return l && !l.error ? rankDirs(l.dir, '', l.names) : [];
  });

  // ── zoxide: the Frequent list once, a search per (debounced) query ──
  const [frequent] = createResource(
    () => props.hostId,
    (id) => zoxideQuery(id, '', { limit: 50, home: home() }),
  );
  const [searchHits, setSearchHits] = createSignal<Array<{ path: string; score: number }>>([]);
  const [searchAvailable, setSearchAvailable] = createSignal<boolean | undefined>();
  const [searching, setSearching] = createSignal(false);
  /** The query zoxide has answered for (Enter knows whether a result set is fresh). */
  const [settledFor, setSettledFor] = createSignal<string | undefined>();
  let seq = 0;
  const recents = createMemo<QuickRow[]>(() => props.recentItems.map((r) => ({ path: r.path, label: r.label, ...(r.detail ? { detail: r.detail } : {}) })));
  const zoxideKnown = () => (searchAvailable() === false ? false : frequent.state === 'ready' ? !!frequent()?.available : undefined);

  const runSearch = async (q: string, token: number): Promise<QuickRow[]> => {
    if (token === seq) setSearching(true);
    const r = await zoxideQuery(props.hostId, q, { limit: 20, home: home() });
    const out = rankQuickRows(q, r.hits, recents(), allChildren(), ancestorRows(q, frequent()?.hits ?? [], home()));
    if (token === seq) {
      setSearching(false);
      setSearchHits(r.hits);
      setSearchAvailable(r.available);
      setSettledFor(q);
      setSel(out.length ? 0 : -1);
    }
    return out;
  };
  createEffect(() => {
    const q = query().trim();
    if (mode() !== 'search') {
      seq += 1;
      setSearching(false);
      return;
    }
    const timer = setTimeout(() => void runSearch(q, ++seq), DEBOUNCE_MS);
    onCleanup(() => clearTimeout(timer));
  });

  // ── the rows ──
  const frequentRows = createMemo(() => mergeFrequent(frequent()?.hits ?? [], recents(), FREQUENT_MAX));
  const rows = createMemo<QuickRow[]>(() => {
    if (mode() === 'path') return children();
    if (mode() === 'search') return rankQuickRows(query().trim(), searchHits(), recents(), allChildren(), ancestorRows(query().trim(), frequent()?.hits ?? [], home()));
    // Frequent leads the folder you opened the picker in, until you first go somewhere; then only the listing shows
    // (going back into the start folder must not push its contents down by a screenful).
    return [...(dir() === start() && !navigated() ? frequentRows() : []), ...allChildren()];
  });
  const sectionOf = (row: QuickRow): Section => {
    if (mode() === 'path' || row.source === 'dir') return 'dir';
    if (mode() === 'browse') return 'frequent';
    return row.source === 'zoxide' ? 'match' : row.source === 'recent' ? 'recent' : 'near';
  };
  const heading = (section: Section): string => {
    if (section === 'frequent') return zoxideKnown() === false || !frequent()?.hits.length ? 'Recent' : 'Frequent';
    if (section === 'match') return 'Best matches';
    // Search merges zoxide's frecency with Oino's recents: with zoxide present the owner calls all of it "Frequent".
    if (section === 'recent') return zoxideKnown() === false ? 'Recent' : 'Frequent';
    if (section === 'near') return 'Folders';
    return `In ${short(shown()?.dir ?? here())}${(shown()?.dir ?? here()) === '/' ? '' : '/'}`;
  };
  // The crumbs already say which folder a lone listing is; a heading only separates sections.
  const showHeading = (i: number) => (i === 0 ? sectionOf(rows()[0]!) !== 'dir' : sectionOf(rows()[i - 1]!) !== sectionOf(rows()[i]!));

  // ── selection ──
  const [sel, setSel] = createSignal(-1);
  /** The user moved the highlight (or typed) — the initial "first Frequent row" no longer applies. */
  let touched = false;
  const resetSel = () => {
    touched = true;
    setSel(mode() === 'search' || (mode() === 'path' && place()?.prefix) ? 0 : -1);
  };
  // Desktop: the first Frequent folder is highlighted on open, so Enter opens the project you use most.
  createEffect(() => {
    if (touched || phone() || props.initial || mode() !== 'browse') return;
    if (rows().length && frequentRows().length) setSel(0);
  });
  const selectedRow = () => rows()[sel()];
  /** What "use" means right now: the highlighted folder, else the one being browsed / typed. */
  const target = createMemo<string | undefined>(() => selectedRow()?.path ?? (mode() === 'search' ? undefined : mode() === 'path' && place()?.prefix ? undefined : here()));
  const canUse = () => !!target() && !(selectedRow() === undefined && shown()?.error && target() === shown()?.dir);

  // ── actions ──
  let input: HTMLInputElement | undefined;
  let listEl: HTMLDivElement | undefined;
  // Focus the box on open on every layout (the narrow desktop window has a keyboard). Only the Android/iOS app
  // leaves it down: focusing there raises the soft keyboard over half the list, so a tap on the box does it.
  const autoFocus = () => platform().mode !== 'tauri-mobile';
  const focusInput = () => autoFocus() && input?.focus();
  const enterDir = (path: string) => {
    moved = true;
    setNavigated(true);
    touched = true;
    setDir(path);
    setQuery('');
    setSel(-1);
    focusInput();
  };
  const goUp = () => {
    const base = here();
    if (base !== '/') enterDir(parentOf(base));
  };
  /** A browsed folder, or a plain folder a search matched, is entered; a Frequent / zoxide / recent one opens as the workspace. */
  const enters = (row: QuickRow) => row.source === 'dir' || row.source === 'near';
  const activate = (row: QuickRow) => (enters(row) ? enterDir(row.path) : props.onUse(row.path));
  const move = (step: 1 | -1) => {
    touched = true;
    const last = rows().length - 1;
    if (last < 0) return;
    setSel((at) => (step > 0 ? Math.min(last, at + 1) : Math.max(0, at - 1)));
  };
  /** The folders that are never a workspace by accident: the user's whole home, and the file system root. */
  const isBroad = (path: string) => {
    const at = path.replace(/\/+$/, '') || '/';
    const h = home()?.replace(/\/+$/, '');
    return at === '/' || (!!h && at === h);
  };
  /**
   * Enter right after typing: the debounce may not have answered yet — ask now and open the top hit.
   * `explicit` is a click on "Use this folder": only that (never a key with nothing highlighted) may take the whole
   * home folder or `/`; on those the keys just open what was typed (`~/` lists home) and otherwise do nothing.
   */
  const useTarget = async (explicit = false) => {
    if (mode() === 'search' && settledFor() !== query().trim()) {
      const out = await runSearch(query().trim(), ++seq);
      if (out[0]) activate(out[0]);
      return;
    }
    const t = target();
    if (!t || !canUse()) return;
    if (!explicit && !selectedRow() && isBroad(t)) {
      if (mode() === 'path' && place()) enterDir(t);
      return;
    }
    props.onUse(t);
  };
  const onKeyDown = (e: KeyboardEvent) => {
    // Esc is handled by the picker-wide listener below; everything else belongs to the box.
    if (e.target !== input || e.key === 'Escape') return;
    // Enter right after typing: the list may be stale (zoxide has not answered yet) — ask now and open its top hit.
    if (e.key === 'Enter' && mode() === 'search' && settledFor() !== query().trim()) {
      e.preventDefault();
      return void useTarget();
    }
    const action = keyAction(e, { query: query(), mode: mode(), rows: rows().length, selected: sel(), source: selectedRow()?.source });
    if (action === 'none') return;
    e.preventDefault();
    switch (action) {
      case 'next':
        return move(1);
      case 'prev':
        return move(-1);
      case 'enter': {
        const row = selectedRow();
        return row && activate(row);
      }
      case 'use': {
        const row = selectedRow();
        return row && props.onUse(row.path);
      }
      case 'use-target':
        return void useTarget();
      case 'complete': {
        const next = tabComplete(query(), children(), sel(), place()?.prefix ?? '');
        if (next !== undefined) {
          setQuery(next);
          resetSel();
        }
        return;
      }
      case 'step-in': {
        const row = selectedRow();
        return row && enterDir(row.path);
      }
      case 'swallow':
        return;
      case 'up':
        return goUp();
    }
  };
  /** Esc, from anywhere inside the picker (a header or footer button has focus too): clear the box, then close. */
  const onEscape = (e: KeyboardEvent) => {
    if (e.key !== 'Escape' || e.isComposing) return;
    const action = keyAction(e, { query: query(), mode: mode(), rows: rows().length, selected: sel(), source: selectedRow()?.source });
    e.preventDefault();
    e.stopPropagation();
    if (action === 'clear') {
      setQuery('');
      setSel(-1);
      focusInput();
    } else props.onClose();
  };
  onMount(() => {
    // Capture: the phone sheet's own Esc handling must not close it before the box was cleared.
    document.addEventListener('keydown', onEscape, true);
    onCleanup(() => document.removeEventListener('keydown', onEscape, true));
  });

  // Keep the highlighted row in view.
  createEffect(() => {
    const at = sel();
    queueMicrotask(() => listEl?.querySelector<HTMLElement>(`[data-index="${at}"]`)?.scrollIntoView?.({ block: 'nearest' }));
  });
  // The phone sheet (a drawer) moves focus itself once it has mounted: ask again after it settles.
  onMount(() => {
    queueMicrotask(focusInput);
    for (const ms of [60, 250]) {
      const t = setTimeout(() => document.activeElement === document.body || (phone() && !input?.contains(document.activeElement)) ? focusInput() : undefined, ms);
      onCleanup(() => clearTimeout(t));
    }
  });
  onCleanup(() => cache.clear());

  const useSystemDialog = () =>
    void pickFolderNative(here())
      .then((picked) => picked && props.onUse(picked))
      .finally(() => focusInput());

  // ── crumbs ──
  const crumbs = createMemo(() => {
    const at = here();
    const h = home();
    const inHome = !!h && (at === h || at.startsWith(`${h}/`));
    const out = [{ label: inHome ? '~' : '/', path: inHome ? h! : '/' }];
    let acc = inHome ? h! : '';
    for (const part of (inHome ? at.slice(h!.length) : at).split('/').filter(Boolean)) {
      acc = `${acc}/${part}`;
      out.push({ label: part, path: acc });
    }
    return out;
  });
  let crumbsEl: HTMLElement | undefined;
  createEffect(on(crumbs, () => requestAnimationFrame(() => crumbsEl && (crumbsEl.scrollLeft = crumbsEl.scrollWidth))));

  const note = createMemo<string | undefined>(() => {
    const l = shown();
    if (mode() === 'path' && !place()) return home() ? 'Type a full path, like /home/you/project or ~/project' : undefined;
    if (l?.error && mode() !== 'search') return l.error;
    if (mode() === 'browse' && !listing.loading && !rows().some((r) => r.source === 'dir')) return rows().length ? undefined : 'No folders inside';
    if (mode() === 'path' && !listing.loading && !rows().length) return place()?.prefix ? `No folder in ${short(place()!.dir)}/ matches “${place()!.prefix}”` : `No folders inside ${short(place()!.dir)}`;
    if (mode() === 'search' && !searching() && settledFor() === query().trim() && !rows().length) return `No folder matches “${query().trim()}”`;
    return undefined;
  });

  const verb = () => props.actionLabel ?? 'Use';
  const heading_ = () => props.title ?? 'Choose a workspace';
  /** The primary button's text: a long folder name is cut in the middle so the label — and the footer — stay on one line. */
  const useLabel = () => (selectedRow() ? <span class="fp-use-label">{verb()} “<MiddleText text={selectedRow()!.label} />”</span> : `${verb()} this folder`);

  const body = () => (
    <div class="fp" data-testid="workspace-picker" data-mode={mode()} data-phone={phone() ? '' : undefined} onKeyDown={onKeyDown}>
      <div class="fp-top">
        <div class="fp-locate">
          <button type="button" class="fp-up" aria-label="Parent folder" title="Parent folder (Backspace)" disabled={here() === '/'} onClick={goUp} data-testid="workspace-up">
            <ArrowUp size={18} />
          </button>
          <div class="fp-box">
            <Show when={mode() === 'path'} fallback={<Search size={16} class="fp-box-ic" />}>
              <Folder size={16} class="fp-box-ic" />
            </Show>
            <input
              ref={(el) => (input = el)}
              type="text"
              class="fp-input"
              placeholder="Search or type a path (~/…)"
              aria-label="Folder to open"
              role="combobox"
              aria-expanded="true"
              aria-controls="fp-options"
              aria-activedescendant={sel() >= 0 && rows().length ? `fp-option-${sel()}` : undefined}
              value={query()}
              onInput={(e) => {
                setQuery(e.currentTarget.value);
                setSettledFor(undefined);
                resetSel();
              }}
              autocomplete="off"
              autocapitalize="off"
              autocorrect="off"
              spellcheck={false}
              enterkeyhint="go"
              data-testid="workspace-input"
            />
            <span class="fp-busy" data-on={searching() || listing.loading ? '' : undefined} aria-hidden="true">
              …
            </span>
          </div>
        </div>
        <Show
          when={mode() !== 'search'}
          fallback={
            <p class="fp-status" data-testid="workspace-status">
              {zoxideKnown() === false ? 'Recent and nearby folders · Tab goes inside' : 'Best matches first · Tab goes inside'}
            </p>
          }
        >
          <nav class="fp-crumbs" aria-label="Folder path" ref={(el) => (crumbsEl = el)} data-testid="workspace-crumbs">
            <For each={crumbs()}>
              {(c, i) => (
                <>
                  <Show when={i() > 0}>
                    <ChevronRight size={12} class="fp-crumb-sep" />
                  </Show>
                  <button type="button" class="fp-crumb" aria-current={i() === crumbs().length - 1 ? 'location' : undefined} onClick={() => enterDir(c.path)}>
                    {c.label}
                  </button>
                </>
              )}
            </For>
          </nav>
        </Show>
      </div>
      <Show when={props.error}>
        {(text) => (
          <p class="fp-error" role="alert" data-testid="workspace-error">
            {text()}
          </p>
        )}
      </Show>
      <div class="fp-list" ref={(el) => (listEl = el)} role="listbox" aria-label="Folders" id="fp-options" data-testid="workspace-list">
        {/* Index (not For): every answer builds new row objects, and the rows must not remount (no blink, no lost hover). */}
        <Index each={rows()}>
          {(row, i) => (
            <>
              <Show when={showHeading(i)}>
                <p class="fp-group" role="presentation" data-section={sectionOf(row())} data-testid="workspace-group">
                  {heading(sectionOf(row()))}
                </p>
              </Show>
              <PickerRow
                row={row()}
                index={i}
                selected={sel() === i}
                home={home()}
                phone={phone()}
                onPick={() => (phone() ? activate(row()) : (touched = true, setSel(i)))}
                onOpen={() => activate(row())}
              />
            </>
          )}
        </Index>
        <Show when={note()}>
          {(text) => (
            <p class="fp-note" role="status" data-testid="workspace-note">
              {text()}
            </p>
          )}
        </Show>
        <Show when={listing.loading && !shown() && !rows().length}>
          <p class="fp-note" role="status">
            Loading…
          </p>
        </Show>
        <Show when={zoxideKnown() === false}>
          <p class="fp-hint" data-testid="workspace-zoxide-hint">
            {ZOXIDE_HINT}
          </p>
        </Show>
      </div>
    </div>
  );

  const secondary = () => (
    <>
      <Show when={props.allowNone !== false && props.onNone}>
        <button type="button" class="ws-btn fp-none" onClick={() => props.onNone?.()} data-testid="workspace-none" title="Runs in a private scratch folder">
          No workspace
        </button>
      </Show>
      <Show when={canNative()}>
        <button type="button" class="ws-btn" onClick={useSystemDialog} data-testid="workspace-sysdlg">
          System dialog…
        </button>
      </Show>
    </>
  );

  return (
    <Show
      when={!phone()}
      fallback={
        <Sheet
          open
          onClose={props.onClose}
          label={heading_()}
          title={heading_()}
          detents={['full']}
          initialDetent="full"
          class="fp-sheet"
          testId="workspace-sheet"
          footer={
            <div class="fp-sheet-foot">
              <span class="fp-foot-path" title={target()}>
                <bdi dir="ltr">
                  <MiddleText text={target() ? short(target()!) : ''} tail={16} />
                </bdi>
              </span>
              <Button kind="primary" block disabled={!canUse() || !!props.busy} onClick={() => void useTarget(true)} testId="workspace-use">
                {useLabel()}
              </Button>
              <div class="fp-sheet-secondary">{secondary()}</div>
            </div>
          }
        >
          {body()}
        </Sheet>
      }
    >
      <Portal>
        <div class="sm-overlay fb-overlay" role="presentation" onClick={props.onClose}>
          <div
            class="sm-dialog fp-dialog"
            role="dialog"
            aria-modal="true"
            aria-label={heading_()}
            tabindex="-1"
            onKeyDown={(e: KeyboardEvent) => {
              if (wrapTab(e, e.currentTarget as HTMLElement)) e.stopPropagation();
            }}
            onClick={(e) => e.stopPropagation()}
            data-testid="workspace-browser"
          >
            <div class="sm-head fp-head">
              <span class="sm-title">
                <h2>{heading_()}</h2>
                <Show when={pcName()}>
                  <span class="sm-sub">{pcName()}</span>
                </Show>
              </span>
              <button type="button" class="ghost-icon-button" aria-label="Close" onClick={props.onClose} data-testid="workspace-browser-close">
                <X size={20} />
              </button>
            </div>
            {body()}
            <div class="fp-foot">
              <span class="fp-foot-path" title={target()}>
                <bdi dir="ltr" data-testid="workspace-target">
                  <MiddleText text={target() ? short(target()!) : ''} tail={16} />
                </bdi>
              </span>
              <span class="fp-foot-grow" />
              {secondary()}
              <button type="button" class="ws-btn" onClick={props.onClose}>
                Cancel
              </button>
              <button type="button" class="ws-btn ws-btn-primary" disabled={!canUse() || !!props.busy} onClick={() => void useTarget(true)} data-testid="workspace-use">
                <FolderSearch size={14} />
                {useLabel()}
              </button>
            </div>
          </div>
        </div>
      </Portal>
    </Show>
  );
}

/** One row: folder icon, name (matched letters marked), and — for a folder found by search/Frequent — its dim parent path. */
function PickerRow(props: {
  row: QuickRow;
  index: number;
  selected: boolean;
  home: string | undefined;
  phone: boolean;
  onPick: () => void;
  onOpen: () => void;
}) {
  const isDir = () => props.row.source === 'dir';
  const enters = () => props.row.source === 'dir' || props.row.source === 'near';
  const display = createMemo(() => splitDisplay(props.row.path, props.home));
  const name = () => (props.row.path === '/' ? '/' : isDir() ? props.row.label : display().name);
  const parts = createMemo(() => {
    const marks = new Set(props.row.marks ?? []);
    const out: Array<{ text: string; hit: boolean }> = [];
    for (const [i, ch] of [...name()].entries()) {
      const hit = marks.has(i);
      const last = out[out.length - 1];
      if (last && last.hit === hit) last.text += ch;
      else out.push({ text: ch, hit });
    }
    return out;
  });
  return (
    <button
      type="button"
      role="option"
      id={`fp-option-${props.index}`}
      data-index={props.index}
      data-source={props.row.source}
      aria-selected={props.selected}
      class="fp-row"
      title={props.row.path}
      tabindex="-1"
      onMouseDown={(e) => e.preventDefault()}
      onClick={props.onPick}
      onDblClick={props.onOpen}
    >
      <Folder size={16} class="fp-row-ic" />
      <span class="fp-row-main">
        <span class="fp-row-name">
          <For each={parts()}>{(part) => (part.hit ? <mark class="fp-mark">{part.text}</mark> : part.text)}</For>
        </span>
        <Show when={!isDir()}>
          <span class="fp-row-path" dir="ltr">
            {display().parent}
          </span>
        </Show>
      </span>
      <Show when={enters()}>
        <ChevronRight size={14} class="fp-row-chev" />
      </Show>
      <Show when={props.selected && !props.phone}>
        <span class="fp-enter" aria-hidden="true">
          ↵
        </span>
      </Show>
    </button>
  );
}
