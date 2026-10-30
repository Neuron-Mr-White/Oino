import { afterEach, describe, expect, it } from 'vitest';
import { buildFindIndex, locateMatch, parseSearchHits, turnFindTexts } from './index';
import { buildTurns } from '../session/turns';
import type { SessionEntry } from '../session/types';
import { closeFind, findState, openFind, setFindQuery, stepFind } from './index';

const msg = (id: string, role: string, text: string, parent: string | null): SessionEntry =>
  ({ id, parentId: parent, type: 'message', message: { role, content: [{ type: 'text', text }] } }) as unknown as SessionEntry;

function turnsOf(entries: SessionEntry[]) {
  return buildTurns({ entries, leafId: entries.at(-1)?.id ?? null, entriesRev: 1, liveTools: {}, run: { running: false, startedAt: undefined } });
}

afterEach(() => closeFind());

describe('buildFindIndex', () => {
  const entries = [
    msg('e1', 'user', 'Fix the failing test', null),
    msg('e2', 'assistant', 'The test in math.ts failed on add().', 'e1'),
    msg('e3', 'user', 'Why did the test fail?', 'e2'),
    msg('e4', 'assistant', 'The test expected 4 and got 3.', 'e3'),
  ];
  const turns = turnsOf(entries);

  it('counts matches per turn over the rendered texts, in document order', () => {
    const index = buildFindIndex(turns, 'test');
    expect(index.total).toBe(4);
    expect(index.perTurn).toEqual([
      { turnId: 'e1', count: 2 }, // user "test" + answer "test"
      { turnId: 'e3', count: 2 },
    ]);
  });

  it('is case-insensitive and trims the query; empty query matches nothing', () => {
    expect(buildFindIndex(turns, '  FAIL ').total).toBe(3); // failing · failed · fail
    expect(buildFindIndex(turns, '').total).toBe(0);
    expect(buildFindIndex(turns, 'zzz').perTurn).toEqual([]);
  });

  it('turnFindTexts lists the user and answer texts of a turn', () => {
    expect(turnFindTexts(turns[0]!)).toHaveLength(2);
    expect(turnFindTexts(turns[0]!)![0]!.text).toBe('Fix the failing test');
    expect(turnFindTexts(turns[0]!)![1]!.text).toBe('The test in math.ts failed on add().');
  });

  it('locateMatch maps a global index to its turn and occurrence', () => {
    const index = buildFindIndex(turns, 'test');
    expect(locateMatch(index, 0)).toEqual({ turnId: 'e1', occurrence: 0 });
    expect(locateMatch(index, 1)).toEqual({ turnId: 'e1', occurrence: 1 });
    expect(locateMatch(index, 2)).toEqual({ turnId: 'e3', occurrence: 0 });
    expect(locateMatch(index, 9)).toBeUndefined();
  });
});

describe('controller', () => {
  it('step wraps both ways and bumps the navigation version', () => {
    openFind('x');
    setFindQuery('test');
    const before = findState().navVersion;
    stepFind(3, 1);
    expect(findState().active).toBe(1);
    expect(findState().navVersion).toBeGreaterThan(before);
    stepFind(3, 1);
    stepFind(3, 1);
    expect(findState().active).toBe(0); // wrapped
    stepFind(3, -1);
    expect(findState().active).toBe(2);
  });
});

describe('parseSearchHits', () => {
  it('accepts the bridge hit shape and tolerates junk', () => {
    const hits = parseSearchHits([{ id: 'h1', role: 'assistant', snippet: 'The tally', marks: [[4, 9]], score: 1 }]);
    expect(hits).toEqual([{ id: 'h1', role: 'assistant', snippet: 'The tally', marks: [[4, 9]], score: 1 }]);
    expect(parseSearchHits({ hits: [] })).toBeNull();
    expect(parseSearchHits([{ junk: true }])).toEqual([]);
    expect(parseSearchHits(null)).toBeNull();
  });
});
