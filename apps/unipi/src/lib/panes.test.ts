import { describe, expect, it } from 'vitest';
import {
  activeTab,
  closeAllTabs,
  closeOtherTabs,
  closeTab,
  createPanes,
  dropSessionTabs,
  EMPTY_SET,
  moveTab,
  openTab,
  recentlyClosed,
  RECENT_MAX,
  reopenTab,
  visibleTabs,
  type KindRule,
  type PaneSet,
} from './panes';
import { BUILTIN_KINDS, launchableKinds, paneKind, registerPaneKind, setPaneComponent, tabTitle } from './panes/kinds';

const rules: Record<string, KindRule> = {
  files: { singleton: true, scope: 'project' },
  tree: { singleton: true, scope: 'session' },
  btw: { singleton: false, scope: 'session' },
  diff: { singleton: (p) => String(p.path), scope: 'session' },
  browser: { singleton: false, scope: 'project' },
  temp: { singleton: false, scope: 'project', noRecent: true },
};
const rule = (k: string) => rules[k];
let n = 0;
const id = () => `t${++n}`;
const s1 = { sessionId: 's1' };
const s2 = { sessionId: 's2' };

function open(set: PaneSet, kind: string, view = s1, params?: Record<string, unknown>) {
  return openTab(set, { kind, params }, view, rules[kind]!, id);
}

describe('open / singletons / scope', () => {
  it('a singleton activates its existing tab; multi kinds open new ones', () => {
    let { set, id: a } = open(EMPTY_SET, 'files');
    const again = open(set, 'files');
    expect(again.id).toBe(a);
    expect(again.set.tabs).toHaveLength(1);
    set = open(set, 'btw').set;
    set = open(set, 'btw').set;
    expect(set.tabs.filter((t) => t.kind === 'btw')).toHaveLength(2);
  });
  it('per-key kinds reuse the tab of the same key', () => {
    let { set, id: a } = open(EMPTY_SET, 'diff', s1, { path: 'a.ts' });
    const same = open(set, 'diff', s1, { path: 'a.ts' });
    expect(same.id).toBe(a);
    set = open(same.set, 'diff', s1, { path: 'b.ts' }).set;
    expect(set.tabs).toHaveLength(2);
  });
  it('session tabs are tagged, hidden in another session and back when it returns', () => {
    let set = open(EMPTY_SET, 'files').set;
    set = open(set, 'tree', s1).set;
    expect(set.tabs.find((t) => t.kind === 'files')!.sessionId).toBeUndefined();
    expect(set.tabs.find((t) => t.kind === 'tree')!.sessionId).toBe('s1');
    expect(visibleTabs(set, s2).map((t) => t.kind)).toEqual(['files']);
    expect(visibleTabs(set, s1).map((t) => t.kind)).toEqual(['files', 'tree']);
    // a session singleton is per session
    const other = open(set, 'tree', s2);
    expect(other.set.tabs.filter((t) => t.kind === 'tree')).toHaveLength(2);
  });
  it('the active tab is remembered per view', () => {
    let { set, id: files } = open(EMPTY_SET, 'files');
    const tree = open(set, 'tree', s1);
    set = tree.set;
    expect(activeTab(set, s1)!.id).toBe(tree.id);
    expect(activeTab(set, s2)!.id).toBe(files); // s2 never picked: first visible
  });
});

describe('close / others / all / recently closed / reopen', () => {
  const three = () => {
    let set = EMPTY_SET;
    const ids: string[] = [];
    for (const k of ['files', 'btw', 'browser']) {
      const r = open(set, k);
      set = r.set;
      ids.push(r.id);
    }
    return { set, ids };
  };
  it('closing the active tab activates the right neighbour, else the left', () => {
    const { set, ids } = three();
    const mid = closeTab({ ...set, active: { s1: ids[1]! } }, ids[1]!, s1, rule);
    expect(activeTab(mid, s1)!.id).toBe(ids[2]);
    const last = closeTab(set, ids[2]!, s1, rule);
    expect(activeTab(last, s1)!.id).toBe(ids[1]);
  });
  it('close others / all touch only the visible tabs', () => {
    let { set, ids } = three();
    set = open(set, 'btw', s2).set; // hidden in s1
    const others = closeOtherTabs(set, ids[0]!, s1, rule);
    expect(visibleTabs(others, s1).map((t) => t.id)).toEqual([ids[0]]);
    expect(others.tabs.some((t) => t.sessionId === 's2')).toBe(true);
    const all = closeAllTabs(set, s1, rule);
    expect(visibleTabs(all, s1)).toEqual([]);
    expect(all.tabs).toHaveLength(1);
  });
  it('recently closed: newest first, max 10, per view; reopen keeps id and owner', () => {
    let { set, ids } = three();
    set = closeTab(set, ids[0]!, s1, rule);
    set = closeTab(set, ids[1]!, s1, rule);
    expect(recentlyClosed(set, s1).map((t) => t.id)).toEqual([ids[1], ids[0]]);
    // the side chat belonged to s1: s2 cannot restore it; project tabs can be restored anywhere
    expect(recentlyClosed(set, s2).map((t) => t.id)).toEqual([ids[0]]);
    const r = reopenTab(set, s1);
    expect(r.id).toBe(ids[1]);
    expect(r.set.tabs.find((t) => t.id === ids[1])!.sessionId).toBe('s1');
    expect(activeTab(r.set, s1)!.id).toBe(ids[1]);
    expect(recentlyClosed(r.set, s1).map((t) => t.id)).toEqual([ids[0]]);
    let many = EMPTY_SET;
    for (let i = 0; i < 14; i++) {
      const o = open(many, 'browser');
      many = closeTab(o.set, o.id, s1, rule);
    }
    expect(many.closed).toHaveLength(RECENT_MAX);
  });
  it('noRecent kinds are not remembered', () => {
    const o = open(EMPTY_SET, 'temp');
    expect(closeTab(o.set, o.id, s1, rule).closed).toEqual([]);
  });
});

