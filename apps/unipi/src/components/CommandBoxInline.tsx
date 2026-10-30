/** A copyable command (the same look as Preview's `.preview-command`). */
import { createSignal, Show } from 'solid-js';
import { Copy } from 'lucide-solid';
import { copyText } from '../lib/board/share';
import '../screens/preview.css';

export function CommandBoxInline(props: { command: string }) {
  const [copied, setCopied] = createSignal(false);
  return (
    <div class="preview-command">
      <code>{props.command}</code>
      <button
        type="button"
        class="ghost-icon-button"
        aria-label="Copy command"
        onClick={async () => {
          setCopied(await copyText(props.command));
          setTimeout(() => setCopied(false), 1500);
        }}
      >
        <Show when={copied()} fallback={<Copy size={16} />}>
          <span class="preview-copied">Copied</span>
        </Show>
      </button>
    </div>
  );
}
