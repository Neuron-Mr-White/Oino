/**
 * U10 slot + dialog scenarios (WP-U10 acceptance: open/update/close/dispose/
 * order/two contributors per slot; G-1/G-4 of zcode-audit).
 *
 * One registry snapshot carries every slot — four `widget` contributors (two
 * orders), five `status` segments + a `footer` tree (the strip's caps), the
 * `header` band, two `sidePane` tabs, a `bottomPane`, a `fullscreen` page, a
 * `turnFooter` renderer and an `overlay` panel. The variants differ only in
 * the scripted patches, so a capture shows one behaviour at a time:
 *
 *   slots           update (meta retitle + badge, tree op) then the overlay's
 *                   close (del) — the baseline conversation
 *   slots-overlay   the same update, overlay left open + a `notify` effect
 *   slots-tray      the same close + an `editorReplace` tray taking the
 *                   composer's place (§3.11)
 *   slots-dialogs   the widened Oino dialog bodies in the dock: `custom`
 *                   (spec tree with a resolving button) queued behind a
 *                   `multiSelect`; `editExternally` arrives last
 */
import type { MockScenario, MockSession, MockStep } from '../host';
import type { SessionEventDto, SessionSnapshot } from '../../../lib/host/client';

type SnapshotUi = NonNullable<SessionSnapshot['ui']>;
import { genCards, genFsTree, genSession } from '../gen';

const HOST = 'pc-coffee';
const CWD = '/home/you/projects/unipi';
const cards = genCards(6, { seed: 5 });

/** Numbered patch events after the snapshot (seq 1…). */
function patches(session: string, ops: Array<Record<string, unknown>>): MockStep[] {
  return ops.map((patch, i) => ({ at: 200 + i * 160, dto: { session, seq: i + 1, event: { type: 'oino_ui_patch', ...patch } } }));
}

/** A `Turn`-footer render answer: the files a turn touched (A27's row). */
const TURN_FOOTER_NODE = {
  t: 'Paragraph',
  text: { lines: [{ spans: ['edited ', { content: 'src/ui/slots/WidgetStack.tsx', style: { fg: 'link' } }, '  ·  +84 −12'] }] },
};

function baseSession(id: string, ui?: SnapshotUi, events: MockStep[] = []): MockSession {
  const base = genSession(id, { seed: 9, turns: 4, toolsPerTurn: 2, thinking: true, cwd: CWD });
  return ui ? { ...base, snapshot: { ...base.snapshot, ui }, events } : { ...base, events };
}

