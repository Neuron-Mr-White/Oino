/**
 * The phone's session-tab surfaces (WP-A10, §3.9): a side-pane tab is a
 * **large bottom sheet** there for tree / side chat / subagent and a
 * **full-screen layer** for the diff viewer. Mounted once by `main.tsx`
 * (`<SessionTabLayer />`), above the routes; Android Back closes it.
 *
 * The desktop side pane and these surfaces render the same components from
 * `lib/panes/kinds.ts`, so a tab looks and behaves the same in both shells.
 */
import { createEffect, createMemo, onCleanup, Show } from 'solid-js';
import { ArrowLeft, X } from 'lucide-solid';
import type { PaneTab } from '../../../lib/panes';
import { paneKind } from '../../../lib/panes/kinds';
import { pushBack } from '../../../lib/back';
import { BoardSheet } from '../../board/BoardSheet';
import { closePhoneSessionTab, phoneSessionTab, type PhoneSessionTab } from './sessionTabs';
import './tabs.css';

/** A synthetic tab for the shared view components. */
function tabOf(open: PhoneSessionTab): PaneTab {
  return { id: `phone:${open.kind}`, kind: open.kind, params: open.params, title: open.title, sessionId: open.base.sessionId };
}

/** The view of the open surface (registered by the tab modules). */
function TabView(props: { open: PhoneSessionTab }) {
  const tab = createMemo(() => tabOf(props.open));
  return (
    <Show when={paneKind(props.open.kind)?.component} fallback={<p class="stab-note">This tab isn't available in this build.</p>}>
      {(Comp) => {
        const C = Comp();
        return (
          <C
            tab={tab()}
            area="side"
            hostId={props.open.base.hostId}
            projectKey={String(props.open.params.root ?? '~')}
            sessionId={props.open.base.sessionId}
            close={closePhoneSessionTab}
            setTitle={() => {}}
          />
        );
      }}
    </Show>
  );
}

export function SessionTabLayer() {
  const open = () => phoneSessionTab();
  // Android Back closes the surface (the sheet also closes on swipe / ✕).
  // Registered after the first frame: the router settles its own history
  // entry on mount, and a Back stacked before that reads the two as one.
  createEffect(() => {
    if (!open()) return;
    let off: (() => void) | undefined;
    const raf = requestAnimationFrame(() => {
      off = pushBack(closePhoneSessionTab);
    });
    onCleanup(() => {
      cancelAnimationFrame(raf);
      off?.();
    });
  });
  return (
    <Show when={open()} keyed>
      {(o) => (
        <Show
          when={o.kind !== 'diff'}
          fallback={
            <div class="stab-layer" role="dialog" aria-modal="true" aria-label={o.title} data-testid="session-tab-layer" data-kind={o.kind}>
              <header class="stab-layer-head">
                <button type="button" class="ghost-icon-button" aria-label="Back to chat" onClick={closePhoneSessionTab} data-testid="session-tab-back">
                  <ArrowLeft size={20} />
                </button>
                <span class="stab-layer-title truncate">{o.title}</span>
              </header>
              <div class="stab-layer-body">
                <TabView open={o} />
              </div>
            </div>
          }
        >
          <BoardSheet
            open
            onClose={closePhoneSessionTab}
            label={o.title}
            title={<span class="truncate">{o.title}</span>}
            headerAction={
              <button type="button" class="ghost-icon-button board-sheet-close" aria-label="Close" onClick={closePhoneSessionTab} data-testid="session-tab-close">
                <X size={20} />
              </button>
            }
            testId="session-tab-sheet"
            class="stab-sheet"
          >
            <div class="stab-sheet-body" data-kind={o.kind}>
              <TabView open={o} />
            </div>
          </BoardSheet>
        </Show>
      )}
    </Show>
  );
}
