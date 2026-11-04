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
 *   working line one line: working / waiting / needs you (ask) / background
 *   composer     the send box                             (A12 replaces it)
 *   widgetBelow  `widgetBelow` slot entries               (U10)
 *
 * Dialogs dock **above** the composer and never take it over: typing, Enter
 * and Stop keep working while a question waits. A session taken over
 * elsewhere is read-only here — the composer is disabled and the
 * `ReadOnlyBanner` offers Take back.
 */
import { Show, createEffect, createMemo, createSignal, onCleanup, type JSX } from 'solid-js';
import type { SessionHandle } from '../../lib/session/store';
import type { UiRequest } from '../../ui/types';
import { sendModeForEnter, followUpMode } from '../../lib/composer/sendMode';
import { installEscapeStop } from '../../lib/composer/escapeStop';
import { DialogPanel } from './DialogPanel';
import { inlineAskCalls } from './AskPendingChip';
import { WorkingLine } from './WorkingLine';
import { dialogShownInApp } from '../../lib/surfaces';
import { QueuePanel } from './QueuePanel';
import { SendButton } from '../composer/SendButton';
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
  /** A11's queue panel (the default below is it). */
  queue?: JSX.Element;
  /** U10's `editorReplace` tray (replaces only the composer). */
  composerTray?: JSX.Element;
  /** A12's composer; the built-in one keeps the surface usable until then. */
  composer?: JSX.Element;
  widgetBelow?: JSX.Element;
  /** The Running tab's live summary ("Subagents 1 running"); feeds the working line. */
  trayLabel?: string;
}) {
  // Esc stops the run from any focus in the conversation (Z-55): the dock
  // itself is inside `.conv-tl`, so keydowns from anywhere above it bubble
  // here — unless a dialog or popup consumed them first.
  let dockEl: HTMLDivElement | undefined;
  createEffect(() => {
    const el = dockEl;
    if (!el) return;
    const root = (el.closest('.conv-tl') as HTMLElement | null) ?? el;
    const off = installEscapeStop({
      root,
      running: () => !props.readOnly && !!props.session.state.run.running,
      stop: () => void props.session.abort(),
    });
    onCleanup(off);
  });
  // [ask-user] questions answered in their inline timeline card: a chip here, not the form.
  const askCalls = createMemo(() => (props.dialogs?.length ? inlineAskCalls(props.dialogs, props.session.turns()) : []));
  const docked = () => {
    const inline = askCalls();
    // A dialog meant for the terminal only (`surfaces: ["tui"]`, e.g. /answer's editor) is the
    // terminal's to answer: it never opens here when a chat is opened.
    const all = (props.dialogs ?? []).filter(dialogShownInApp);
    return inline.length ? all.filter((d) => !inline.some((c) => c.dialog.id === d.id)) : all;
  };
  return (
    <div class="dock" data-testid="bottom-dock" ref={dockEl}>
      <Show when={props.widgets}>
        <div class="dock-region" data-region="widget">
          {props.widgets}
        </div>
      </Show>
      <Show when={docked().length}>
        <div class="dock-region dock-region-dialog" data-region="dialog">
          <DialogPanel
            dialogs={docked()}
            onAnswer={(id, value) => props.onAnswer?.(id, value)}
            phone={props.phone}
          />
        </div>
      </Show>
      <div class="dock-region" data-region="queue">
        {props.queue ?? <QueuePanel session={props.session} phone={props.phone} />}
      </div>
      <div class="dock-region dock-region-working" data-region="working">
        <WorkingLine session={props.session} askCalls={askCalls()} tray={props.trayLabel} />
      </div>
      <div class="dock-region dock-region-composer" data-region="composer">
        {/* §3.11 / §6.2: an `editorReplace` entry takes the composer's place
            (U10's tray); dialogs above it still win. */}
        {props.composerTray ?? props.composer ?? <DockComposer session={props.session} readOnly={props.readOnly} />}
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
 * `components/composer/*`): Enter sends — the global follow-up default
 * while it runs, Cmd/Ctrl+Enter the opposite (Z-52) — and the send control
 * is A11's `SendButton` (Send ↔ Stop Z-55, "Send now (stops run)"). Nothing
 * here is disabled by a waiting dialog.
 */
export function DockComposer(props: { session: SessionHandle; readOnly?: boolean }) {
  let input: HTMLInputElement | undefined;
  const [text, setText] = createSignal('');
  const running = () => !!props.session.state.run.running;
  const placeholder = () =>
    props.readOnly
      ? 'Read-only — this session is open elsewhere'
      : running()
        ? `Keep typing to ${followUpMode() === 'steer' ? 'steer' : 'queue'}…`
        : 'Message…';
  const send = (mode?: 'prompt' | 'steer' | 'followUp') => {
    const value = input?.value ?? '';
    if (!value.trim() || props.readOnly) return;
    props.session.send(value, mode ? { mode } : undefined);
    if (input) input.value = '';
    setText('');
  };
  const stop = () => {
    if (props.readOnly) return;
    void props.session.abort();
  };
  /** "Send now (stops run)": stop, then send the text as a prompt once idle. */
  const sendNow = () => {
    const value = input?.value ?? '';
    if (!value.trim() || props.readOnly) return;
    if (input) input.value = '';
    setText('');
    void props.session
      .abort()
      .then(() => props.session.send(value, { mode: 'prompt' }))
      .catch(() => undefined);
  };
  return (
    <div class="tl-composer" data-testid="tl-composer" data-readonly={props.readOnly ? '' : undefined}>
      <input
        ref={input}
        class="tl-composer-input"
        disabled={props.readOnly}
        placeholder={placeholder()}
        onInput={(e) => setText(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
            e.preventDefault();
            send(sendModeForEnter(e, running()));
          }
        }}
      />
      <SendButton running={running()} disabled={props.readOnly} empty={!text().trim()} onSend={() => send()} onStop={stop} onSendNow={sendNow} />
    </div>
  );
}