/** The registry snapshot every slot host renders from. */
function uiSnapshot(sessionId: string): SnapshotUi {
  return {
    scope: 'session',
    sessionId,
    rev: 1,
    dialogs: [],
    meta: {},
    entries: [
      // ── widget slot: two contributors, `order` decides (§6.4) ──
      {
        id: 'utility/widget:sysinfo',
        extensionId: 'utility',
        scope: 'session',
        slot: 'widget',
        order: 10,
        seq: 2,
        rev: 1,
        title: 'System',
        view: {
          kind: 'tree',
          root: {
            t: 'Layout',
            direction: 'Horizontal',
            spacing: 2,
            constraints: [{ Length: 20 }, { Length: 14 }],
            children: [
              { t: 'Paragraph', text: 'CPU 62 %' },
              { t: 'Gauge', ratio: 0.62, label: 'load' },
            ],
          },
        },
      },
      {
        id: 'unipi.fusion/widget:wake',
        extensionId: 'unipi.fusion',
        scope: 'session',
        slot: 'widget',
        order: 0,
        seq: 1,
        rev: 1,
        view: { kind: 'lines', lines: ['fusion: 2 background jobs · next wake in 4 m'] },
      },
      {
        id: 'unipi.watchdog/widget:caps',
        extensionId: 'unipi.watchdog',
        scope: 'session',
        slot: 'widget',
        order: -10,
        seq: 13,
        rev: 1,
        view: { kind: 'lines', lines: ['watchdog: 12 turns · $0.42 · cap 40 turns'] },
      },
      {
        id: 'unipi.memory/widget:recall',
        extensionId: 'unipi.memory',
        scope: 'session',
        slot: 'widget',
        order: -20,
        seq: 14,
        rev: 1,
        view: { kind: 'lines', lines: ['memory: 3 recall reminders queued'] },
      },
      // ── status slot (G-4) + a footer entry folded into the strip ──
      { id: 'utility/status:git', extensionId: 'utility', scope: 'session', slot: 'status', order: 20, seq: 3, rev: 1, view: { kind: 'status', text: 'main ↑1 ↓0' }, tone: 'success' },
      { id: 'unipi.fusion/status:jobs', extensionId: 'unipi.fusion', scope: 'session', slot: 'status', order: 10, seq: 4, rev: 1, view: { kind: 'status', text: 'fusion 2 jobs' }, tone: 'info' },
      { id: 'unipi.memory/status:memory', extensionId: 'unipi.memory', scope: 'session', slot: 'status', order: -10, seq: 15, rev: 1, view: { kind: 'status', text: 'memory 41 items' } },
      { id: 'unipi.watchdog/status:watchdog', extensionId: 'unipi.watchdog', scope: 'session', slot: 'status', order: -20, seq: 16, rev: 1, view: { kind: 'status', text: 'watchdog: 12' }, tone: 'warning' },
      { id: 'unipi.long-horizon/status:goal', extensionId: 'unipi.long-horizon', scope: 'session', slot: 'status', order: -30, seq: 17, rev: 1, view: { kind: 'status', text: 'goal 4/7 · running' }, tone: 'success' },
      {
        id: 'utility/footer:ctx',
        extensionId: 'utility',
        scope: 'session',
        slot: 'footer',
        order: 0,
        seq: 5,
        rev: 1,
        view: {
          kind: 'tree',
          root: { t: 'Paragraph', text: 'ctx 41 % · $0.012' },
        },
      },
      // ── header slot: the band above the transcript ──
      {
        id: 'workflow/header:plan',
        extensionId: 'workflow',
        scope: 'session',
        slot: 'header',
        order: 0,
        seq: 6,
        rev: 1,
        view: { kind: 'tree', root: { t: 'Paragraph', text: 'Plan: 3 of 7 steps · verifying the slot hosts' } },
      },
      // ── sidePane: two extensions, badges + focus (§6.5) ──
      {
        id: 'unipi.kanboard/panel-board',
        extensionId: 'unipi.kanboard',
        scope: 'session',
        slot: 'sidePane',
        order: 0,
        seq: 7,
        rev: 1,
        title: 'Board panel',
        icon: 'board',
        view: { kind: 'tree', root: { t: 'List', items: [{ key: 't1', content: 'T-141 Fix slot hosts', badge: 'doing' }, { key: 't2', content: 'T-142 Board panel polish' }] } },
      },
      {
        id: 'utility/panel:notes',
        extensionId: 'utility',
        scope: 'session',
        slot: 'sidePane',
        order: 50,
        seq: 8,
        rev: 1,
        title: 'Notes',
        icon: 'list',
        badge: '3',
        focus: true,
        view: { kind: 'tree', root: { t: 'Paragraph', text: 'Three scratch notes live in this panel; the first is open.' } },
      },
      // ── bottomPane: a build log ──
      {
        id: 'utility/panel:build',
        extensionId: 'utility',
        scope: 'session',
        slot: 'bottomPane',
        order: 0,
        seq: 9,
        rev: 1,
        title: 'Build',
        icon: 'terminal',
        view: { kind: 'tree', root: { t: 'Log', lines: ['cargo build --release', 'Compiling unipi-host v2', 'Finished in 41 s'], maxLines: 200 } },
      },
      // ── fullscreen: the sidebar page ──
      {
        id: 'utility/page:usage',
        extensionId: 'utility',
        scope: 'session',
        slot: 'fullscreen',
        order: 0,
        seq: 10,
        rev: 1,
        title: 'Usage',
        icon: 'chart',
        view: {
          kind: 'tree',
          root: {
            t: 'Block',
            title: 'This week',
            borders: ['TOP'],
            child: { t: 'BarChart', data: [{ bars: [{ value: 12, label: 'Mon' }, { value: 19, label: 'Tue' }, { value: 9, label: 'Wed' }, { value: 22, label: 'Thu' }] }] },
          },
        },
      },
      // ── turnFooter: a renderer entry per visible turn (§6.7) ──
      { id: 'unipi.file-summary/turnFooter:files', extensionId: 'unipi.file-summary', scope: 'session', slot: 'turnFooter', order: 0, seq: 11, rev: 1, view: { kind: 'renderer', key: 'files' } },
      // ── overlay: the modal (a later patch closes it) ──
      {
        id: 'utility/panel:help',
        extensionId: 'utility',
        scope: 'session',
        slot: 'overlay',
        order: 0,
        seq: 12,
        rev: 1,
        title: 'Slot map',
        size: 'medium',
        keys: [{ key: 'esc', label: 'Esc close' }],
        view: { kind: 'tree', root: { t: 'Paragraph', text: 'Overlay panels centre here; the dock below stays usable.' } },
      },
    ],
  };
}

/** The three patch steps every variant shares: a tab retitle + badge, then a widget tree update. */
const UPDATE_STEPS: Array<Record<string, unknown>> = [
  // update: retitle + rebadge the Notes tab (meta op bumps rev)
  { scope: 'session', rev: 2, ops: [{ op: 'meta', id: 'utility/panel:notes', set: { title: 'Scratch notes', badge: '5' } }] },
  // update: the CPU widget's tree changes (tree op)
  {
    scope: 'session',
    rev: 3,
    ops: [{ op: 'tree', id: 'utility/widget:sysinfo', rev: 2, ops: [{ op: 'set', path: ['#1'], props: { ratio: 0.81, label: 'load' } }] }],
  },
];

/** close: the overlay panel goes away (§6.6). */
const CLOSE_OVERLAY: Record<string, unknown> = { scope: 'session', rev: 4, ops: [{ op: 'del', id: 'utility/panel:help' }] };

