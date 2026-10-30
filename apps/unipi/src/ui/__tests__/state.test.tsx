/**
 * Local state "prop changed ⇒ adopt" (D-U13, §4.6), the action payload /
 * values collection (§4.6, §9.3) and the stream → registry routing (§13.2).
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createRoot, createSignal } from 'solid-js';
import { createStore } from 'solid-js/store';
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import { resetLocal, useLocal } from '../localState';
import { createEntryActions, setUiTransport, type UiActionPayload, type UiTransport } from '../actions';
import { onUiEffect, resetUiRegistries, routeSessionUi, sessionUi } from '../registries';
import { EntryView } from '../NodeView';
import type { UiEntry } from '../types';

afterEach(() => {
  cleanup();
  resetLocal();
  resetUiRegistries();
  setUiTransport(undefined);
});

describe('useLocal', () => {
  it('keeps local edits across prop-equal updates and adopts a changed prop', () => {
    createRoot((dispose) => {
      const [prop, setProp] = createSignal<string | undefined>('a');
      const [v, set] = useLocal('e', ['x'], 'sel', { prop, init: () => 'none' });
      expect(v()).toBe('a');
      set('b');
      setProp('a'); // same value: local edit stays
      expect(v()).toBe('b');
      setProp('c'); // changed: adopt
      expect(v()).toBe('c');
      dispose();
    });
  });

  it('survives a re-mount and adopts a prop that changed while unmounted', () => {
    let set!: (v: string) => void;
    createRoot((dispose) => {
      [, set] = useLocal('e', ['x'], 'sel', { prop: () => 'a', init: () => '' });
      set('mine');
      dispose();
    });
    createRoot((dispose) => {
      const [v] = useLocal('e', ['x'], 'sel', { prop: () => 'a', init: () => '' });
      expect(v()).toBe('mine');
      dispose();
    });
    createRoot((dispose) => {
      const [v] = useLocal('e', ['x'], 'sel', { prop: () => 'z', init: () => '' });
      expect(v()).toBe('z');
      dispose();
    });
  });

  it('a tree update that changes `selected` drives the List', () => {
    const [entry, setEntry] = createStore<UiEntry>({
      id: 'x/list',
      extensionId: 'x',
      scope: 'session',
      slot: 'sidePane',
      order: 0,
      seq: 1,
      rev: 1,
      view: { kind: 'tree', root: { t: 'List', items: [{ key: 'a', content: 'A' }, { key: 'b', content: 'B' }], state: { selected: 'a' }, onSelect: 'pick' } },
    });
    setUiTransport({ action: async () => ({}), visible: async () => ({}), resync: async () => ({}) });
    const { container } = render(() => <EntryView entry={entry} target={{ scope: 'session', hostId: 'h', sessionId: 's' }} />);
    const sel = () => [...container.querySelectorAll('[role=option]')].map((r) => r.getAttribute('aria-selected'));
    expect(sel()).toEqual(['true', 'false']);
    fireEvent.click(container.querySelectorAll('[role=option]')[1]!);
    expect(sel()).toEqual(['false', 'true']);
    setEntry('view', 'root' as never, 'items' as never, [{ key: 'a', content: 'A' }, { key: 'b', content: 'B2' }] as never); // unrelated change
    expect(sel()).toEqual(['false', 'true']);
    setEntry('view', 'root' as never, 'state' as never, { selected: 'a' } as never); // same as before → no adopt
    expect(sel()).toEqual(['false', 'true']);
  });
});

describe('actions', () => {
  it('sends _oino/ui_action with values only for submit-style actions', async () => {
    const calls: UiActionPayload[] = [];
    const transport: UiTransport = { action: async (_t, p) => void calls.push(p), visible: vi.fn(async () => ({})), resync: async () => ({}) };
    setUiTransport(transport);
    const a = createEntryActions('ext/p', { scope: 'session', hostId: 'h', sessionId: 's' });
    a.registerValue('name', () => 'Ada');
    a.registerValue('on', () => true);
    a.send('pick', 'k', ['main', 'list']);
    a.send('save', 'Ada', [], true);
    a.send(undefined, 'ignored');
    a.visible(true);
    expect(calls).toEqual([
      { entryId: 'ext/p', actionId: 'pick', value: 'k', path: ['main', 'list'] },
      { entryId: 'ext/p', actionId: 'save', value: 'Ada', values: { name: 'Ada', on: true } },
    ]);
    expect(transport.visible).toHaveBeenCalledWith({ scope: 'session', hostId: 'h', sessionId: 's' }, 'ext/p', true);
  });
});

describe('registries (stream → registry)', () => {
  const snap = {
    type: 'oino_ui_snapshot',
    scope: 'session',
    rev: 1,
    entries: [{ id: 'x/w', extensionId: 'x', scope: 'session', slot: 'widget', order: 0, seq: 1, rev: 1, view: { kind: 'lines', lines: ['hi'] } }],
    dialogs: [],
  };

  it('routes snapshot, patch, dialogs and effects per session', () => {
    const effects: unknown[] = [];
    onUiEffect((_t, r) => effects.push(r));
    routeSessionUi('h', 's', snap);
    routeSessionUi('h', 's', { type: 'oino_ui_patch', scope: 'session', rev: 2, ops: [{ op: 'effect', request: { id: '1', method: 'notify', message: 'hey' } }] });
    routeSessionUi('h', 's', { type: 'extension_ui_request', id: 'd1', method: 'confirm', title: 'Sure?' });
    const reg = sessionUi('h', 's');
    expect(reg.state.rev).toBe(2);
    expect(reg.entriesFor('widget')().map((e) => e.id)).toEqual(['x/w']);
    expect(reg.dialogs().map((d) => d.id)).toEqual(['d1']);
    expect(effects).toEqual([{ id: '1', method: 'notify', message: 'hey' }]);
    expect(sessionUi('h', 'other').state.rev).toBe(0);
  });

  it('a gap asks the transport to resync and applies its snapshot', async () => {
    const resync = vi.fn(async () => ({ ...snap, type: undefined, rev: 7 }));
    setUiTransport({ action: async () => ({}), visible: async () => ({}), resync });
    routeSessionUi('h', 's', snap);
    routeSessionUi('h', 's', { type: 'oino_ui_patch', scope: 'session', rev: 5, ops: [] });
    expect(resync).toHaveBeenCalledWith({ scope: 'session', hostId: 'h', sessionId: 's' }, 1);
    await vi.waitFor(() => expect(sessionUi('h', 's').state.rev).toBe(7));
  });
});
