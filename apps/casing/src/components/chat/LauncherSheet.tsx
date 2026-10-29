/**
 * UNI-260: Settings → PCs → "pi launcher": the command a PC runs when the app
 * starts a new chat there (host `config.json` `chat.launcher`, default `pi`).
 */
import { createEffect, createSignal, on, Show } from 'solid-js';
import { BoardSheet } from '../board/BoardSheet';
import { chatConfig, errorWords, isOlderHost } from '../../lib/chat/newChat';

export function LauncherSheet(props: { pc: { id: string; name: string } | null; onClose: () => void }) {
  const [value, setValue] = createSignal('');
  const [initial, setInitial] = createSignal<string | undefined>();
  const [state, setState] = createSignal<'loading' | 'ready' | 'saving' | 'older' | 'error'>('loading');
  const [message, setMessage] = createSignal<string | undefined>();

  createEffect(
    on(
      () => props.pc?.id,
      async (id) => {
        if (!id) return;
        setState('loading');
        setMessage(undefined);
        const res = await chatConfig(id);
        if (props.pc?.id !== id) return;
        if (!res.ok) {
          setState(isOlderHost(res.error) ? 'older' : 'error');
          setMessage(errorWords(res.error).text);
          return;
        }
        const v = res.value.default ? '' : res.value.launcher;
        setValue(v);
        setInitial(v);
        setState('ready');
      },
    ),
  );

  const save = async () => {
    const pc = props.pc;
    if (!pc || state() !== 'ready') return;
    setState('saving');
    const res = await chatConfig(pc.id, value().trim());
    if (!res.ok) {
      setState('ready');
      setMessage(errorWords(res.error).text);
      return;
    }
    props.onClose();
  };

  return (
    <BoardSheet
      open={props.pc !== null}
      onClose={props.onClose}
      label="pi launcher"
      testId="launcher-sheet"
      footer={
        <div class="dialog-actions">
          <button type="button" class="btn btn-secondary" onClick={props.onClose}>
            Cancel
          </button>
          <button type="button" class="btn btn-primary" data-testid="launcher-save" disabled={state() !== 'ready' || value().trim() === (initial() ?? '').trim()} onClick={() => void save()}>
            {state() === 'saving' ? 'Saving…' : 'Save'}
          </button>
        </div>
      }
    >
      <p class="new-chat-lead">
        What {props.pc?.name ?? 'this PC'} runs in the new tab when you start a chat from the app. It's typed into a login shell there, so your PATH applies.
      </p>
      <Show when={state() === 'older'}>
        <p class="new-chat-note warn">This PC runs an older UniPi. Update UniPi on the PC to start chats from the app.</p>
      </Show>
      <Show when={state() !== 'older'}>
        <input
          type="text"
          class="files-search-input new-chat-input font-mono"
          aria-label="Launcher command"
          placeholder="pi"
          maxLength={256}
          value={value()}
          disabled={state() === 'loading' || state() === 'error'}
          onInput={(e) => setValue(e.currentTarget.value.replace(/[\r\n]/g, ' '))}
          onKeyDown={(e) => e.key === 'Enter' && void save()}
          autofocus
        />
        <p class="new-chat-hint">Empty = <span class="font-mono">pi</span>. Examples: <span class="font-mono">unipi</span>, <span class="font-mono">pi --model sonnet</span>.</p>
      </Show>
      <Show when={message() && state() !== 'older'}>
        <p class="new-chat-note warn" role="alert">{message()}</p>
      </Show>
    </BoardSheet>
  );
}