/** open: the work tray takes the composer's place (`editorReplace`, §3.11). */
const OPEN_TRAY: Record<string, unknown> = {
  scope: 'session',
  ops: [
    {
      op: 'put',
      entry: {
        id: 'core/panel:work-tray',
        extensionId: 'core',
        scope: 'session',
        slot: 'editorReplace',
        order: 0,
        seq: 18,
        rev: 1,
        title: 'Work tray',
        view: { kind: 'tree', root: { t: 'Paragraph', text: 'Background work runs here — Esc returns your composer.' } },
      },
    },
  ],
};

/** one-shot effect: a warning toast (§3.2 `notify`). */
const NOTIFY: Record<string, unknown> = { scope: 'session', ops: [{ op: 'effect', request: { id: 'n1', method: 'notify', message: 'compactor: context trimmed 12 %', notifyType: 'warning' } }] };

/** Renumbers the steps from the snapshot's rev 1 (the wire requires `rev` = last + 1 in order). */
function steps(ops: Array<Record<string, unknown>>): Array<Record<string, unknown>> {
  return ops.map((op, i) => ({ ...op, rev: i + 2 }));
}

/** One variant of the slot scenario: the same snapshot, a different patch script. */
function slotVariant(name: string, ops: Array<Record<string, unknown>>, layout?: Record<string, unknown>): MockScenario {
  return {
    name,
    hosts: [
      {
        id: HOST,
        name: 'coffee',
        conn: { state: 'connected' },
        cards,
        fs: genFsTree(CWD),
        renderers: { 'turnFooter/files': TURN_FOOTER_NODE },
        sessions: { s001: baseSession('s001', uiSnapshot('s001'), patches('s001', steps(ops))) },
      },
    ],
    speed: 1,
    setup: () => {
      localStorage.setItem(
        'unipi.shell.layout',
        JSON.stringify({ sidebarOpen: true, sidebarW: 264, sideOpen: true, sideW: 460, bottomOpen: true, bottomH: 200, ...layout }),
      );
      localStorage.setItem('unipi.shell.last', JSON.stringify({ hostId: HOST, sessionId: 's001' }));
    },
  };
}

/** Baseline conversation: the update patches, then the overlay's close. */
export const slots = slotVariant('slots', [...UPDATE_STEPS, CLOSE_OVERLAY]);

/** The overlay left open (a modal / phone sheet over the dock). */
export const slotsOverlay = slotVariant('slots-overlay', [...UPDATE_STEPS]);

/** The `editorReplace` tray taking the composer's place + a notify toast. */
export const slotsTray = slotVariant('slots-tray', [...UPDATE_STEPS, CLOSE_OVERLAY, OPEN_TRAY, NOTIFY]);

/** The widened dialog bodies: custom (spec) + multiSelect queued, editExternally last. */
const CUSTOM_DIALOG: SessionEventDto['event'] = {
  type: 'extension_ui_request',
  id: 'cd-1',
  method: 'custom',
  title: 'Compact the session?',
  size: 'small',
  timeout: 0,
  spec: {
    t: 'Layout',
    direction: 'Vertical',
    spacing: 2,
    children: [
      { t: 'Paragraph', text: 'The context is 84 % full. Compact now and keep working?' },
      {
        t: 'Layout',
        direction: 'Horizontal',
        spacing: 2,
        children: [
          { t: 'Button', label: 'Compact now', action: 'compact-now', variant: 'Primary' },
          { t: 'Button', label: 'Keep going', action: 'keep-going', variant: 'Ghost' },
        ],
      },
    ],
  },
};

const MULTISELECT_DIALOG: SessionEventDto['event'] = {
  type: 'extension_ui_request',
  id: 'ms-1',
  method: 'multiSelect',
  title: 'Which panels should auto-open?',
  options: [
    { label: 'Board panel', value: 'board', description: 'unipi.kanboard' },
    { label: 'Build log', value: 'build', description: 'utility' },
    { label: 'Notes', value: 'notes', description: 'utility' },
  ],
  min: 1,
  max: 2,
};

const EDIT_DIALOG: SessionEventDto['event'] = {
  type: 'extension_ui_request',
  id: 'ed-1',
  method: 'editExternally',
  text: '# Notes\n\n- widgets fold on the phone\n',
  extensionHint: 'md',
};

export const slotsDialogs: MockScenario = {
  name: 'slots-dialogs',
  hosts: [
    {
      id: HOST,
      name: 'coffee',
      conn: { state: 'connected' },
      cards,
      fs: genFsTree(CWD),
      sessions: {
        s001: baseSession('s001', undefined, [
          { at: 200, dto: { session: 's001', seq: 1, event: CUSTOM_DIALOG } },
          { at: 400, dto: { session: 's001', seq: 2, event: MULTISELECT_DIALOG } },
          { at: 600, dto: { session: 's001', seq: 3, event: EDIT_DIALOG } },
        ]),
      },
    },
  ],
  speed: 1,
  setup: () => {
    localStorage.setItem('unipi.shell.last', JSON.stringify({ hostId: HOST, sessionId: 's001' }));
  },
};
