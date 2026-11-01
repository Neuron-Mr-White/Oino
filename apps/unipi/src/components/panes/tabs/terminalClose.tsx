/**
 * Closing a terminal tab kills its shell (WP-A31). When a program is still
 * running in it (`vi`, `top`, a build…) the ✕ asks first — the same sheet
 * look as the app's other "are you sure" asks (Quit UniPi?).
 *
 * Kept free of xterm so `lib/panes/kinds.ts` can wire `canUserClose` without
 * loading the terminal: the live terminals register how busy they are here.
 *
 * Busy = the host says so: `term_list` rows carry `foreground: {name?}` while
 * a program other than the shell holds the PTY. The app's own guess (the
 * output tracker) only answers for a tab with no host terminal yet, or a host
 * that did not answer. "Close others" / "Close all" ask once for every busy
 * terminal they would end (`confirmCloseMany`).
 */
import { createSignal, For, Show } from 'solid-js';
import { render } from 'solid-js/web';
import type { PaneTab } from '../../../lib/panes';
import { hostClient } from '../../../lib/host';
import { clearTermSnapshot, tabTermBinding, unbindTabTerm } from '../../../lib/term';
import { BoardSheet } from '../../board/BoardSheet';

export type TermBusy = { running: boolean; hint?: string; shell?: string; termId?: string; hostId?: string };

type Live = { busy: () => TermBusy; end: () => void };
const live = new Map<string, Live>();
/** Tabs whose kill the user confirmed (the close re-enters `canUserClose`). */
const confirmed = new Set<string>();

/** A live terminal reports how busy it is and how to end it (returns the unregister). */
export function registerTermBusy(tabId: string, busy: () => TermBusy, end: () => void): () => void {
  const entry = { busy, end };
  live.set(tabId, entry);
  return () => {
    if (live.get(tabId) === entry) live.delete(tabId);
    confirmed.delete(tabId);
  };
}

/**
 * The tab is being closed: end its shell. A live terminal disposes itself; a
 * tab that was never shown since a reload only has its binding — kill that
 * terminal on the host.
 */
export function endTerminal(tabId: string): void {
  const entry = live.get(tabId);
  if (entry) {
    entry.end();
    return;
  }
  const binding = tabTermBinding(tabId);
  if (!binding) return;
  unbindTabTerm(tabId);
  clearTermSnapshot(binding.termId);
  void hostClient(binding.hostId)
    .request('term_kill', { termId: binding.termId })
    .catch(() => {});
}

type AskItem = { tabId: string; title: string; hint?: string; shell?: string };
type Ask = AskItem & { proceed: () => void; /** Close others / Close all: every busy terminal at once. */ items?: AskItem[] };
const [ask, setAsk] = createSignal<Ask | undefined>();
let mounted = false;

function shellName(shell: string | undefined): string {
  const leaf = (shell ?? '').split(/[\\/]/).pop() ?? '';
  return leaf.replace(/\.exe$/i, '') || 'terminal';
}

function ConfirmLayer() {
  const answer = (kill: boolean) => {
    const cur = ask();
    setAsk(undefined);
    if (kill && cur) {
      confirmed.add(cur.tabId);
      for (const item of cur.items ?? []) confirmed.add(item.tabId);
      cur.proceed();
    }
  };
  return (
    <BoardSheet open={!!ask()} onClose={() => answer(false)} label={(ask()?.items?.length ?? 0) > 1 ? 'Close terminals?' : 'Close terminal?'} testId="term-close-sheet">
      <Show when={ask()}>
        {(a) => (
          <>
            <Show
              when={(a().items?.length ?? 0) > 1}
              fallback={
                <p class="term-close-body" data-testid="term-close-message">
                  <Show when={a().hint} fallback={<>A program is still running in {a().title}. Closing the tab stops it and ends the {shellName(a().shell)} shell.</>}>
                    <code class="term-close-code">{a().hint}</code> is still running in {a().title}. Closing the tab stops it and ends the {shellName(a().shell)} shell.
                  </Show>
                </p>
              }
            >
              <p class="term-close-body" data-testid="term-close-message">Programs are still running in {a().items!.length} terminals. Closing the tabs stops them and ends their shells:</p>
              <ul class="term-close-list" data-testid="term-close-list">
                <For each={a().items}>
                  {(item) => (
                    <li>
                      <Show when={item.hint} fallback={<span>a program</span>}>
                        <code class="term-close-code">{item.hint}</code>
                      </Show>{' '}
                      in {item.title}
                    </li>
                  )}
                </For>
              </ul>
            </Show>
            <div class="dialog-actions">
              <button type="button" class="btn btn-secondary" data-testid="term-close-keep" onClick={() => answer(false)}>
                Keep running
              </button>
              <button type="button" class="btn btn-danger" data-testid="term-close-confirm" onClick={() => answer(true)}>
                {(a().items?.length ?? 0) > 1 ? 'Close terminals' : 'Close terminal'}
              </button>
            </div>
          </>
        )}
      </Show>
    </BoardSheet>
  );
}

