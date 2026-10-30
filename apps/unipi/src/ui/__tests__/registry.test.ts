/**
 * Registry (ui-protocol §6, §9.2, §13.7): snapshot / patch / resync /
 * ordering / dispose, mirrored from `oino-view`'s `RegistryMirror` tests on
 * the shared `crates/oino-ui/fixtures/registry/*.json` corpus.
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createUiRegistry } from '../registry';
import { localEntries, resetLocal, useLocal } from '../localState';
import { createRoot } from 'solid-js';
import type { RegistryOp, UiEntry, UiPatch, UiSnapshot } from '../types';

const FIX = resolve(__dirname, '../../../../../crates/oino-ui/fixtures/registry');
const fixture = <T>(name: string): T => JSON.parse(readFileSync(resolve(FIX, name), 'utf8')) as T;

function entry(id: string, slot: UiEntry['slot'], seq: number, extra: Partial<UiEntry> = {}): UiEntry {
  return { id, extensionId: 'ext', scope: 'session', slot, order: 0, seq, view: { kind: 'tree', root: { t: 'Paragraph', text: id } }, rev: 1, ...extra };
}
const patchAt = (rev: number, ops: RegistryOp[]): UiPatch => ({ scope: 'session', sessionId: 's-2', rev, ops });
const mirrored = (name: string, hooks = {}) => {
  const reg = createUiRegistry('session', hooks);
  expect(reg.applySnapshot(fixture<UiSnapshot>(name)).ok).toBe(true);
  return reg;
};

afterEach(() => resetLocal());

describe('snapshot', () => {
  it('mirrors entries, dialogs and meta', () => {
    const reg = mirrored('snapshot.json');
    expect(reg.state.rev).toBe(4);
    expect(reg.state.sessionId).toBe('s-1');
    expect(Object.keys(reg.state.entries)).toHaveLength(3);
    expect(reg.entriesFor('widget')().map((e) => e.id)).toEqual(['ext/widget:progress']);
    const board = reg.entry('unipi.kanboard/board')!;
    expect(board.order).toBe(1000);
    expect(board.view.kind).toBe('reserved');
    expect((board.view as { fallback: { t: string } }).fallback.t).toBe('List');
    expect(reg.state.meta).toMatchObject({ title: 'fix: bug', workingMessage: 'running', workingVisible: true });
    expect(reg.state.meta.shortcuts).toHaveLength(1);
    expect(reg.dialogs().map((d) => d.id)).toEqual(['1']);
  });

  it('re-applying the same snapshot keeps every object identity (no re-render)', () => {
    const reg = mirrored('snapshot.json');
    const before = reg.entry('ext/widget:progress')!;
    const root = (before.view as { root: unknown }).root;
    reg.applySnapshot(fixture<UiSnapshot>('snapshot.json'));
    expect(reg.entry('ext/widget:progress')).toBe(before);
    expect((reg.entry('ext/widget:progress')!.view as { root: unknown }).root).toBe(root);
  });

  it('refuses the other scope', () => {
    const reg = createUiRegistry('host');
    expect(reg.applySnapshot(fixture<UiSnapshot>('snapshot.json')).ok).toBe(false);
    expect(reg.applyPatch(patchAt(1, [])).ok).toBe(false);
  });
});

describe('patch', () => {
  it('refuses the fixture whole (its tree op is a vocabulary sample) and asks for a resync', () => {
    const resync = vi.fn();
    const reg = mirrored('snapshot.json', { resync });
    const out = reg.applyPatch(fixture<UiPatch>('patch.json'));
    expect(out.ok).toBe(false);
    expect(resync).toHaveBeenCalledWith(4);
    expect(reg.state.rev).toBe(4);
    expect(reg.entry('ext/panel-1')).toBeUndefined();
    expect(reg.dialogs()).toHaveLength(1);
  });

  it('applies every other op kind, handing out effects and invalidations', () => {
    const onEffect = vi.fn();
    const onInvalidate = vi.fn();
    const reg = mirrored('snapshot.json', { onEffect, onInvalidate });
    const p = fixture<UiPatch>('patch.json');
    p.ops = p.ops.filter((op) => op.op !== 'tree');
    const out = reg.applyPatch(p);
    expect(out.ok).toBe(true);
    expect(reg.state.rev).toBe(5);
    expect(reg.entry('ext/panel-1')).toBeUndefined();
    expect(reg.entriesFor('sidePane')()).toHaveLength(0);
    expect(onEffect).toHaveBeenCalledWith(expect.objectContaining({ id: '10', method: 'notify', message: 'done' }));
    expect(onInvalidate).toHaveBeenCalledWith({ id: 'ext/turnFooter:files', turns: ['t-9'] });
    expect(reg.state.meta.workingVisible).toBe(false);
    expect(reg.state.meta.title).toBe('fix: bug');
    expect(reg.dialogs().map((d) => d.id)).toEqual(['1']);
    expect(reg.entry('ext/widget:progress')!.rev).toBe(1);
  });

  it('a gap requests a resync and a snapshot recovers', () => {
    const resync = vi.fn();
    const reg = mirrored('ordering.json', { resync });
    const n = Object.keys(reg.state.entries).length;
    const gap = fixture<UiPatch>('resync-gap.json');
    const out = reg.applyPatch(gap);
    expect(out).toMatchObject({ ok: false, resync: true });
    expect(resync).toHaveBeenCalledWith(1);
    expect(reg.state.rev).toBe(1);
    expect(Object.keys(reg.state.entries)).toHaveLength(n);
    expect(reg.entry('ext/widget:late')).toBeUndefined();
    // Patches while the resync is in flight are dropped (no second request).
    expect(reg.applyPatch(patchAt(2, [])).ok).toBe(false);
    expect(resync).toHaveBeenCalledTimes(1);
    reg.applySnapshot(fixture<UiSnapshot>('snapshot.json'));
    expect(reg.state.rev).toBe(4);
    expect(reg.state.sessionId).toBe('s-1');
    expect(Object.keys(reg.state.entries)).toHaveLength(3);
    expect(reg.applyPatch({ ...patchAt(5, []), sessionId: 's-1' }).ok).toBe(true);
  });

  it('applies atomically: a later failing op rolls back the earlier ones', () => {
    const reg = mirrored('ordering.json');
    const out = reg.applyPatch(
      patchAt(2, [
        { op: 'put', entry: entry('ext/panel-new', 'overlay', 9) },
        { op: 'meta', id: 'ext/side-x', set: { title: 'Changed' } },
        { op: 'tree', id: 'ext/panel-a', rev: 2, ops: [{ op: 'set', path: [], props: { alignment: 'Center' } }] },
        { op: 'meta', id: 'ext/missing', set: { title: 'x' } },
      ]),
    );
    expect(out.ok).toBe(false);
    expect(reg.entry('ext/panel-new')).toBeUndefined();
    expect(reg.entry('ext/side-x')!.title).toBe('X');
    expect((reg.entry('ext/panel-a')!.view as { root: Record<string, unknown> }).root.alignment).toBeUndefined();
    expect(reg.entry('ext/panel-a')!.rev).toBe(1);
    expect(reg.state.rev).toBe(1);
  });

  it('refuses tree ops on non-tree entries and malformed node ops', () => {
    const reg = mirrored('snapshot.json');
    expect(reg.applyPatch({ ...patchAt(5, [{ op: 'tree', id: 'ext/settings:ext', rev: 2, ops: [] }]), sessionId: 's-1' })).toMatchObject({
      ok: false,
      error: expect.stringContaining('not a tree'),
    });
    reg.endResync();
    const bad = reg.applyPatch({
      ...patchAt(5, [{ op: 'tree', id: 'ext/widget:progress', rev: 2, ops: [{ op: 'splice', path: [], field: 'children', index: 0, remove: 1, insert: [] }] }]),
      sessionId: 's-1',
    });
    expect(bad).toMatchObject({ ok: false, error: expect.stringContaining('ext/widget:progress') });
    expect(reg.state.rev).toBe(4);
  });

  it('tree ops reach the entry root and carry the entry rev', () => {
    const reg = mirrored('ordering.json');
    expect(reg.applyPatch(patchAt(2, [{ op: 'tree', id: 'ext/panel-a', rev: 2, ops: [{ op: 'replace', path: [], node: { t: 'List', items: [] } }] }])).ok).toBe(true);
    expect(reg.applyPatch(patchAt(3, [{ op: 'tree', id: 'ext/panel-a', rev: 3, ops: [{ op: 'append', path: [], field: 'items', items: [{ key: 'b', content: 'B' }] }] }])).ok).toBe(true);
    const e = reg.entry('ext/panel-a')!;
    expect(e.rev).toBe(3);
    expect((e.view as { root: { items: Array<{ key: string }> } }).root.items[0]!.key).toBe('b');
  });

  it('meta ops patch fields, bump rev and re-sort stacks; a repeated patch is a gap', () => {
    const reg = mirrored('ordering.json');
    const ops: RegistryOp[] = [
      { op: 'put', entry: entry('ext/panel-new', 'overlay', 9) },
      { op: 'meta', id: 'ext/panel-new', set: { title: 'Renamed', icon: 'gear', order: 3, focus: true } },
      { op: 'meta', id: 'ext/panel-c', set: { badge: '3' } },
    ];
    expect(reg.applyPatch(patchAt(2, ops)).ok).toBe(true);
    const n = reg.entry('ext/panel-new')!;
    expect(n).toMatchObject({ title: 'Renamed', icon: 'gear', order: 3, focus: true, rev: 2 });
    expect(reg.entry('ext/panel-c')!.badge).toBe('3');
    expect(reg.stackTop('overlay')?.id).toBe('ext/panel-new');
    expect(reg.applyPatch(patchAt(2, ops))).toMatchObject({ ok: false, error: expect.stringContaining('expected 3, got 2') });
  });
});

describe('ordering (§6.4)', () => {
  it('sorts multi-entry slots by order desc then seq asc, stacks bottom → top', () => {
    const reg = mirrored('ordering.json');
    expect(reg.entriesFor('sidePane')().map((e) => e.id)).toEqual(['ext/side-z', 'ext/side-x', 'ext/side-y']);
    expect(reg.stack('overlay')().map((e) => e.id)).toEqual(['ext/panel-a', 'ext/panel-b', 'ext/panel-c']);
    expect(reg.stackTop('overlay')?.id).toBe('ext/panel-c');
    expect(reg.stackTop('bottomPane')).toBeUndefined();
  });

  it('slot selectors keep their array when an unrelated slot changes', () => {
    const reg = mirrored('ordering.json');
    const side = reg.entriesFor('sidePane')();
    reg.applyPatch(patchAt(2, [{ op: 'put', entry: entry('ext/w', 'widget', 10) }]));
    expect(reg.entriesFor('sidePane')()).toBe(side);
  });
});

describe('dialogs (pi records)', () => {
  it('upserts dialog requests by id, ignores retained producers, dismisses', () => {
    const reg = mirrored('ordering.json');
    expect(reg.dialogs().map((d) => d.id)).toEqual(['dlg-1', 'dlg-2']);
    reg.applyRequest({ type: 'extension_ui_request', id: 'dlg-3', method: 'input', title: 'Name' });
    reg.applyRequest({ type: 'extension_ui_request', id: 'x', method: 'setStatus', statusKey: 'k' });
    reg.applyRequest({ type: 'extension_ui_request', id: 'dlg-3', method: 'input', title: 'Name again' });
    expect(reg.dialogs().map((d) => d.id)).toEqual(['dlg-1', 'dlg-2', 'dlg-3']);
    expect(reg.dialogs()[2]!.title).toBe('Name again');
    reg.applyRequest({ type: 'extension_ui_dismiss', id: 'dlg-1' });
    expect(reg.dialogs().map((d) => d.id)).toEqual(['dlg-2', 'dlg-3']);
  });
});

describe('dispose (§6.6)', () => {
  it('drops local state of deleted entries and of every entry on dispose', () => {
    const reg = mirrored('ordering.json');
    createRoot(() => {
      useLocal('ext/panel-a', [], 'sel', { init: () => 1 });
      useLocal('ext/panel-b', [], 'sel', { init: () => 1 });
    });
    expect(localEntries()).toBe(2);
    reg.applyPatch(patchAt(2, [{ op: 'del', id: 'ext/panel-a' }]));
    expect(localEntries()).toBe(1);
    reg.dispose();
    expect(localEntries()).toBe(0);
  });
});
