/** [running-tab] a finished run's transcript by record id (`subagents.transcript` items → messages). */
import { describe, expect, it } from 'vitest';
import { messagesFromTranscript } from './SubagentTab';

describe('messagesFromTranscript', () => {
  it('task / text / tool items become the transcript column messages', () => {
    const msgs = messagesFromTranscript({
      items: [
        { kind: 'task', text: 'Map the topics' },
        { kind: 'tool', name: 'bash', arg: 'ls src', output: 'a.rs', isError: false, running: false },
        { kind: 'tool', name: 'read', arg: 'x.rs', output: '', isError: false, running: true },
        { kind: 'text', text: 'Done: 4 topics.' },
      ],
      report: 'Done: 4 topics.',
      status: 'completed',
    });
    expect(msgs.map((m) => m.role)).toEqual(['user', 'assistant', 'toolResult', 'assistant', 'assistant']);
    expect(msgs[2]).toMatchObject({ role: 'toolResult', toolCallId: 'rec-1', content: 'a.rs' });
    // The report is not added twice when the last text already says it.
    expect(msgs.filter((m) => JSON.stringify(m.content).includes('Done: 4 topics.'))).toHaveLength(1);
    expect(messagesFromTranscript(null)).toEqual([]);
    expect(messagesFromTranscript({ items: [], report: 'R' }).map((m) => m.role)).toEqual(['assistant']);
  });
});