function show(next: Ask): void {
  if (!mounted && typeof document !== 'undefined') {
    mounted = true;
    const host = document.createElement('div');
    host.className = 'term-close-layer';
    document.body.appendChild(host);
    render(() => <ConfirmLayer />, host);
  }
  setAsk(next);
}

type Foreground = { name?: string };

/** The host's `term_list` foreground per terminal id (`null` = the host did not answer). */
async function hostForeground(hostId: string, timeoutMs = 1500): Promise<Map<string, Foreground | undefined> | null> {
  try {
    const rows = await Promise.race([
      hostClient(hostId).request<unknown>('term_list', {}),
      new Promise<never>((_, reject) => setTimeout(() => reject(new Error('timeout')), timeoutMs)),
    ]);
    if (!Array.isArray(rows)) return null;
    const out = new Map<string, Foreground | undefined>();
    for (const row of rows as Array<Record<string, unknown>>) {
      const id = typeof row.term_id === 'string' ? row.term_id : typeof row.termId === 'string' ? row.termId : undefined;
      if (!id) continue;
      const fg = row.foreground && typeof row.foreground === 'object' ? (row.foreground as { name?: unknown }) : undefined;
      out.set(id, fg ? { ...(typeof fg.name === 'string' && fg.name ? { name: fg.name } : {}) } : undefined);
    }
    return out;
  } catch {
    return null;
  }
}

/** Where a tab's terminal runs (the live runtime first, else its saved binding). */
function termOf(tabId: string): { hostId: string; termId: string } | undefined {
  const b = live.get(tabId)?.busy();
  if (b?.termId && b.hostId) return { hostId: b.hostId, termId: b.termId };
  const saved = tabTermBinding(tabId);
  return saved ? { hostId: saved.hostId, termId: saved.termId } : undefined;
}

/** Which of `tabs` run a program, with what (host first; the app's guess only without one). */
async function busyItems(tabs: Array<{ tab: PaneTab; title: string }>): Promise<AskItem[]> {
  const hosts = new Map<string, Promise<Map<string, Foreground | undefined> | null>>();
  const out: AskItem[] = [];
  for (const { tab, title } of tabs) {
    const busy = live.get(tab.id)?.busy();
    const term = termOf(tab.id);
    let running = !!busy?.running;
    let hint = busy?.hint;
    if (term) {
      if (!hosts.has(term.hostId)) hosts.set(term.hostId, hostForeground(term.hostId));
      const fg = await hosts.get(term.hostId)!;
      if (fg) {
        running = fg.has(term.termId) && fg.get(term.termId) !== undefined;
        hint = fg.get(term.termId)?.name ?? (running ? hint : undefined);
      }
    }
    if (running) out.push({ tabId: tab.id, title, ...(hint ? { hint } : {}), ...(busy?.shell ? { shell: busy.shell } : {}) });
  }
  return out;
}

const checking = new Set<string>();

/**
 * `PaneKindDef.canUserClose` for terminals: `false` (and the ask) while a
 * program runs and the user has not confirmed; `proceed()` re-runs the close.
 * A tab with a host terminal asks the host (`term_list` foreground) first and
 * closes by itself when nothing runs.
 */
export function confirmTerminalClose(tab: PaneTab, proceed: () => void, title?: string): boolean {
  if (confirmed.has(tab.id)) return true;
  if (ask() || checking.has(tab.id)) return false;
  const name = title ?? tab.title ?? 'this terminal';
  if (!termOf(tab.id)) {
    // No host terminal yet: only the app's own view can tell.
    const busy = live.get(tab.id)?.busy();
    if (!busy?.running) return true;
    show({ tabId: tab.id, title: name, hint: busy.hint, shell: busy.shell, proceed });
    return false;
  }
  checking.add(tab.id);
  void busyItems([{ tab, title: name }]).then((items) => {
    checking.delete(tab.id);
    const item = items[0];
    if (!item) {
      confirmed.add(tab.id);
      proceed();
      return;
    }
    if (!ask()) show({ ...item, proceed });
  });
  return false;
}

/**
 * "Close others" / "Close all" (P1 owner item): one ask for every terminal
 * among `tabs` that runs a program (host-reported), then `proceed()`; with
 * none busy it proceeds at once. Non-terminal tabs are ignored.
 */
export async function confirmCloseMany(tabs: Array<{ tab: PaneTab; title: string }>, proceed: () => void): Promise<void> {
  const terms = tabs.filter((t) => t.tab.kind === 'terminal' && !confirmed.has(t.tab.id));
  const items = terms.length ? await busyItems(terms) : [];
  if (!items.length) {
    proceed();
    return;
  }
  if (ask()) return;
  show({ ...items[0]!, items, proceed });
}

/** Test hooks. */
export const terminalCloseAsk = ask;
export function resetTerminalClose(): void {
  setAsk(undefined);
  confirmed.clear();
  checking.clear();
  live.clear();
}
