/**
 * The bottom dock (WP-A9, app spec §3.11 — Z-46/Z-50; amends ui-protocol
 * §3.2): the composer region of the conversation, sticky inside the timeline
 * scroll viewport with a gradient fade above (Z-66, in `timeline.css`).
 *
 * Top to bottom (ordered regions, each present only when it has content):
 *
 *   widgets      `widget` slot entries                    (U10)
 *   dialogs      pending pi/Oino dialogs, "1 of N"        (A9 frame, U10 bodies)
 *   queue        steering / follow-up queue               (A11)
 *   tray         `editorReplace` entries                  (U10)
 *   composer     the send box                             (A12 replaces it)
 *   widgetBelow  `widgetBelow` slot entries               (U10)
 *
 * Dialogs dock **above** the composer and never take it over: typing, Enter
 * and Stop keep working while a question waits. A session taken over
 * elsewhere is read-only here — the composer is disabled and the
 * `ReadOnlyBanner` offers Take back.
 */
import { Show, type JSX } from 'solid-js';
import { SendHorizonal, Square } from 'lucide-solid';
import type { SessionHandle } from '../../lib/session/store';
import type { UiRequest } from '../../ui/types';
import { DialogPanel } from './DialogPanel';
import './dock.css';

export function BottomDock(props: {
  session: SessionHandle;
  /** Open dialogs in arrival order (`sessionUi(host).dialogs()`). */
  dialogs?: readonly UiRequest[];
  /** Answers one dialog (`extension_ui_response` on the session stream). */
  onAnswer?: (id: string, value: unknown) => void;
  /** The session is open in another runner: no sending, no stopping. */
  readOnly?: boolean;
  /** Phone shell: dialogs collapse to a one-line bar. */
  phone?: boolean;
  widgets?: JSX.Element;
  /** A11's queue panel. */
  queue?: JSX.Element;
  /** U10's `editorReplace` tray (replaces only the composer). */
  composerTray?: JSX.Element;
  /** A12's composer; the built-in one keeps the surface usable until then. */
  composer?: JSX.Element;
  widgetBelow?: JSX.Element;
}) {
  return (
    <div class="dock" data-testid="bottom-dock">
      <Show when={props.widgets}>
        <div class="dock-region" data-region="widget">
          {props.widgets}
        </div>
      </Show>
      <Show when={props.dialogs?.length}>
        <div class="dock-region dock-region-dialog" data-region="dialog">
          <DialogPanel
            dialogs={props.dialogs ?? []}
            onAnswer={(id, value) => props.onAnswer?.(id, value)}
            phone={props.phone}
          />
        </div>
      </Show>
      <Show when={props.queue}>
        <div class="dock-region" data-region="queue">
          {props.queue}
        </div>
      </Show>
      <Show when={props.composerTray}>
        <div class="dock-region" data-region="tray">
          {props.composerTray}
        </div>
      </Show>
      <div class="dock-region dock-region-composer" data-region="composer">
        {props.composer ?? <DockComposer session={props.session} readOnly={props.readOnly} />}
      </div>
      <Show when={props.widgetBelow}>
        <div class="dock-region" data-region="widgetBelow">
          {props.widgetBelow}
        </div>
      </Show>
    </div>
  );
}

/**
 * The built-in composer (A6's send box; A12 replaces it with
 * `components/composer/*`): Enter sends, Esc stops a running turn, the send
 * button is Stop while it runs. Nothing here is disabled by a waiting dialog.
 */
export function DockComposer(props: { session: SessionHandle; readOnly?: boolean }) {
  let input: HTMLInputElement | undefined;
  const running = () => !!props.session.state.run.running;
  const send = () => {
    const text = input?.value ?? '';
    if (!text.trim() || props.readOnly) return;
    props.session.send(text);
    if (input) input.value = '';
  };
  const stop = () => {
    if (props.readOnly) return;
    void props.session.abort();
  };
  return (
    <div class="tl-composer" data-testid="tl-composer" data-readonly={props.readOnly ? '' : undefined}>
      <input
        ref={input}
        class="tl-composer-input"
        disabled={props.readOnly}
        placeholder={props.readOnly ? 'Read-only — this session is open elsewhere' : running() ? 'Keep typing to steer…' : 'Message…'}
        onKeyDown={(e) => {
          if (e.key === 'Escape' && running()) {
            e.preventDefault();
            stop();
            return;
          }
          if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            send();
          }
        }}
      />
      <Show
        when={running()}
        fallback={
          <button type="button" class="tl-composer-send" aria-label="Send" onClick={send} disabled={props.readOnly}>
            <SendHorizonal size={17} />
          </button>
        }
      >
        <button type="button" class="tl-composer-send tl-composer-stop" aria-label="Stop" onClick={stop} disabled={props.readOnly}>
          <Square size={15} fill="currentColor" />
        </button>
      </Show>
    </div>
  );
}
