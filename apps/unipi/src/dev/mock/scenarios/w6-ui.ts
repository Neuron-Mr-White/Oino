/**
 * w6-ui capture fixtures (development only): `w6-approvals` — three chats with a pending approval (untrusted project
 * → no "Always allow", a bash call with the "(legacy call)" fallback summary, an ask_user approval) — and
 * `w6-ask-loading` — an ask_user call whose question dialog has not opened yet ("Opening the question…").
 */
import type { MockScenario } from '../host';
import { genSession } from '../gen';

const HOST = 'pc-coffee';
const CWD = '/home/you/projects/unipi';

function chat(id: string, events: Array<Record<string, unknown>>) {
  const session = genSession(id, { seed: 8, turns: 1, toolsPerTurn: 0, cwd: CWD });
  session.events = events.map((event, i) => ({ at: i, dto: { session: id, seq: i + 1, event } })) as never;
  session.cmd = { approval_response: () => undefined, set_autonomy: () => undefined };
  return session;
}

const approval = (id: string, tool: string, args: Record<string, unknown>, extra: Record<string, unknown> = {}) => ({
  type: 'approval_request', requestId: `ap-${id}`, sessionId: id, tool, arguments: args, riskLevel: 'high', ceiling: 'medium', reason: 'This command needs Autonomy High or your approval.', ...extra,
});

export const w6Approvals: MockScenario = {
  name: 'w6-approvals',
  hosts: [{
    id: HOST, name: 'coffee', conn: { state: 'connected' },
    sessions: {
      a1: chat('a1', [approval('a1', 'bash', { command: 'git push origin feature/autonomy', summary: 'Push the reviewed branch' }, { alwaysAvailable: false })]),
      a2: chat('a2', [approval('a2', 'bash', { command: 'echo APPROVED-RAN-$((6*7))', summary: '(legacy call)' })]),
      a3: chat('a3', [approval('a3', 'ask_user', { questions: [{ question: 'Which colour should the badge be?', header: 'Colour', options: [{ label: 'Blue' }, { label: 'Red' }, { label: 'Green' }] }] }, { riskLevel: null, reason: 'needs interactive permission approval' })]),
    },
  }],
  setup() { location.hash = '#/s/pc-coffee/a1'; },
};

const ask = { type: 'toolCall', id: 'call_ask', name: 'ask_user', arguments: { questions: [{ question: 'Which colour should the badge be?', header: 'Colour', options: [{ label: 'Blue', description: 'Calm' }, { label: 'Red', description: 'Loud' }] }] } };
const meta = { api: 'anthropic-messages', provider: 'anthropic', model: 'claude-sonnet-4-5' };
export const w6AskLoading: MockScenario = {
  name: 'w6-ask-loading',
  hosts: [{
    id: HOST, name: 'coffee', conn: { state: 'connected' },
    sessions: {
      s1: chat('s1', [
        { type: 'agent_start' },
        { type: 'message_end', message: { role: 'assistant', content: [ask], ...meta, stopReason: 'toolUse', timestamp: Date.now() } },
        { type: 'tool_execution_start', toolCallId: ask.id, toolName: 'ask_user', args: ask.arguments },
      ]),
    },
  }],
  setup() { location.hash = '#/s/pc-coffee/s1'; },
};
