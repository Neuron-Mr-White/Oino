import { createSignal, Show } from 'solid-js';
import { SheetFrame } from '../phone/Sheet';
import type { Blocker } from './api';
import type { SuperagentState } from './state';

/** What the owner is being asked to switch on, and what runs afterwards. */
export type BlockedRequest = {
  projectId: string;
  name: string;
  /** The workspace folder (register needs it). */
  path?: string | null;
  blocker: Blocker;
  /** Ready tickets that start by themselves once Auto is on. */
  ready?: number;
  /** Runs after the enable succeeded (a start retry); gets the project id the host returned. Resolves to an error text, or null on success. */
  retry?: (projectId: string) => Promise<string | null>;
  /** How many tickets the retry starts, for the sentence under the explanation. */
  startCount?: number;
};

const plural = (n: number, one: string, many: string) => (n === 1 ? one : many);

export function blockedTitle(r: BlockedRequest): string {
  switch (r.blocker.action.kind) {
    case 'register': return `Set up Kanboard for ${r.name}?`;
    case 'unarchive': return `Unarchive ${r.name}?`;
    case 'enable_project_auto': return `Turn on Auto for ${r.name}?`;
    case 'enable_auto_start': return 'Turn on Auto-start?';
  }
}

/** The sentence after the host's own explanation: what happens once the owner confirms. */
export function blockedNext(r: BlockedRequest): string {
  const n = r.startCount ?? 0;
  const start = n > 0 ? `Then Superagent starts ${n} ${plural(n, 'ticket', 'tickets')}.` : '';
  switch (r.blocker.action.kind) {
    case 'register': return start || 'Then you can add tickets and start work.';
    case 'unarchive': return start || 'Then it can take work again.';
    case 'enable_project_auto':
    case 'enable_auto_start': return start || 'Ready tickets will start automatically.';
  }
}

/**
 * The owner's yes/no before Superagent changes anything: explains the blocker,
 * performs the exact enable action on Confirm, then runs the retry. Cancel,
 * Escape, Back and the scrim do nothing. A centred dialog on the desktop, a sheet on the phone.
 */
export function DispatchBlockedModal(props: { sa: SuperagentState; request: BlockedRequest; onClose: () => void }) {
  const r = () => props.request;
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal('');
  // The enable already went through when only the retry failed: confirming again must not repeat it.
  let enabledId: string | null | undefined;
  const close = () => { if (!busy()) props.onClose(); };
  async function confirm() {
    if (busy()) return;
    setBusy(true); setError('');
    try {
      if (enabledId === undefined) {
        const action = r().blocker.action.kind;
        const result = await props.sa.enable({ action, ...(action === 'enable_auto_start' ? {} : action === 'register' ? { path: r().path ?? undefined } : { projectId: r().projectId }) });
        if (!result.ok) { setError(result.message); return; }
        enabledId = result.projectId ?? r().projectId;
      }
      const failed = await r().retry?.(enabledId ?? r().projectId);
      if (failed) { setError(failed); return; }
      props.onClose();
    } finally { setBusy(false); }
  }
  return <SheetFrame open onClose={close} label={blockedTitle(r())} title={blockedTitle(r())} level={2} detents={['content']} testId="sa-blocked"
    footer={<div class="sa-blocked-foot">
      <button type="button" class="sa-btn" data-block disabled={busy()} onClick={close}>Cancel</button>
      <button type="button" class="sa-btn" data-tone="primary" data-block aria-busy={busy()} disabled={busy()} onClick={() => void confirm()}>{busy() ? 'Working…' : r().blocker.action.label}</button>
    </div>}>
    <div class="sa-blocked" data-testid="sa-blocked-body">
      <p class="sa-blocked-why">{r().blocker.message}</p>
      <Show when={r().blocker.action.kind === 'register' && r().path}><code class="sa-blocked-path" title={r().path ?? undefined}>{r().path}</code></Show>
      <p class="sa-blocked-next">{blockedNext(r())}</p>
      <Show when={error()}><p class="sa-message sa-blocked-error" role="alert">{error()}</p></Show>
    </div>
  </SheetFrame>;
}
