import { describe, expect, it } from 'vitest';
import { allCommands, rankCommands, registerCommands, type Command } from './shell';

const cmd = (id: string, title: string, group = 'G', keywords?: string): Command => ({ id, title, group, keywords, run: () => {} });

describe('palette commands', () => {
  it('registerCommands replaces by id, removes on cleanup, honours when()', () => {
    const offA = registerCommands([cmd('t.a', 'Alpha'), { ...cmd('t.hidden', 'Hidden'), when: () => false }]);
    const offB = registerCommands([cmd('t.a', 'Alpha 2')]);
    const titles = allCommands().map((c) => c.title);
    expect(titles).toContain('Alpha 2');
    expect(titles).not.toContain('Alpha');
    expect(titles).not.toContain('Hidden');
    offB();
    offA();
    expect(allCommands().some((c) => c.id.startsWith('t.'))).toBe(false);
  });

  it('rankCommands: prefix > word start > substring > keyword > subsequence; all words must match', () => {
    const list = [cmd('1', 'Go to Files', 'Go to'), cmd('2', 'Filter tasks', 'Board'), cmd('3', 'Display options', 'Board', 'lanes columns'), cmd('4', 'New task', 'Board')];
    expect(rankCommands(list, 'fil').map((c) => c.id)).toEqual(['2', '1']);
    expect(rankCommands(list, 'columns').map((c) => c.id)).toEqual(['3']);
    expect(rankCommands(list, 'board task').map((c) => c.id)).toEqual(['2', '4']); // tie → registration order
    expect(rankCommands(list, 'nwtsk').map((c) => c.id)).toEqual(['4']);
    expect(rankCommands(list, 'zzz')).toEqual([]);
    expect(rankCommands(list, '  ')).toEqual(list);
  });
});
