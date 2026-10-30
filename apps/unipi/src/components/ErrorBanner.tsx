import { Show, createSignal } from 'solid-js';
import type { AppError } from '@unipi/protocol';
import { AlertCircle, Copy } from 'lucide-solid';
export function ErrorBanner(props: { error: AppError; retry?: () => void; dismiss?: () => void }) {
  const [copied, setCopied] = createSignal(false);
  return <aside role="alert" class="rounded-card border border-line bg-surface-2 p-4 text-sm">
    <div class="flex items-center gap-2"><AlertCircle size={18} class="text-wm-red" />{props.error.message}</div>
    <Show when={props.error.hint}><p class="mt-2 text-text-muted">{props.error.hint}</p></Show>
    <div class="mt-2 flex gap-3">
      <Show when={props.error.retryable}><button class="text-shell px-2" onClick={() => props.retry?.()}>Retry</button></Show>
      <Show when={props.error.detail !== undefined}><button class="flex items-center gap-2 text-text-muted" onClick={async () => { try { await navigator.clipboard.writeText(typeof props.error.detail === 'string' ? props.error.detail : JSON.stringify(props.error.detail, null, 2)); setCopied(true); } catch { setCopied(false); } }}><Copy size={16} />{copied() ? 'Copied' : 'Copy details'}</button></Show>
      <Show when={props.dismiss}><button class="ml-auto text-text-muted" onClick={props.dismiss}>Dismiss</button></Show>
    </div>
  </aside>;
}
