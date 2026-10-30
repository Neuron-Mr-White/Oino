/** NodeOp apply (ui-protocol §4.7), mirroring `oino_ui::patch::apply` (patch.rs). */
import { describe, expect, it } from 'vitest';
import { createStore, produce } from 'solid-js/store';
import { applyNodeOps, Tx, TreeOpError } from '../tree';
import type { NodeOp } from '../types';

type Holder = { root: Record<string, unknown> };
const run = (root: unknown, ops: NodeOp[]) => {
  const h = { root: structuredClone(root) } as Holder;
  applyNodeOps(h as unknown as Record<string, unknown>, 'root', ops);
  return h.root;
};

const layout = {
  t: 'Layout',
  children: [
    { t: 'Paragraph', key: 'head', text: 'h' },
    { t: 'Block', key: 'main', child: { t: 'List', items: [{ key: 'a', content: 'A' }, 'plain'] } },
    { t: 'Tabs', key: 'tabs', titles: [{ key: 'x', title: 'X' }], panes: { x: { t: 'Log', lines: ['1'] } } },
    { t: 'Gauge', ratio: 0.1 },
  ],
};

describe('paths', () => {
  it('resolves child keys, Block "child", Tabs panes and positional keys', () => {
    const out = run(layout, [
      { op: 'set', path: ['head'], props: { alignment: 'Center' } },
      { op: 'set', path: ['main', 'child'], props: { multi: true } },
      { op: 'append', path: ['tabs', 'x'], field: 'lines', items: ['2'] },
      { op: 'set', path: ['#3'], props: { ratio: 0.5 } },
    ]) as typeof layout & Record<string, never>;
    const c = out.children as unknown as Array<Record<string, any>>;
    expect(c[0]!.alignment).toBe('Center');
    expect(c[1]!.child.multi).toBe(true);
    expect(c[2]!.panes.x.lines).toEqual(['1', '2']);
    expect(c[3]!.ratio).toBe(0.5);
  });

  it('a missing path fails with a diagnostic', () => {
    expect(() => run(layout, [{ op: 'set', path: ['nope'], props: {} }])).toThrow(/missing path \["nope"\]/);
    expect(() => run(layout, [{ op: 'set', path: ['head', 'child'], props: {} }])).toThrow(TreeOpError);
  });
});

describe('ops', () => {
  it('replace at the root and below', () => {
    expect(run(layout, [{ op: 'replace', path: [], node: { t: 'Empty' } }])).toEqual({ t: 'Empty' });
    const out = run(layout, [{ op: 'replace', path: ['head'], node: { t: 'Spinner', key: 'head' } }]) as { children: Array<{ t: string }> };
    expect(out.children[0]!.t).toBe('Spinner');
  });

  it('set merges shallowly; null unsets; t cannot change', () => {
    const out = run({ t: 'Paragraph', text: 'a', alignment: 'Left' }, [{ op: 'set', path: [], props: { alignment: null, scroll: [1, 0] } }]);
    expect(out).toEqual({ t: 'Paragraph', text: 'a', scroll: [1, 0] });
    expect(() => run({ t: 'Paragraph', text: 'a' }, [{ op: 'set', path: [], props: { t: 'List' } }])).toThrow(/"t" cannot be set/);
  });

  it('splice children / items, bounds-checked', () => {
    const out = run(layout, [{ op: 'splice', path: [], field: 'children', index: 1, remove: 2, insert: [{ t: 'Spinner', key: 's' }] }]) as {
      children: Array<{ key?: string }>;
    };
    expect(out.children.map((c) => c.key)).toEqual(['head', 's', undefined]);
    expect(() => run(layout, [{ op: 'splice', path: [], field: 'children', index: 3, remove: 2, insert: [] }])).toThrow(/out of bounds/);
    expect(() => run(layout, [{ op: 'splice', path: [], field: 'items', index: 0, remove: 0, insert: [] }])).toThrow(/does not exist on Layout/);
  });

  it('splice panes keeps them sorted by key (BTreeMap order)', () => {
    const tabs = { t: 'Tabs', titles: [], panes: { b: { t: 'Empty', key: 'b' }, a: { t: 'Empty', key: 'a' } } };
    const out = run(tabs, [{ op: 'splice', path: [], field: 'panes', index: 1, remove: 1, insert: [{ t: 'Spinner', key: 'c' }] }]) as {
      panes: Record<string, { t: string }>;
    };
    expect(Object.keys(out.panes)).toEqual(['a', 'c']);
  });

  it('append ring-trims Log to maxLines and refuses non-appendable fields', () => {
    const out = run({ t: 'Log', lines: ['1', '2'], maxLines: 3 }, [{ op: 'append', path: [], field: 'lines', items: ['3', '4', '5'] }]) as { lines: string[] };
    expect(out.lines).toEqual(['3', '4', '5']);
    expect(() => run({ t: 'Tree', nodes: [] }, [{ op: 'append', path: [], field: 'nodes', items: [] }])).toThrow(/does not exist on Tree/);
  });

  it('item replaces by explicit key, else positional #i', () => {
    const list = { t: 'List', items: [{ key: 'a', content: 'A' }, 'plain'] };
    const out = run(list, [
      { op: 'item', path: [], field: 'items', key: 'a', value: { key: 'a', content: 'A2' } },
      { op: 'item', path: [], field: 'items', key: '#1', value: 'plain2' },
    ]) as { items: unknown[] };
    expect(out.items).toEqual([{ key: 'a', content: 'A2' }, 'plain2']);
    expect(() => run(list, [{ op: 'item', path: [], field: 'items', key: 'zz', value: 'x' }])).toThrow(/missing key "zz"/);
  });
});

describe('atomicity', () => {
  it('rolls back every op of a failing batch', () => {
    const h = { root: structuredClone(layout) } as unknown as Record<string, unknown>;
    const before = JSON.stringify(h);
    expect(() =>
      applyNodeOps(h, 'root', [
        { op: 'set', path: ['head'], props: { alignment: 'Right' } },
        { op: 'splice', path: [], field: 'children', index: 0, remove: 1, insert: [] },
        { op: 'append', path: ['tabs', 'x'], field: 'lines', items: ['2'] },
        { op: 'set', path: ['gone'], props: {} },
      ]),
    ).toThrow();
    expect(JSON.stringify(h)).toBe(before);
  });

  it('works on a Solid store draft (in place, fine-grained)', () => {
    const [state, setState] = createStore({ view: { root: structuredClone(layout) as Record<string, unknown> } });
    const headBefore = (state.view.root.children as unknown[])[0];
    setState(
      produce((s) => {
        applyNodeOps(s.view as unknown as Record<string, unknown>, 'root', [{ op: 'set', path: ['main', 'child'], props: { multi: true } }], new Tx());
      }),
    );
    const children = state.view.root.children as Array<Record<string, any>>;
    expect(children[1]!.child.multi).toBe(true);
    expect(children[0]).toBe(headBefore);
  });
});
