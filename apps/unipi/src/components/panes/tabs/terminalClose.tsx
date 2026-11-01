/**
 * Closing a terminal tab kills its shell (WP-A31). When a program is still
 * running in it (`vi`, `top`, a build…) the ✕ asks first — the same sheet
 * look as the app's other "are you sure" asks (Quit UniPi?).
 *
 * Kept free of xterm so `lib/panes/kinds.ts` can wire `canUserClose` without
 * loading the terminal: the live terminals register how busy they are here.
 */
import { createSignal, Show } from 'solid-js';
import { render } from 'solid-js/web';
import type { PaneTab } from '../../../lib/panes';
import { hostClient } from '../../../lib/host';
import { clearTermSnapshot, tabTermBinding, unbindTabTerm } from '../../../lib/term';
import { BoardSheet } from '../../board/BoardSheet';

export type TermBusy = { running: boolean; hint?: string; shell?: string };

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

type Ask = { tabId: string; title: string; hint?: string; shell?: string; proceed: () => void };
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
      cur.proceed();
    }
  };
  return (
    <BoardSheet open={!!ask()} onClose={() => answer(false)} label="Close terminal?" testId="term-close-sheet">
      <Show when={ask()}>
        {(a) => (
          <>
            <p class="term-close-body" data-testid="term-close-message">
              <Show when={a().hint} fallback={<>A program is still running in {a().title}. Closing the tab stops it and ends the {shellName(a().shell)} shell.</>}>
                <code class="term-close-code">{a().hint}</code> is still running in {a().title}. Closing the tab stops it and ends the {shellName(a().shell)} shell.
              </Show>
            </p>
            <div class="dialog-actions">
              <button type="button" class="btn btn-secondary" data-testid="term-close-keep" onClick={() => answer(false)}>
                Keep running
              </button>
              <button type="button" class="btn btn-danger" data-testid="term-close-confirm" onClick={() => answer(true)}>
                Close terminal
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

/**
 * `PaneKindDef.canUserClose` for terminals: `false` (and the ask) while a
 * program runs and the user has not confirmed; `proceed()` re-runs the close.
 */
export function confirmTerminalClose(tab: PaneTab, proceed: () => void, title?: string): boolean {
  if (confirmed.has(tab.id)) return true;
  const busy = live.get(tab.id)?.busy();
  if (!busy?.running) return true;
  if (ask()) return false;
  show({ tabId: tab.id, title: title ?? tab.title ?? 'this terminal', hint: busy.hint, shell: busy.shell, proceed });
  return false;
}

/** Test hooks. */
export const terminalCloseAsk = ask;
export function resetTerminalClose(): void {
  setAsk(undefined);
  confirmed.clear();
  live.clear();
}
