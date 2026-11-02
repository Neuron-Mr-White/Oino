import { describe, expect, it } from 'vitest';
import { composeAnswers, extractQuestions } from './questions';

describe('extractQuestions (PH-21)', () => {
  it('finds questions, keeps wrapped ones whole, skips code and takes a list under a question as choices', () => {
    const reply = [
      'I looked at the repo.',
      '',
      'Which database should the new service use',
      'for its sessions?',
      '- Postgres',
      '- SQLite',
      '',
      '```',
      'is this code?',
      '```',
      'Should I also add tests? And bump it to v1.2 in the README?',
    ].join('\n');
    const qs = extractQuestions(reply);
    expect(qs.map((q) => q.text)).toEqual(['Which database should the new service use for its sessions?', 'Should I also add tests?', 'And bump it to v1.2 in the README?']);
    expect(qs[0]!.options).toEqual(['Postgres', 'SQLite']);
  });
  it('composes answers like /answer does', () => {
    const qs = [{ text: 'A?', options: [] }, { text: 'B?', options: [] }];
    expect(composeAnswers(qs, ['yes', ''])).toBe('1. > A?\n\nyes\n\n(Not answered: #2)');
    expect(composeAnswers(qs, ['', ''])).toBeUndefined();
  });
});
