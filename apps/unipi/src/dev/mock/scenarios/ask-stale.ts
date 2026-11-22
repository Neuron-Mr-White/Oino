/**
 * ASK-STALE capture fixture (development only): four chats whose `ask_user` call is over —
 * interrupted by a restart, cancelled, answered, and left behind by a live run that moved on.
 * None of them may show an interactive question card.
 */
import type { MockScenario } from '../host';
import { genSession } from '../gen';

const HOST = 'pc-coffee';
const meta = { api: 'anthropic-messages', provider: 'anthropic', model: 'claude-sonnet-4-5' };
const args = { questions: [{ question: 'Which database should the migration target?', header: 'Database', options: [{ label: 'Postgres', description: 'Server' }, { label: 'SQLite', description: 'Embedded' }] }] };
const call = (id: string) => ({ type: 'toolCall', id, name: 'ask_user', arguments: args });
const assistant = (content: unknown[], stopReason = 'toolUse') => ({ type: 'message_end', message: { role: 'assistant', content, ...meta, stopReason, timestamp: Date.now() } });
const result = (id: string, text: string, isError: boolean, details?: unknown) => ({ type: 'message_end', message: { role: 'toolResult', toolCallId: id, toolName: 'ask_user', content: [{ type: 'text', text }], isError, ...(details ? { details } : {}), timestamp: Date.now() } });

function chat(id: string, events: Array<Record<string, unknown>>) {
  const session = genSession(id, { seed: 8, turns: 1, toolsPerTurn: 0, cwd: '/home/you/projects/unipi' });
  session.events = events.map((event, i) => ({ at: i, dto: { session: id, seq: i + 1, event } })) as never;
  return session;
}

export const askStale: MockScenario = {
  name: 'ask-stale',
  hosts: [{
    id: HOST, name: 'coffee', conn: { state: 'connected' },
    sessions: {
      interrupted: chat('interrupted', [{ type: 'agent_start' }, assistant([call('c1')]), result('c1', 'Tool execution was interrupted before completion; the outcome is unknown. The effect may or may not have happened. Do not retry blindly.', true), { type: 'agent_end', messages: [] }]),
      cancelled: chat('cancelled', [{ type: 'agent_start' }, assistant([call('c2')]), result('c2', 'The user cancelled the questions and stopped the turn.', false, { outcome: 'cancelled', questions: args.questions }), { type: 'agent_end', messages: [] }]),
      answered: chat('answered', [{ type: 'agent_start' }, assistant([call('c3')]), result('c3', 'User answered your questions:', false, { outcome: 'answered', questions: args.questions, answers: [{ selected: ['SQLite'], skipped: false }] }), { type: 'agent_end', messages: [] }]),
      movedon: chat('movedon', [{ type: 'agent_start' }, assistant([call('c4')]), assistant([{ type: 'text', text: 'Moving on with the default.' }], 'stop')]),
    },
  }],
  setup() { location.hash = '#/s/pc-coffee/interrupted'; },
};
