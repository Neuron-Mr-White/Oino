/**
 * Golden per node (ui-protocol §13.7): every fixture of
 * `crates/oino-ui/fixtures/nodes/*.json` — the corpus the TUI snapshots —
 * rendered through `EntryView` in jsdom at the desktop and the phone shell;
 * the serialized DOM (classes, styles, aria) is snapshotted. Interactive
 * fixtures also assert the emitted `_oino/ui_action` payload.
 */
import { readdirSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import { EntryView } from '../NodeView';
import { refreshShell } from '../layout/units';
import { resetLocal } from '../localState';
import { setUiTransport, type UiActionPayload } from '../actions';
import type { UiEntry, UiNode } from '../types';

const DIR = resolve(__dirname, '../../../../../crates/oino-ui/fixtures/nodes');
const FILES = readdirSync(DIR)
  .filter((f) => f.endsWith('.json'))
  .sort();
const load = (file: string) => JSON.parse(readFileSync(resolve(DIR, file), 'utf8')) as UiNode;

const entryOf = (file: string, root: UiNode): UiEntry => ({
  id: `gallery/${file.replace(/\.json$/, '')}`,
  extensionId: 'gallery',
  scope: 'session',
  slot: 'sidePane',
  order: 0,
  seq: 1,
  view: { kind: 'tree', root },
  rev: 1,
});

function setShell(shell: 'desktop' | 'phone') {
  if (shell === 'desktop') document.documentElement.dataset.shell = 'desktop';
  else delete document.documentElement.dataset.shell;
  refreshShell();
}

function mount(file: string) {
  const sent: UiActionPayload[] = [];
  const view = render(() => (
    <EntryView entry={entryOf(file, load(file))} target={{ scope: 'session', hostId: 'h', sessionId: 's' }} onAction={(p) => sent.push(p)} />
  ));
  return { ...view, sent };
}

/** Generated ids differ by render order; keep snapshots stable. */
function normalize(el: Element): Element {
  const clone = el.cloneNode(true) as Element;
  const ids = new Map<string, string>();
  const norm = (v: string) => v.replace(/\b(cl-\d+|solid-\S+-\d+|[a-z]+-\d+-\d+)\b/g, (m) => ids.get(m) ?? (ids.set(m, `id${ids.size}`), ids.get(m)!));
  for (const node of [clone, ...clone.querySelectorAll('*')]) {
    for (const attr of ['id', 'for', 'aria-labelledby', 'aria-controls', 'aria-describedby', 'aria-activedescendant']) {
      const v = node.getAttribute(attr);
      if (v) node.setAttribute(attr, norm(v));
    }
  }
  return clone;
}

beforeEach(() => {
  resetLocal();
  setUiTransport({ action: async () => ({}), visible: async () => ({}), resync: async () => ({}) });
});
afterEach(() => {
  cleanup();
  setUiTransport(undefined);
  setShell('phone');
});

describe.each(['desktop', 'phone'] as const)('golden — %s shell', (shell) => {
  beforeEach(() => setShell(shell));
  it('covers every node kind in the corpus', () => {
    const kinds = new Set(FILES.map((f) => load(f).t));
    expect([...kinds].sort()).toEqual(
      ['BarChart', 'Block', 'Button', 'Chart', 'Code', 'Diff', 'Empty', 'Gauge', 'Image', 'Input', 'KeyHints', 'Layout', 'LineGauge', 'List', 'Log', 'Markdown', 'Paragraph', 'Scrollbar', 'Select', 'Sparkline', 'Spinner', 'Table', 'Tabs', 'Toggle', 'Tree'].sort(),
    );
  });
  it.each(FILES)('%s', (file) => {
    const { container } = mount(file);
    const root = container.firstElementChild!;
    expect(root.querySelector('[data-testid=ui-entry-failed]')).toBeNull();
    expect(normalize(root)).toMatchSnapshot();
  });
});

describe('interactive fixtures emit their actions', () => {
  beforeEach(() => setShell('desktop'));
  const action = (sent: UiActionPayload[], id: string) => sent.filter((p) => p.actionId === id);

  it('Button → value; confirm asks first', async () => {
    const plain = mount('Button-danger-confirm.json');
    fireEvent.click(plain.container.querySelector('button')!);
    expect(plain.sent).toEqual([{ entryId: 'gallery/Button-danger-confirm', actionId: 'delete', value: null }]);
    cleanup();
    const asked = mount('Button.json');
    fireEvent.click(asked.container.querySelector('button')!);
    expect(asked.sent).toEqual([]);
    const confirm = asked.container.querySelector('.ui-confirm .btn-primary')!;
    expect(asked.container.querySelector('.ui-confirm-text')!.textContent).toBe('Deploy to production?');
    fireEvent.click(confirm);
    expect(asked.sent).toEqual([{ entryId: 'gallery/Button', actionId: 'deploy', value: { env: 'prod' } }]);
  });

  it('Layout children + Empty action keep their paths', () => {
    const l = mount('Layout-margin-spacing-flex.json');
    fireEvent.click(l.container.querySelectorAll('button')[1]!);
    expect(l.sent).toEqual([{ entryId: 'gallery/Layout-margin-spacing-flex', actionId: 'cancel', value: null, path: ['#1'] }]);
    cleanup();
    const e = mount('Empty-action.json');
    fireEvent.click(e.container.querySelector('button')!);
    expect(e.sent[0]).toMatchObject({ actionId: 'new-task', path: ['action'] });
  });

  it('Block toggle → collapsed bool', () => {
    const { container, sent } = mount('Block-collapsible-collapsed.json');
    expect(container.querySelector('.ui-block-body')).toBeNull();
    fireEvent.click(container.querySelector('.ui-block-toggle')!);
    expect(sent).toEqual([{ entryId: 'gallery/Block-collapsible-collapsed', actionId: 'toggle-details', value: false }]);
    expect(container.querySelector('.ui-block-body')!.textContent).toContain('hidden');
  });

  it('List select / activate', () => {
    const { container, sent } = mount('List-state-selected.json');
    const rows = container.querySelectorAll('[role=option]');
    // Text items have positional keys (#0, #1), like the TUI: `selected: "b"` matches none.
    expect([...rows].map((r) => r.getAttribute('aria-selected'))).toEqual(['false', 'false']);
    fireEvent.click(rows[0]!);
    fireEvent.keyDown(container.querySelector('[role=listbox]')!, { key: 'Enter' });
    expect(sent.map((p) => [p.actionId, p.value])).toEqual([
      ['pick', '#0'],
      ['open', '#0'],
    ]);
    fireEvent.keyDown(container.querySelector('[role=listbox]')!, { key: 'ArrowDown' });
    expect(action(sent, 'pick').at(-1)!.value).toBe('#1');
  });

  it('Table activate on Enter', () => {
    const { container, sent } = mount('Table-state-sticky.json');
    fireEvent.keyDown(container.querySelector('[role=grid]')!, { key: 'Enter' });
    expect(sent).toEqual([{ entryId: 'gallery/Table-state-sticky', actionId: 'open-row', value: 'r1' }]);
  });

  it('Tabs switch locally and report the key', () => {
    const { container, sent } = mount('Tabs-basic.json');
    const tabs = container.querySelectorAll('[role=tab]');
    expect(tabs[1]!.getAttribute('aria-selected')).toBe('true');
    fireEvent.click(tabs[0]!);
    expect(sent).toEqual([{ entryId: 'gallery/Tabs-basic', actionId: 'tab', value: 'one' }]);
    expect(container.querySelectorAll('[role=tab]')[0]!.getAttribute('aria-selected')).toBe('true');
  });

  it('Tree toggle + keyboard select', () => {
    const { container, sent } = mount('Tree.json');
    const twisties = container.querySelectorAll('button.ui-tree-twisty');
    fireEvent.click(twisties[1]!); // tests (lazy)
    expect(action(sent, 'fold')[0]!.value).toEqual({ key: 'tests', expanded: true });
    fireEvent.keyDown(container.querySelector('[role=tree]')!, { key: 'ArrowUp' });
    expect(action(sent, 'pick')[0]!.value).toBe('src');
    fireEvent.keyDown(container.querySelector('[role=tree]')!, { key: 'ArrowLeft' });
    expect(action(sent, 'fold')[1]!.value).toEqual({ key: 'src', expanded: false });
  });

  it('Input: Enter submits with the form values', () => {
    const { container, sent } = mount('Input.json');
    const input = container.querySelector('input')!;
    fireEvent.input(input, { target: { value: 'Bob' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(sent).toEqual([
      { entryId: 'gallery/Input', actionId: 'preview', value: 'Bob' },
      { entryId: 'gallery/Input', actionId: 'save', value: 'Bob', values: { name: 'Bob' } },
    ]);
  });

  it('Toggle → bool', () => {
    const { container, sent } = mount('Toggle.json');
    fireEvent.click(container.querySelector('.ui-toggle-label')!);
    expect(sent).toEqual([{ entryId: 'gallery/Toggle', actionId: 'set-verbose', value: false }]);
  });

  it('KeyHints action chip; phone hides plain hints', () => {
    const { container, sent } = mount('KeyHints.json');
    expect(container.querySelectorAll('.ui-hint')).toHaveLength(2);
    fireEvent.click(container.querySelector('.ui-hint-action')!);
    expect(sent).toEqual([{ entryId: 'gallery/KeyHints', actionId: 'new-task', value: null }]);
    cleanup();
    setShell('phone');
    const phone = mount('KeyHints.json');
    expect(phone.container.querySelectorAll('.ui-hint')).toHaveLength(1);
  });

  it('BarChart bar → "<group>:<bar>"', () => {
    const { container, sent } = mount('BarChart-vertical.json');
    fireEvent.click(container.querySelectorAll('.ui-bar-hit')[1]!);
    expect(sent).toEqual([{ entryId: 'gallery/BarChart-vertical', actionId: 'pick-bar', value: '0:1' }]);
  });
});
