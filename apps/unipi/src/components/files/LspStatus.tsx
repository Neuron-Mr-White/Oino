/**
 * The editor's language-server chip (WP-A21): shows the pooled
 * connection's status for the active tab — `off` (no server / degraded
 * plain editing), `connecting`, `ready` (server name), `degraded`,
 * `taken` (another device took the attach — clicking offers to take it
 * back). Small title tooltip carries the detail.
 */
import { Show } from 'solid-js';
import { CircleAlert, Languages, Loader2, RefreshCw } from 'lucide-solid';
import type { PooledLsp } from '../../lib/lsp/pool';

const LABEL: Record<string, string> = {
  off: 'LSP off',
  connecting: 'LSP…',
  ready: '',
  degraded: 'LSP degraded',
  taken: 'LSP taken',
};

export function LspStatus(props: { pooled: () => PooledLsp | null | undefined; onTakeBack?: () => void }) {
  const pooled = () => props.pooled();
  const status = () => pooled()?.status() ?? 'off';
  const title = () => {
    const p = pooled();
    switch (status()) {
      case 'ready':
        return `Language server: ${p?.serverName} — diagnostics, hover, completion, go-to-definition on`;
      case 'connecting':
        return 'Connecting to the language server…';
      case 'degraded':
        return 'The language server stopped — editing continues without it';
      case 'taken':
        return 'Language features moved to another device — click to take them back';
      default:
        return 'No language server for this file';
    }
  };
  return (
    <button
      type="button"
      class={`ftree-tool feditor-lsp ${status()}`}
      title={title()}
      data-testid="files-editor-lsp"
      data-status={status()}
      onClick={() => {
        if (status() === 'taken') props.onTakeBack?.();
      }}
    >
      <Show when={status() === 'connecting'}>
        <Loader2 size={13} class="spin" />
      </Show>
      <Show when={status() === 'ready'}>
        <Languages size={13} />
        <span class="feditor-lsp-name">{pooled()?.serverName}</span>
      </Show>
      <Show when={status() === 'degraded' || status() === 'off'}>
        <CircleAlert size={13} />
        <span class="feditor-lsp-name">{LABEL[status()]}</span>
      </Show>
      <Show when={status() === 'taken'}>
        <RefreshCw size={13} />
        <span class="feditor-lsp-name">{LABEL.taken}</span>
      </Show>
    </button>
  );
}
