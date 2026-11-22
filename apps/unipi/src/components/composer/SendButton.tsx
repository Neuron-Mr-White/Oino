/**
 * The send / stop control (WP-A11, Z-52 + Z-55): the button is **Stop**
 * while a run is going, **Send** otherwise. Its chevron menu is the only
 * send-related menu left (the v1 S/Q inline choice is gone): it switches
 * the global follow-up default (queue = "After turn" | steer = "Next step") and keeps unipi's
 * "Send now (stops run)". Esc closes the menu and is consumed — it must
 * not stop the run (Z-55's popup rule).
 *
 * "Send as note" (a toggle under its own separator) is a mode, not a send
 * option: while on, Send saves a note (the agent never reads it) and the
 * button shows the note icon. The composer owns the mode.
 */
import { Show, createSignal, onCleanup } from 'solid-js';
import { ArrowUp, ChevronUp, Square, StickyNote, Zap } from 'lucide-solid';
import { followUpMode, modeLabel, setFollowUpMode } from '../../lib/composer/sendMode';
import { modKey } from '../../lib/shell';
import { Tooltip } from '../Tooltip';
import './sendButton.css';

export function SendButton(props: {
  running: boolean;
  /** No input text: Send is disabled (Stop never is). */
  empty?: boolean;
  disabled?: boolean;
  /** Send is held for another reason (WP-A13: attachments still uploading) —
   *  it disables Send only, never Stop. */
  blocked?: boolean;
  /** Why Send is held (tooltip). */
  blockedReason?: string;
  onSend(): void;
  onStop(): void;
  /** "Send now (stops run)": abort, then send the text as a prompt. */
  onSendNow?(): void;
  /** The queue/steer chevron (off on the new-chat draft: nothing runs there yet). */
  options?: boolean;
  /** Note mode is on: the button saves a note. */
  note?: boolean;
  /** "Send as note" toggle in the menu (absent: no such item). */
  onToggleNote?(): void;
}) {
  const [menu, setMenu] = createSignal(false);
  const close = () => setMenu(false);
  /** Z-116: a disabled Send still explains itself ("Type something to send"). */
  const sendTip = () => (props.running && props.empty ? 'Stop this run' : props.note ? (props.empty ? 'Type a note to save' : 'Save note') : props.empty ? 'Type something to send' : props.blocked ? (props.blockedReason ?? 'Send is held') : 'Send');

  // A pointerdown elsewhere closes the menu (capture phase, so it wins over
  // the menu's own clicks travelling up).
  const onDocPointerDown = (e: PointerEvent) => {
    if (!menu()) return;
    if (e.composedPath?.().some((t) => t instanceof Element && t.closest?.('[data-send-menu-root]'))) return;
    close();
  };
  document.addEventListener('pointerdown', onDocPointerDown, true);
  onCleanup(() => document.removeEventListener('pointerdown', onDocPointerDown, true));

  // Esc inside the control closes the menu and is consumed (a popup took it,
  // Z-55) — it must not also stop the run. Native listener: it must run
  // before the conversation root's stop rule (Solid's `onKeyDown` would run
  // at the document, too late). Everything else bubbles normally.
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Escape' && menu()) {
      e.preventDefault();
      e.stopPropagation();
      close();
    }
  };

  return (
    <div class="send-ctl" data-send-menu-root data-open={menu() ? '' : undefined} on:keydown={onKeyDown}>
      <Show when={menu()}>
        <div class="send-menu" role="menu" aria-label="Send options" data-testid="send-menu">
          <div class="send-menu-heading">While it runs</div>
          <button
            type="button"
            role="menuitemradio"
            aria-checked={followUpMode() === 'queue'}
            class="send-menu-item"
            onClick={() => (setFollowUpMode('queue'), close())}
            data-testid="send-mode-queue"
          >
            <span class="send-menu-label">After turn</span>
            <span class="send-menu-desc">Sent when the agent finishes this turn</span>
            <span class="send-menu-key" aria-hidden>
              {modKey()}↵ {modeLabel('steer')}
            </span>
          </button>
          <button
            type="button"
            role="menuitemradio"
            aria-checked={followUpMode() === 'steer'}
            class="send-menu-item"
            onClick={() => (setFollowUpMode('steer'), close())}
            data-testid="send-mode-steer"
          >
            <span class="send-menu-label">Next step</span>
            <span class="send-menu-desc">Sent after the agent’s current step</span>
            <span class="send-menu-key" aria-hidden>
              {modKey()}↵ {modeLabel('queue')}
            </span>
          </button>
          <Show when={props.running && props.onSendNow && !props.note}>
            <div class="send-menu-sep" role="separator" />
            <button type="button" role="menuitem" class="send-menu-item send-menu-now" onClick={() => (close(), props.onSendNow!())} data-testid="send-now">
              <Zap size={15} />
              <span class="send-menu-label">Send now (stops run)</span>
            </button>
          </Show>
          <Show when={props.onToggleNote}>
            <div class="send-menu-sep" role="separator" />
            <button
              type="button"
              role="menuitemcheckbox"
              aria-checked={!!props.note}
              class="send-menu-item send-menu-note"
              onClick={() => (close(), props.onToggleNote!())}
              data-testid="send-as-note"
            >
              <StickyNote size={15} />
              <span class="send-menu-label">Send as note</span>
              <span class="send-menu-desc">Only you see it · the agent never reads it</span>
            </button>
          </Show>
        </div>
      </Show>
      <div class="send-btn-main">
        <Show when={props.options !== false && (!(props.running && props.empty) || !!props.onToggleNote)}>
          <button
            type="button"
            class="send-btn-chevron"
            aria-label="Send options"
            title="While it runs: after this turn, or at the next step"
            aria-haspopup="menu"
            aria-expanded={menu()}
            onClick={() => setMenu(!menu())}
            data-testid="send-chevron"
          >
            <ChevronUp size={13} />
          </button>
        </Show>
        {/* The wrapper keeps the tip alive on the disabled state (Z-116: tooltips work on disabled controls). */}
        <Tooltip label={sendTip()}>
          {props.running && props.empty ? (
            <button type="button" class="tl-composer-send tl-composer-stop" aria-label="Stop" onClick={() => props.onStop()} disabled={props.disabled} data-testid="send-stop">
              <Square size={12} fill="currentColor" stroke-width={0} />
            </button>
          ) : (
            <button
              type="button"
              class="tl-composer-send"
              aria-label={props.note ? 'Save note' : 'Send'}
              data-note={props.note ? '' : undefined}
              onClick={() => props.onSend()}
              disabled={props.disabled || props.empty || props.blocked}
              data-testid="send-send"
            >
              <Show when={props.note} fallback={<ArrowUp size={17} stroke-width={2.4} />}>
                <StickyNote size={16} stroke-width={2.2} />
              </Show>
            </button>
          )}
        </Tooltip>
        <Show when={props.running && !props.empty}>
          <Tooltip label="Stop this run">
            <button type="button" class="tl-composer-send tl-composer-stop" aria-label="Stop" onClick={() => props.onStop()} disabled={props.disabled} data-testid="send-stop">
              <Square size={12} fill="currentColor" stroke-width={0} />
            </button>
          </Tooltip>
        </Show>
      </div>
    </div>
  );
}
