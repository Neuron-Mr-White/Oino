import { createSignal, Show, For, onMount } from 'solid-js';
import { Copy, ShieldAlert } from 'lucide-solid';
import { copyText } from '../../lib/board/share';
import type { SessionHandle } from '../../lib/session/store';
import type { ApprovalRequest, ApprovalDecision } from '../../lib/session/autonomy';
import './autonomy.css';

/** Host requests only: never infer consent from tool output or a transcript entry. */
export function ApprovalCard(props: { request: ApprovalRequest; session: SessionHandle; disabled?: boolean }) {
  const [pending, setPending] = createSignal(false);
  const [error, setError] = createSignal<string>();
  const [answered, setAnswered] = createSignal(false);
  const decide = async (decision: ApprovalDecision) => {
    if (pending() || answered() || props.disabled || props.session.state.conn !== 'live') return;
    setPending(true);
    setError(undefined);
    try {
      await props.session.cmd({ type: 'approval_response', requestId: props.request.requestId, decision });
      setAnswered(true);
    } catch (e) { setError(e instanceof Error ? e.message : 'Could not send approval. The command remains blocked.'); }
    finally { setPending(false); }
  };
  const args = () => props.request.arguments;
  const command = () => typeof args().command === 'string' ? String(args().command) : JSON.stringify(args(), null, 2);
  let root: HTMLElement | undefined;
  onMount(() => root?.focus({ preventScroll: true }));
  return <section ref={root} tabindex="-1" class="dlg dlg-approval autonomy-approval" aria-label="Command approval" data-testid="approval-card" aria-busy={pending()}
    on:keydown={(event) => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); } }}>
    <div class="dlg-head"><ShieldAlert size={16} class="dlg-head-icon" /><span class="dlg-eyebrow">Approval needed</span><span class="ap-origin">{props.request.tool}</span>
      <Show when={props.request.riskLevel}><span class="autonomy-risk" data-risk={props.request.riskLevel}>{props.request.riskLevel}</span></Show>
    </div>
    <Show when={typeof args().summary === 'string'}><p class="dlg-question">{String(args().summary)}</p></Show>
    <div class="autonomy-command"><pre class="ap-cmd" tabIndex={0}>{command()}</pre><button type="button" class="dock-queue-icon" aria-label="Copy command" title="Copy command" onClick={() => void copyText(command())}><Copy size={15} /></button></div>
    <p class="ap-hint">Autonomy: {props.request.ceiling} · {props.request.reason}</p>
    <Show when={!answered()} fallback={<p role="status">Response sent</p>}>
      <div class="ap-options autonomy-approval-actions">
        <For each={['allow_once', 'always', 'deny'] as const}>{(decision) =>
          <button type="button" class={`dlg-btn ap-btn ${decision === 'allow_once' ? 'primary' : decision === 'always' ? 'ghost' : 'danger ghost'}`} disabled={props.disabled || pending() || props.session.state.conn !== 'live'}
            title={decision === 'always' ? 'Allow this exact command in interactive chats in this project only' : undefined}
            onClick={() => void decide(decision)}>{decision === 'allow_once' ? 'Allow once' : decision === 'always' ? 'Always allow' : 'Deny'}</button>
        }</For>
      </div>
    </Show>
    <Show when={error()}><p role="alert">{error()}</p></Show>
  </section>;
}
