/**
 * The send / stop control (WP-A11, Z-52 + Z-55): the button is **Stop**
 * while a run is going, **Send** otherwise. Its chevron menu is the only
 * send-related menu left (the v1 S/Q inline choice is gone): it switches
 * the global follow-up default (queue | steer) and keeps unipi's
 * "Send now (stops run)". Esc closes the menu and is consumed — it must
 * not stop the run (Z-55's popup rule).
 */
import { Show, createSignal, onCleanup } from 'solid-js';
import { ChevronUp, SendHorizonal, Square, Zap } from 'lucide-solid';
import { followUpMode, modeLabel, setFollowUpMode } from '../../lib/composer/sendMode';
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
}) {
  const [menu, setMenu] = createSignal(false);
  const close = () => setMenu(false);
  /** Z-116: a disabled Send still explains itself ("Type something to send"). */
  const sendTip = () => (props.running ? 'Stop this run' : props.empty ? 'Type something to send' : props.blocked ? (props.blockedReason ?? 'Send is held') : 'Send');

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
    <div class="send-btn" data-send-menu-root data-open={menu() ? '' : undefined} on:keydown={onKeyDown}>
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
            <span class="send-menu-label">Queue</span>
            <span class="send-menu-desc">Enter · send after this run ends</span>
            <span class="send-menu-key" aria-hidden>
              ⌘↵ {modeLabel('steer')}
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
            <span class="send-menu-label">Steer</span>
            <span class="send-menu-desc">Enter · deliver at the next tool boundary</span>
            <span class="send-menu-key" aria-hidden>
              ⌘↵ {modeLabel('queue')}
            </span>
          </button>
          <Show when={props.running && props.onSendNow}>
            <div class="send-menu-sep" role="separator" />
            <button type="button" role="menuitem" class="send-menu-item send-menu-now" onClick={() => (close(), props.onSendNow!())} data-testid="send-now">
              <Zap size={15} />
              <span class="send-menu-label">Send now (stops run)</span>
            </button>
          </Show>
        </div>
      </Show>
      <div class="send-btn-main">
        {/* The wrapper keeps the tip alive on the disabled state (Z-116: tooltips work on disabled controls). */}
        <Tooltip label={sendTip()}>
          {props.running ? (
            <button type="button" class="tl-composer-send tl-composer-stop" aria-label="Stop" onClick={() => props.onStop()} disabled={props.disabled} data-testid="send-stop">
              <Square size={15} fill="currentColor" />
            </button>
          ) : (
            <button
              type="button"
              class="tl-composer-send"
              aria-label="Send"
              onClick={() => props.onSend()}
              disabled={props.disabled || props.empty || props.blocked}
              data-testid="send-send"
            >
              <SendHorizonal size={17} />
            </button>
          )}
        </Tooltip>
        <button type="button" class="send-btn-chevron" aria-label="Send options" aria-haspopup="menu" aria-expanded={menu()} onClick={() => setMenu(!menu())} data-testid="send-chevron">
          <ChevronUp size={14} />
        </button>
      </div>
    </div>
  );
}
