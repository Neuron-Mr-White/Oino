/**
 * Terminals on the phone (§3.9, Z-107): no bottom panel there, so the ⋯
 * menu's Panes → Terminal opens a full-screen layer over the chat. It shows
 * the same terminal tabs as the desktop's bottom panel (the project's
 * `bottom` pane set), as pills with ✕ and a `+`, the active one in the phone
 * `TerminalView` (IME input, toolbar capsule, arrow pad, composer). Android
 * Back closes the layer; the shells keep running.
 *
 * The layer mounts once, on first open, under the opener's owner so it sees
 * the app's settings (toolbar keys, font) like any other surface.
 */
import { createEffect, createRoot, createSignal, For, lazy, onCleanup, Show, Suspense, type Owner } from 'solid-js';
import { insert } from 'solid-js/web';
import { ArrowLeft, Plus, SquareTerminal, X } from 'lucide-solid';
import { panes, type PaneScope, type PaneTab, type PaneView } from '../../../lib/panes';
import { pushBack } from '../../../lib/back';
import { nextTerminalTitle } from '../../../lib/term';
import { confirmTerminalClose, endTerminal } from './terminalClose';
import './TerminalTab.css';

const TerminalView = lazy(() => import('./TerminalTab').then((m) => ({ default: m.TerminalView })));

type Shown = { hostId: string; cwd: string; scope: PaneScope; view: PaneView };
const [shown, setShown] = createSignal<Shown | undefined>();
let mounted = false;

const terminalTabs = (s: Shown): PaneTab[] => panes().visible(s.scope, 'bottom', s.view).filter((t) => t.kind === 'terminal');

function openNew(s: Shown): void {
  const title = nextTerminalTitle(s.cwd, terminalTabs(s).map((t) => t.title ?? ''));
  panes().open(s.scope, 'bottom', s.view, { kind: 'terminal', title });
}

function Layer(props: { s: Shown }) {
  const tabs = () => terminalTabs(props.s);
  const active = () => {
    const a = panes().active(props.s.scope, 'bottom', props.s.view);
    return a?.kind === 'terminal' ? a : tabs()[0];
  };
  const close = () => setShown(undefined);
  // The last terminal gone (✕ or its shell exited): back to the chat.
  createEffect(() => {
    if (tabs().length === 0) close();
  });
  createEffect(() => {
    let off: (() => void) | undefined;
    const raf = requestAnimationFrame(() => (off = pushBack(close)));
    onCleanup(() => {
      cancelAnimationFrame(raf);
      off?.();
    });
  });
  const closeTab = (tab: PaneTab) => {
    if (!confirmTerminalClose(tab, () => closeTab(tab), tab.title)) return;
    endTerminal(tab.id);
    panes().close(props.s.scope, 'bottom', props.s.view, tab.id);
  };
  return (
    <div class="term-phone-layer" role="dialog" aria-modal="true" aria-label="Terminal" data-testid="term-phone-layer">
      <header class="term-phone-head">
        <button type="button" class="term-phone-icon-btn" aria-label="Back to chat" onClick={close} data-testid="term-phone-back">
          <ArrowLeft size={20} />
        </button>
        <div class="term-phone-tabs" role="tablist" aria-label="Terminals">
          <For each={tabs()}>
            {(tab) => (
              <div class="term-phone-pill" data-active={active()?.id === tab.id ? '' : undefined} data-testid="term-phone-pill">
                <button
                  type="button"
                  role="tab"
                  class="term-phone-pill-main"
                  aria-selected={active()?.id === tab.id}
                  title={tab.title}
                  onClick={() => panes().activate(props.s.scope, 'bottom', props.s.view, tab.id)}
                >
                  <SquareTerminal size={15} class="term-phone-pill-icon" />
                  <span class="term-phone-pill-label">{tab.title ?? 'Terminal'}</span>
                </button>
                <button type="button" class="term-phone-pill-close" aria-label={`Close ${tab.title ?? 'terminal'}`} onClick={() => closeTab(tab)} data-testid="term-phone-pill-close">
                  <X size={14} />
                </button>
              </div>
            )}
          </For>
        </div>
        <button type="button" class="term-phone-icon-btn" aria-label="New terminal" onClick={() => openNew(props.s)} data-testid="term-phone-new">
          <Plus size={20} />
        </button>
      </header>
      <div class="term-phone-body">
        <Suspense>
          <Show when={active()} keyed>
            {(tab) => (
              <TerminalView
                tabId={tab.id}
                hostId={props.s.hostId}
                cwd={props.s.cwd}
                where={{ scope: props.s.scope, area: 'bottom', sessionId: props.s.view.sessionId }}
                phone
                close={() => closeTab(tab)}
              />
            )}
          </Show>
        </Suspense>
      </div>
    </div>
  );
}

function PhoneTerminalLayer() {
  return <Show when={shown()} keyed>{(s) => <Layer s={s} />}</Show>;
}

/** Panes → Terminal on the phone: the project's terminals (a new one when there is none). */
export function openPhoneTerminal(ctx: { hostId: string; sessionId?: string; cwd?: string }, owner?: Owner | null): void {
  const cwd = ctx.cwd || '~';
  const s: Shown = { hostId: ctx.hostId, cwd, scope: { hostId: ctx.hostId, projectKey: cwd }, view: { sessionId: ctx.sessionId } };
  if (terminalTabs(s).length === 0) openNew(s);
  if (!mounted && typeof document !== 'undefined') {
    mounted = true;
    const host = document.createElement('div');
    host.className = 'term-phone-root';
    document.body.appendChild(host);
    createRoot(() => insert(host, () => <PhoneTerminalLayer />), owner ?? undefined);
  }
  setShown(s);
}

/** Closes the layer (tests, route changes); the shells keep running. */
export function closePhoneTerminal(): void {
  setShown(undefined);
}

export const phoneTerminalOpen = (): boolean => !!shown();