describe('reorder and drop session', () => {
  it('moves before a tab or to the end', () => {
    let set = EMPTY_SET;
    const ids: string[] = [];
    for (const k of ['files', 'btw', 'browser']) {
      const r = open(set, k);
      set = r.set;
      ids.push(r.id);
    }
    expect(moveTab(set, ids[2]!, ids[0]).tabs.map((t) => t.id)).toEqual([ids[2], ids[0], ids[1]]);
    expect(moveTab(set, ids[0]!).tabs.map((t) => t.id)).toEqual([ids[1], ids[2], ids[0]]);
  });
  it('dropSession removes its open and closed tabs', () => {
    let set = open(EMPTY_SET, 'files').set;
    const t = open(set, 'tree', s1);
    set = closeTab(open(t.set, 'btw', s1).set, t.id, s1, rule);
    const dropped = dropSessionTabs(set, 's1');
    expect(dropped.tabs.map((x) => x.kind)).toEqual(['files']);
    expect(dropped.closed).toEqual([]);
  });
});

describe('store: persistence + scope switch', () => {
  const mem = () => {
    const data = new Map<string, string>();
    return {
      getItem: (k: string) => data.get(k) ?? null,
      setItem: (k: string, v: string) => void data.set(k, v),
      removeItem: (k: string) => void data.delete(k),
      key: (i: number) => [...data.keys()][i] ?? null,
      get length() {
        return data.size;
      },
      data,
    };
  };
  const projA = { hostId: 'h', projectKey: '/a' };
  const projB = { hostId: 'h', projectKey: '/b' };
  it('tabs are per project, persisted, and come back in a new store', () => {
    const storage = mem();
    const store = createPanes({ storage, rules: rule });
    store.open(projA, 'side', s1, { kind: 'files' });
    store.open(projA, 'side', s1, { kind: 'tree' });
    store.open(projA, 'bottom', s1, { kind: 'browser' });
    expect(store.visible(projB, 'side', s1)).toEqual([]);
    expect(storage.data.has('unipi.panes:h|/a')).toBe(true);
    const again = createPanes({ storage, rules: rule });
    expect(again.visible(projA, 'side', s1).map((t) => t.kind)).toEqual(['files', 'tree']);
    expect(again.visible(projA, 'bottom', s1).map((t) => t.kind)).toEqual(['browser']);
    again.dropSession('h', 's1');
    expect(again.visible(projA, 'side', s1).map((t) => t.kind)).toEqual(['files']);
    again.closeAll(projA, 'side', s1);
    again.closeAll(projA, 'bottom', s1);
  });
});

describe('kind registry', () => {
  it('declares the §3.8 kinds with their singleton / scope rules', () => {
    const k = Object.fromEntries(BUILTIN_KINDS.map((d) => [d.kind, d]));
    expect(Object.keys(k).sort()).toEqual(['browser', 'btw', 'diff', 'ext', 'files', 'forward', 'git', 'preview', 'subagent', 'terminal', 'tree']);
    expect(k.files!.singleton).toBe(true);
    expect(k.git!.scope).toBe('project');
    expect(k.tree!.scope).toBe('session');
    expect(k.btw!.singleton).toBe(false);
    expect(k.terminal!.areas).toContain('bottom');
  });
  it('the launcher offers only kinds with a view; setPaneComponent plugs one in', () => {
    expect(launchableKinds('side', true).map((x) => x.kind)).not.toContain('files');
    const off = setPaneComponent('files', () => null);
    expect(launchableKinds('side', true).map((x) => x.kind)).toContain('files');
    expect(launchableKinds('bottom', true).map((x) => x.kind)).not.toContain('files');
    off();
    expect(paneKind('files')!.component).toBeUndefined();
    const offTree = setPaneComponent('tree', () => null);
    expect(launchableKinds('side', false).map((x) => x.kind)).not.toContain('tree'); // needs a session
    offTree();
  });
  it('registerPaneKind adds an extension kind; titles come from params', () => {
    const off = registerPaneKind({ kind: 'demo', icon: (() => null) as never, title: (p) => `Demo ${String(p.n)}`, singleton: false, scope: 'project' });
    expect(tabTitle({ id: 'x', kind: 'demo', params: { n: 2 } })).toBe('Demo 2');
    expect(tabTitle({ id: 'x', kind: 'demo', title: 'Mine' })).toBe('Mine');
    off();
    expect(paneKind('demo')).toBeUndefined();
    expect(tabTitle({ id: 'y', kind: 'forward', params: { port: 5173 } })).toBe('localhost:5173');
  });
});
