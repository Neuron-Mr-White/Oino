import { createSignal, Show, For } from 'solid-js';
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
  return <section class="autonomy-approval" aria-label="Command approval" data-testid="approval-card" aria-busy={pending()}>
    <strong>Approval needed · {props.request.tool}</strong>
    <Show when={typeof args().summary === 'string'}><p>{String(args().summary)}</p></Show>
    <pre tabIndex={0}>{typeof args().command === 'string' ? String(args().command) : JSON.stringify(args(), null, 2)}</pre>
    <p>{props.request.riskLevel ? `Risk: ${props.request.riskLevel} · ` : ''}Autonomy: {props.request.ceiling}</p>
    <p>{props.request.reason}</p>
    <Show when={!answered()} fallback={<p role="status">Response sent</p>}>
      <div class="autonomy-approval-actions">
        <For each={['allow_once', 'always', 'deny'] as const}>{(decision) =>
          <button type="button" class="dock-btn" disabled={props.disabled || pending() || props.session.state.conn !== 'live'}
            title={decision === 'always' ? 'Allow this exact command in interactive chats in this project only' : undefined}
            onClick={() => void decide(decision)}>{decision === 'allow_once' ? 'Allow once' : decision === 'always' ? 'Always allow' : 'Deny'}</button>
        }</For>
      </div>
    </Show>
    <Show when={error()}><p role="alert">{error()}</p></Show>
  </section>;
}
