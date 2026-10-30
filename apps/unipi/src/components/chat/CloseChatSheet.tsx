/**
 * "Close chat" (UNI-260): chat ⋯ → Close chat…, with a confirm and two ways:
 * - Stop pi and close its tab — the host stops pi gracefully (SIGTERM: pi
 *   saves and quits as with /quit) and closes the herdr tab / tmux window
 *   (only the pane when the tab holds others);
 * - Just hide from list — nothing on the PC changes; the chat is hidden on
 *   this device for this PC until a new pi runs in that pane (Chat list →
 *   "Show hidden" brings it back).
 */
import { createSignal, Show } from 'solid-js';
import { EyeOff, Power } from 'lucide-solid';
import { BoardSheet } from '../board/BoardSheet';
import { errorWords, stopChat } from '../../lib/chat/newChat';
import type { ChatTarget } from '../../lib/chat/list';

export type CloseChoice = 'stop' | 'hide';

export function CloseChatSheet(props: {
  target: ChatTarget | null;
  pcName: string;
  running?: boolean;
  onClose: () => void;
  /** After the host stopped pi (and closed its tab), or after hiding. */
  onDone: (choice: CloseChoice) => void;
}) {
  const [choice, setChoice] = createSignal<CloseChoice>('stop');
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<{ text: string; hint?: string } | undefined>();
  const tmux = () => props.target?.mux === 'tmux';
  const place = () => (tmux() ? 'window' : 'tab');

  const confirm = async () => {
    const t = props.target;
    if (!t || t.pid === null || busy()) return;
    setError(undefined);
    if (choice() === 'hide') {
      props.onDone('hide');
      return;
    }
    setBusy(true);
    const res = await stopChat(t.hostId, t.pid, true);
    setBusy(false);
    if (!res.ok) {
      setError(errorWords(res.error));
      return;
    }
    props.onDone('stop');
  };
  const close = () => {
    if (busy()) return;
    setError(undefined);
    setChoice('stop');
    props.onClose();
  };

  return (
    <BoardSheet
      open={props.target !== null}
      onClose={close}
      label="Close this chat?"
      testId="close-chat-sheet"
      footer={
        <div class="dialog-actions">
          <button type="button" class="btn btn-secondary" onClick={close} disabled={busy()}>
            Cancel
          </button>
          <button type="button" class={choice() === 'stop' ? 'btn btn-danger' : 'btn btn-primary'} data-testid="close-chat-confirm" disabled={busy()} onClick={() => void confirm()}>
            {busy() ? 'Stopping pi…' : choice() === 'stop' ? `Stop pi and close ${place()}` : 'Hide from list'}
          </button>
        </div>
      }
    >
      <p class="new-chat-lead">
        <b>{props.target?.title}</b> on {props.pcName || 'this PC'}
        <Show when={props.target?.workspaceLabel}> · {props.target!.workspaceLabel}</Show>
      </p>
      <div class="new-chat-where" role="radiogroup" aria-label="How to close">
        <button type="button" role="radio" class="new-chat-where-row" aria-checked={choice() === 'stop'} disabled={busy()} onClick={() => setChoice('stop')}>
          <span class="new-chat-where-icon danger">
            <Power size={16} />
          </span>
          <span class="min-w-0 flex-1 text-left">
            <span class="new-chat-where-title">Stop pi and close {place()}</span>
            <span class="new-chat-where-sub wrap">
              pi quits like /quit (the session is saved; resume it later) and its {tmux() ? 'tmux window' : 'herdr tab'} closes.
            </span>
          </span>
        </button>
        <button type="button" role="radio" class="new-chat-where-row" aria-checked={choice() === 'hide'} disabled={busy()} onClick={() => setChoice('hide')}>
          <span class="new-chat-where-icon">
            <EyeOff size={16} />
          </span>
          <span class="min-w-0 flex-1 text-left">
            <span class="new-chat-where-title">Just hide from list</span>
            <span class="new-chat-where-sub wrap">pi keeps running on the PC; only this device stops listing it.</span>
          </span>
        </button>
      </div>
      <Show when={choice() === 'stop' && props.running}>
        <p class="new-chat-note warn">pi is working right now. Stopping it ends the current run.</p>
      </Show>
      <Show when={error()}>
        {(e) => (
          <div class="new-chat-note warn" role="alert">
            <b>Couldn't close it.</b> {e().text}
            <Show when={e().hint}>
              <span class="block mt-1 text-text-muted">{e().hint}</span>
            </Show>
          </div>
        )}
      </Show>
    </BoardSheet>
  );
}
