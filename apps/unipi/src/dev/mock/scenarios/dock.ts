/**
 * Dock, dialog, error and takeover scenarios (WP-A9):
 *
 *   dialog-dock      two pi dialogs waiting in the dock ("1 of 2") while a run
 *                    streams; the composer stays usable
 *   dialog-approval  an `approval` card from `unipi.workflow` with a code
 *                    detail (40 vh), allow once / always / deny / deny+note
 *   error-row        a turn that failed with a raw provider error (Details)
 *   taken-over      another runner took the session over: read-only banner,
 *                    Take back; the composer is disabled
 *   crash-boundary   a side-pane tab that throws (boundary card) next to a
 *                    conversation that keeps streaming
 *
 * A dialog is an `extension_ui_request` event (pi shape); its answer goes back
 * as `extension_ui_response` on the session stream.
 */
import { TriangleAlert } from 'lucide-solid';
import type { MockScenario, MockSession, MockStep } from '../host';
import type { SessionEventDto } from '../../../lib/host/client';
import { genCards, genFsTree, genLiveRun, genSession } from '../gen';
import { registerPaneKind, setPaneComponent } from '../../../lib/panes/kinds';

const HOST = 'pc-coffee';
const CWD = '/home/you/projects/unipi';
const cards = genCards(6, { seed: 5 });

/** One session script: numbered events after the snapshot (seq 1…). */
function scripted(session: string, events: Array<SessionEventDto['event']>, from = 1, at0 = 0): MockStep[] {
  let at = at0;
  return events.map((event, i) => {
    at += 140;
    return { at, dto: { session, seq: from + i, event } };
  });
}

/** A run that is still going plus dialog requests arriving during it. */
function sessionWithRun(id: string, dialogs: Array<SessionEventDto['event']>): MockSession {
  const base = genSession(id, { seed: 8, turns: 3, toolsPerTurn: 2, thinking: true, cwd: CWD });
  const run = genLiveRun(id, { seq: 1, running: true, at: 0 });
  const lastAt = run.at(-1)?.at ?? 0;
  const dialogSteps = scripted(id, dialogs, run.length + 1, lastAt);
  return { ...base, events: [...run, ...dialogSteps] };
}

/** A provider error with the trace the Details disclosure shows raw (Z-48). */
export const RAW_ERROR = [
  'overloaded_error: the model is overloaded (529)',
  '',
  'request-id: req_011Cb3f2a9e04b1c',
  'retry-after: 12s',
  '{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"},"provider":"anthropic","model":"claude-sonnet-4-5"}',
].join('\n');

function failedSession(id: string): MockSession {
  const s = genSession(id, { seed: 6, turns: 3, toolsPerTurn: 2, thinking: true, cwd: CWD });
  const entries = s.snapshot.entries as Array<{ message?: { stopReason?: string; errorMessage?: string } }>;
  const last = entries.at(-1);
  if (last?.message) {
    last.message.stopReason = 'error';
    last.message.errorMessage = RAW_ERROR;
  }
  return s;
}

const select: SessionEventDto['event'] = {
  type: 'extension_ui_request',
  id: 'ask-1',
  method: 'select',
  title: 'Which file should I read first?',
  options: ['src/math.ts — the failing test', 'src/cli.ts — the flag parser', 'README.md — the docs'],
};

const confirm: SessionEventDto['event'] = {
  type: 'extension_ui_request',
  id: 'ask-2',
  method: 'confirm',
  title: 'Overwrite README.md?',
  message: 'The file already has 42 lines and three sections.',
};

const approval: SessionEventDto['event'] = {
  type: 'extension_ui_request',
  id: 'ap-1',
  method: 'approval',
  extensionId: 'unipi.workflow',
  toolName: 'bash',
  summary: 'npm test -- --run',
  title: 'Allow bash: npm test -- --run?',
  detail: {
    t: 'Code',
    language: 'sh',
    lineNumbers: false,
    source: '# the workflow asks before running the suite\nnpm test -- --run\n\n# 9 files, 41 tests — writes nothing outside ./tmp',
  },
  options: [
    { optionId: 'allow_once', label: 'Allow once', kind: 'allow_once' },
    { optionId: 'allow_always', label: 'Always allow `npm test`', kind: 'allow_always' },
    { optionId: 'deny', label: 'Deny', kind: 'reject_once' },
    { optionId: 'deny_note', label: 'Deny with note…', kind: 'reject_once' },
  ],
  feedback: { placeholder: 'Why deny? (sent to the agent)' },
};

export const dialogDock: MockScenario = {
  name: 'dialog-dock',
  hosts: [
    {
      id: HOST,
      name: 'coffee',
      conn: { state: 'connected' },
      cards,
      fs: genFsTree(CWD),
      sessions: { s001: sessionWithRun('s001', [select, confirm]) },
    },
  ],
};

export const dialogApproval: MockScenario = {
  name: 'dialog-approval',
  hosts: [
    {
      id: HOST,
      name: 'coffee',
      conn: { state: 'connected' },
      cards,
      fs: genFsTree(CWD),
      sessions: { s001: sessionWithRun('s001', [approval]) },
    },
  ],
};

export const errorRow: MockScenario = {
  name: 'error-row',
  hosts: [
    {
      id: HOST,
      name: 'coffee',
      conn: { state: 'connected' },
      cards,
      fs: genFsTree(CWD),
      sessions: { s001: failedSession('s001') },
    },
  ],
};

export const takenOver: MockScenario = {
  name: 'taken-over',
  hosts: [
    {
      id: HOST,
      name: 'coffee',
      conn: { state: 'connected' },
      cards,
      fs: genFsTree(CWD),
      sessions: {
        s001: {
          ...genSession('s001', { seed: 4, turns: 3, toolsPerTurn: 1, thinking: true, cwd: CWD }),
          // Another runner opened the session: the host takes it from this stream.
          events: scripted('s001', [{ kind: 'taken_over', from: 'tui' }]),
        },
      },
    },
  ],
};

/** A side-pane tab that throws on render (`crash-boundary`, Z-89). */
function ThrowingTab(): never {
  throw new Error('The kanboard bridge returned an unexpected payload');
}

export const crashBoundary: MockScenario = {
  name: 'crash-boundary',
  hosts: [
    {
      id: HOST,
      name: 'coffee',
      conn: { state: 'connected' },
      cards,
      fs: genFsTree(CWD),
      sessions: {
        s001: { ...genSession('s001', { seed: 4, turns: 3, toolsPerTurn: 1, thinking: true, cwd: CWD }), events: genLiveRun('s001', { seq: 1, running: true }) },
      },
    },
  ],
  speed: 1,
  setup: () => {
    registerPaneKind({
      kind: 'crash-demo',
      icon: TriangleAlert,
      title: () => 'Board',
      singleton: true,
      scope: 'project',
      component: ThrowingTab as never,
    });
    setPaneComponent('crash-demo', ThrowingTab as never);
    localStorage.setItem('unipi.shell.layout', JSON.stringify({ sidebarOpen: true, sidebarW: 264, sideOpen: true, sideW: 480, bottomOpen: false, bottomH: 220 }));
    localStorage.setItem('unipi.shell.last', JSON.stringify({ hostId: HOST, sessionId: 's001' }));
    localStorage.setItem(`unipi.panes:${HOST}|${CWD}`, JSON.stringify({ side: { tabs: [{ id: 'tc', kind: 'crash-demo' }], active: { s001: 'tc' }, closed: [] }, bottom: { tabs: [], active: {}, closed: [] } }));
  },
};
