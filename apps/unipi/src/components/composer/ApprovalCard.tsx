import { createSignal, Show, For } from 'solid-js';
import { Copy, ShieldAlert } from 'lucide-solid';
import { copyText } from '../../lib/board/share';
import type { SessionHandle } from '../../lib/session/store';
import { approvalView } from '../../lib/approvalText';
import type { ApprovalRequest, ApprovalDecision } from '../../lib/session/autonomy';
import './autonomy.css';

/** Host requests only: never infer consent from tool output or a transcript entry. */
export function ApprovalCard(props: { request: ApprovalRequest; session?: SessionHandle; onDecision?: (decision: ApprovalDecision) => Promise<void>; disabled?: boolean }) {
  const [pending, setPending] = createSignal(false);
  const [error, setError] = createSignal<string>();
  const [answered, setAnswered] = createSignal(false);
  const decide = async (decision: ApprovalDecision) => {
    if (pending() || answered() || props.disabled || (!props.onDecision && props.session?.state.conn !== 'live')) return;
    setPending(true);
    setError(undefined);
    try {
      if (props.onDecision) await props.onDecision(decision);
      else await props.session!.cmd({ type: 'approval_response', requestId: props.request.requestId, decision });
      setAnswered(true);
    } catch (e) { setError(e instanceof Error ? e.message : 'Could not send approval. The command remains blocked.'); }
    finally { setPending(false); }
  };
  const view = () => approvalView(props.request.tool, props.request.arguments);
  const command = () => view().detail;
  /** N5: "Always allow" only when the host can save the rule (older hosts omit the flag = offered). */
  const alwaysOffered = () => props.request.alwaysAvailable !== false;
  const decisions = () => (alwaysOffered() ? (['allow_once', 'always', 'deny'] as const) : (['allow_once', 'deny'] as const));
  return <section tabindex="-1" class="dlg dlg-approval autonomy-approval" aria-label="Command approval" data-testid="approval-card" aria-busy={pending()}
    on:keydown={(event) => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); } }}>
    <div class="dlg-head"><ShieldAlert size={16} class="dlg-head-icon" /><span class="dlg-eyebrow">Approval needed</span><span class="ap-origin">{props.request.tool}</span>
      <Show when={props.request.riskLevel}><span class="autonomy-risk" data-risk={props.request.riskLevel}>{props.request.riskLevel}</span></Show>
    </div>
    <Show when={view().title}><p class="dlg-question">{view().title}</p></Show>
    <div class="autonomy-command"><pre class="ap-cmd" data-kind={view().kind} tabIndex={0}>{command()}</pre><Show when={view().kind !== 'preview'}><button type="button" class="dock-queue-icon" aria-label="Copy command" title="Copy command" onClick={() => void copyText(command())}><Copy size={15} /></button></Show></div>
    <p class="ap-hint">Autonomy: {props.request.ceiling} · {props.request.reason}</p>
    <Show when={!answered()} fallback={<p role="status">Response sent</p>}>
      <div class="ap-options autonomy-approval-actions">
        <For each={decisions()}>{(decision) =>
          <button type="button" class={`dlg-btn ap-btn ${decision === 'allow_once' ? 'primary' : decision === 'always' ? 'ghost' : 'danger ghost'}`} disabled={props.disabled || pending() || (!props.onDecision && props.session?.state.conn !== 'live')}
            title={decision === 'always' ? 'Allow this exact command in interactive chats in this project only' : undefined}
            onClick={() => void decide(decision)}>{decision === 'allow_once' ? 'Allow once' : decision === 'always' ? 'Always allow' : 'Deny'}</button>
        }</For>
      </div>
      <Show when={!alwaysOffered()}>
        <p class="ap-hint" data-testid="approval-no-always">“Always allow” isn’t offered here: this project isn’t trusted (or this {view().kind === 'command' ? 'command' : 'call'} can’t be saved as a rule), so “Allow once” applies to this call only.</p>
      </Show>
    </Show>
    <Show when={error()}><p role="alert">{error()}</p></Show>
  </section>;
}
