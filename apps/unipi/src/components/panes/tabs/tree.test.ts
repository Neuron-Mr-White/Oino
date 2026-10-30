/**
 * The session tree tab's pure logic (WP-A10, Z-18): the flattening (pi's
 * `/tree` layout) and the five pi filters, ported with the TreeSheet tests
 * from `SessionMenus.test.tsx` (the sheet itself is gone — the tree is a
 * side-pane tab now), plus the `get_tree` reply → nodes build.
 */
import { describe, expect, it } from 'vitest';
import { buildTreeNodes, filterTree, flattenTree, kindOfEntry, TREE_FILTERS, TREE_MAX_DEPTH, type TreeNode } from './tree';
import type { SessionEntry } from '../../../lib/session/types';

describe('flattenTree (UNI-186, pi /tree layout)', () => {
  function node(id: string, parentId: string | null, current = false): TreeNode {
    return { id, parentId, kind: 'message', preview: id, onPath: false, current };
  }
  const view = (rows: ReturnType<typeof flattenTree>) => rows.map((r) => `${r.cells.map((c) => c || ' ').join('')}${r.node.id}`);

  it('a linear chain of messages stays flat', () => {
    const rows = flattenTree([node('a', null), node('b', 'a'), node('c', 'b'), node('d', 'c')]);
    expect(rows.map((r) => r.depth)).toEqual([0, 0, 0, 0]);
  });

  it('a fork steps in with connectors and the first generation after it steps in again', () => {
    const rows = flattenTree([node('a', null), node('b', 'a'), node('c', 'a'), node('d', 'b'), node('e', 'd')]);
    expect(view(rows)).toEqual(['a', '├b', '│ d', '│ e', '└c']);
  });

  it('the branch holding the current leaf comes first among siblings', () => {
    const rows = flattenTree([node('a', null), node('b', 'a'), node('c', 'a'), node('d', 'c', true)]);
    expect(rows.map((r) => r.node.id)).toEqual(['a', 'c', 'd', 'b']);
    expect(view(rows)[1]).toBe('├c');
  });

  it('several roots hang off a virtual root and their chains indent once', () => {
    const rows = flattenTree([node('r1', null), node('x', 'r1'), node('r2', null), node('y', 'r2')]);
    expect(rows.map((r) => [r.node.id, r.depth])).toEqual([
      ['r1', 0],
      ['x', 1],
      ['r2', 0],
      ['y', 1],
    ]);
  });

  it('survives a malformed cycle without hanging', () => {
    expect(() => flattenTree([node('a', 'b'), node('b', 'a')])).not.toThrow();
  });
});

describe('filterTree (UNI-194, pi /tree filters)', () => {
  const n = (id: string, parentId: string | null, kind: string, extra: Partial<TreeNode> = {}): TreeNode => ({ id, parentId, kind, preview: id, onPath: false, current: false, ...extra });
  const nodes = [
    n('u1', null, 'user'),
    n('a1', 'u1', 'assistant', { preview: '' }),
    n('t1', 'a1', 'toolResult'),
    n('m1', 't1', 'model_change'),
    n('a2', 'm1', 'assistant', { label: 'checkpoint' }),
    n('u2', 'a2', 'user', { current: true }),
  ];
  const ids = (list: TreeNode[]) => list.map((x) => `${x.id}<${x.parentId ?? '-'}`);

  it('default hides settings entries and text-less assistant rows, re-parenting', () => {
    expect(ids(filterTree(nodes, 'default'))).toEqual(['u1<-', 't1<u1', 'a2<t1', 'u2<a2']);
  });
  it('no-tools also hides tool results', () => {
    expect(ids(filterTree(nodes, 'no-tools'))).toEqual(['u1<-', 'a2<u1', 'u2<a2']);
  });
  it('user-only keeps user messages (and the current leaf)', () => {
    expect(ids(filterTree(nodes, 'user-only'))).toEqual(['u1<-', 'u2<u1']);
  });
  it('labeled-only keeps labeled entries plus the current leaf', () => {
    expect(ids(filterTree(nodes, 'labeled-only'))).toEqual(['a2<-', 'u2<a2']);
  });
  it('all shows everything', () => {
    expect(filterTree(nodes, 'all')).toHaveLength(6);
  });
  it('offers pi\'s five filters in order', () => {
    expect(TREE_FILTERS.map((f) => f.id)).toEqual(['default', 'no-tools', 'user-only', 'labeled-only', 'all']);
  });
});

describe('buildTreeNodes (get_tree reply → rows)', () => {
  const entry = (id: string, parentId: string | null, extra: Partial<SessionEntry> = {}): SessionEntry => ({ type: 'message', id, parentId, ...extra }) as SessionEntry;

  it('walks the reply pre-order and marks the leaf and its branch', () => {
    const tree = [
      {
        entry: entry('u1', null, { message: { role: 'user', content: [{ type: 'text', text: 'fix the test' }] } }),
        children: [
          {
            entry: entry('a1', 'u1', { message: { role: 'assistant', content: [{ type: 'text', text: 'on it' }] }, timestamp: '2026-01-01T00:00:00.000Z' }),
            children: [{ entry: entry('u2', 'a1', { message: { role: 'user', content: [{ type: 'text', text: 'continue' }] } }) }],
          },
          { entry: entry('b1', 'u1', { message: { role: 'assistant', content: [{ type: 'text', text: 'other branch' }] } }) },
        ],
      },
    ];
    const nodes = buildTreeNodes(tree as never, 'u2');
    expect(nodes.map((n) => n.id)).toEqual(['u1', 'a1', 'u2', 'b1']);
    expect(nodes.map((n) => n.onPath)).toEqual([true, true, true, false]);
    expect(nodes.map((n) => n.current)).toEqual([false, false, true, false]);
    expect(nodes[1]!.parentId).toBe('u1');
    expect(nodes[1]!.timestamp).toBe('2026-01-01T00:00:00.000Z');
    expect(nodes[0]!.preview).toBe('fix the test');
  });

  it('an orphan leaf leaves nothing on the path (a stale leafId)', () => {
    const nodes = buildTreeNodes([{ entry: entry('a', null) }] as never, 'gone');
    expect(nodes[0]!.onPath).toBe(false);
    expect(nodes[0]!.current).toBe(false);
  });

  it('labels and kinds come from the entry', () => {
    const nodes = buildTreeNodes(
      [
        { entry: { type: 'compaction', id: 'c1', parentId: null, summary: 'Compacted 40k tokens' } as SessionEntry, label: 'checkpoint' },
        { entry: { type: 'model_change', id: 'm1', parentId: 'c1', modelId: 'gpt-5' } as SessionEntry },
      ] as never,
      'm1',
    );
    expect(nodes[0]).toMatchObject({ kind: 'compaction', label: 'checkpoint', preview: 'Compacted 40k tokens' });
    expect(nodes[1]!.kind).toBe('model_change');
    expect(kindOfEntry({ type: 'message', id: 'x', message: { role: 'bashExecution' } } as SessionEntry)).toBe('bash');
  });

  it('depth compression keeps the last TREE_MAX_DEPTH cells', () => {
    expect(TREE_MAX_DEPTH).toBe(6);
  });
});
