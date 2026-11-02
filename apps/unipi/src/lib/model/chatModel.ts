/**
 * E-19: a model pick inside a chat belongs to that chat. The host's chat-only path sets the
 * session's model without writing the PC default; "Use at startup" (and Settings) are the only
 * writers of `defaultProvider` / `defaultModel`.
 */
import type { SessionHandle } from '../session/store';

export async function setChatModel(s: SessionHandle, provider: string, modelId: string): Promise<void> {
  await s.cmd({ type: 'set_model', provider, modelId, persist: false });
}
