// @vitest-environment jsdom
/**
 * BottomDock (WP-A9, §3.11 / Z-46): dialogs dock **above** the composer and
 * never take it over — typing + Enter keep sending (a steer while the run
 * goes), Esc and the Stop button abort the run, and a taken-over session is
 * read-only (composer disabled) until Take back.
 */
import { CRAB_WORDS } from '../../lib/status/workingLine';
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { createRoot } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createMockHost, type MockScenario, type MockWorld } from '../../dev/mock/host';
import type { RpcCommand, SessionSnapshot } from '../../lib/host/client';
import { createSession } from '../../lib/session/store';
import { clearScrollMemory } from '../../lib/timeline/scrollMemory';
import type { UiRequest } from '../../ui/types';
import { BottomDock } from './BottomDock';
import roundtrip from '../../lib/session/fixtures/roundtrip.json';

const rt = roundtrip as unknown as { fresh: SessionSnapshot };

let disposers: Array<() => void> = [];
beforeEach(() => {
  localStorage.clear();
  clearScrollMemory();
});
afterEach(() => {
  disposers.forEach((d) => d());
  disposers = [];
  cleanup();
});

function world(): MockWorld {
  const scenario: MockScenario = { name: 'dock', hosts: [{ id: 'h1', name: 'coffee', sessions: { s1: { snapshot: { ...rt.fresh, seq: 0, entries: [] } as SessionSnapshot, events: [] } } }] };
  return createMockHost(scenario);
}

async function open(w: MockWorld) {
  const { s, dispose } = createRoot((d) => ({
    s: createSession('h1', 's1', { client: w.client('h1'), schedule: (f) => f(), now: () => Date.now() }),
    dispose: d,
  }));
  disposers.push(() => {
    s.close();
    dispose();
  });
  await waitFor(() => expect(s.state.conn).toBe('live'));
  return s;
}

/** Makes the session look like a run is in flight (`agent_start`). */
function startRun(w: MockWorld, seq = 1): void {
  w.push('h1', 's1', [{ session: 's1', seq, event: { type: 'agent_start' } }]);
}

const cmds = (w: MockWorld, op: string): RpcCommand[] => w.log.filter((e) => e.kind === 'cmd' && e.op === op).map((e) => e.args as RpcCommand);

const select: UiRequest = { id: 'd1', method: 'select', title: 'Which file first?', options: ['math.ts', 'cli.ts'] };

describe('BottomDock', () => {
  it('[ask-user] a terminal-only dialog (/answer\'s editor) never opens in the dock; an ask_user form without its card stays docked', async () => {
    const w = world();
    const s = await open(w);
    const answerEditor: UiRequest = { id: 'd9', method: 'editor', extensionId: 'unipi.utility', title: 'Answer 1 question — Tab next answer · external editor available', prefill: '' };
    const tagged: UiRequest = { id: 'd8', method: 'editor', title: 'x', surfaces: ['tui'] };
    const ask: UiRequest = { id: 'd7', method: 'form', extensionId: 'unipi.ask-user', questions: [{ id: 'q0', question: 'Which DB?', options: [] }] };
    render(() => <BottomDock session={s} dialogs={[answerEditor, tagged]} onAnswer={() => {}} />);
    expect(screen.queryByTestId('dialog-panel')).toBeNull();
    expect(screen.queryByText(/Answer 1 question/)).toBeNull();
    cleanup();
    render(() => <BottomDock session={s} dialogs={[answerEditor, ask]} onAnswer={() => {}} />);
    // No timeline card for this call: the form still docks (never lost), the chip is not shown.
    expect(screen.getByTestId('dialog-panel')).toBeTruthy();
    expect(screen.queryByTestId('ask-pending')).toBeNull();
  });

  it('keeps the composer usable while a dialog waits: typing + Enter steers', async () => {
    const w = world();
    const s = await open(w);
    startRun(w);
    const answers: Array<[string, unknown]> = [];
    render(() => <BottomDock session={s} dialogs={[select]} onAnswer={(id, v) => answers.push([id, v])} />);
    // The dialog is docked above the composer, not instead of it.
    const dock = screen.getByTestId('bottom-dock');
    const dialog = screen.getByTestId('dialog-panel');
    const composer = screen.getByTestId('tl-composer');
    expect(dock.contains(dialog)).toBe(true);
    expect(dialog.compareDocumentPosition(composer) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

    const input = screen.getByPlaceholderText('Keep typing to queue…') as HTMLInputElement;
    fireEvent.input(input, { target: { value: 'also check cli.ts' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    // Z-52 (WP-A11): Enter follows the global follow-up default — `queue` →
    // a follow-up; Cmd/Ctrl+Enter is the opposite (a steer).
    await waitFor(() => expect(cmds(w, 'prompt')).toHaveLength(1));
    expect(cmds(w, 'prompt')[0]).toMatchObject({ message: 'also check cli.ts', streamingBehavior: 'followUp' });
    expect(input.value).toBe('');
    // Answering the dialog still reaches the caller (the conversation sends `extension_ui_response`).
    fireEvent.click(screen.getByText('cli.ts'));
    expect(answers).toEqual([['d1', 'cli.ts']]);
  });

  it('Esc and the Stop button abort a running turn (the dialog does not eat them)', async () => {
    const w = world();
    const s = await open(w);
    startRun(w);
    render(() => <BottomDock session={s} dialogs={[select]} onAnswer={() => {}} />);
    const input = screen.getByPlaceholderText('Keep typing to queue…');
    fireEvent.keyDown(input, { key: 'Escape' });
    await waitFor(() => expect(cmds(w, 'abort')).toHaveLength(1));
    fireEvent.click(screen.getByLabelText('Stop'));
    await waitFor(() => expect(cmds(w, 'abort')).toHaveLength(2));
  });

  it('sends a prompt when idle and shows Send, not Stop', async () => {
    const w = world();
    const s = await open(w);
    render(() => <BottomDock session={s} dialogs={[select]} onAnswer={() => {}} />);
    const input = screen.getByPlaceholderText('Message…') as HTMLInputElement;
    fireEvent.input(input, { target: { value: 'hello' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => expect(cmds(w, 'prompt')).toHaveLength(1));
    expect(screen.getByLabelText('Send')).toBeInTheDocument();
    expect(screen.queryByLabelText('Stop')).toBeNull();
  });

  it('a taken-over session is read-only: no typing, no send, no stop', async () => {
    const w = world();
    const s = await open(w);
    startRun(w);
    render(() => <BottomDock session={s} readOnly dialogs={[select]} onAnswer={() => {}} />);
    const input = screen.getByPlaceholderText('Read-only — this session is open elsewhere') as HTMLInputElement;
    expect(input).toBeDisabled();
    expect(screen.getByLabelText('Stop')).toBeDisabled();
    fireEvent.click(screen.getByLabelText('Stop'));
    await new Promise((r) => setTimeout(r, 0));
    expect(cmds(w, 'abort')).toHaveLength(0);
  });
});

describe('BottomDock · WP-A11 (queue, steering, stop)', () => {
  it('Cmd/Ctrl+Enter is the opposite of the default while running (Z-52)', async () => {
    const w = world();
    const s = await open(w);
    startRun(w);
    render(() => <BottomDock session={s} />);
    const input = screen.getByPlaceholderText('Keep typing to queue…') as HTMLInputElement;
    fireEvent.input(input, { target: { value: 'a queue default' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    fireEvent.input(input, { target: { value: 'a steer via modifier' } });
    fireEvent.keyDown(input, { key: 'Enter', metaKey: true });
    fireEvent.input(input, { target: { value: 'a steer via ctrl' } });
    fireEvent.keyDown(input, { key: 'Enter', ctrlKey: true });
    await waitFor(() => expect(cmds(w, 'prompt')).toHaveLength(3));
    expect(cmds(w, 'prompt')[0]).toMatchObject({ message: 'a queue default', streamingBehavior: 'followUp' });
    expect(cmds(w, 'prompt')[1]).toMatchObject({ message: 'a steer via modifier', streamingBehavior: 'steer' });
    expect(cmds(w, 'prompt')[2]).toMatchObject({ message: 'a steer via ctrl', streamingBehavior: 'steer' });
  });

  it('the chevron menu switches the default and "Send now (stops run)" aborts then prompts', async () => {
    const w = world();
    const s = await open(w);
    startRun(w);
    render(() => <BottomDock session={s} />);
    fireEvent.click(screen.getByTestId('send-chevron'));
    fireEvent.click(screen.getByTestId('send-mode-steer'));
    const input = screen.getByPlaceholderText('Keep typing to steer…') as HTMLInputElement;
    fireEvent.input(input, { target: { value: 'now the default is steer' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => expect(cmds(w, 'prompt')).toHaveLength(1));
    expect(cmds(w, 'prompt')[0]).toMatchObject({ message: 'now the default is steer', streamingBehavior: 'steer' });

    // "Send now (stops run)": abort, then send the text as a prompt once idle.
    fireEvent.input(input, { target: { value: 'right now' } });
    fireEvent.click(screen.getByTestId('send-chevron'));
    fireEvent.click(screen.getByTestId('send-now'));
    await waitFor(() => expect(cmds(w, 'abort')).toHaveLength(1));
    await waitFor(() => expect(cmds(w, 'prompt')).toHaveLength(2));
    expect(cmds(w, 'prompt')[1]).toMatchObject({ message: 'right now' });
  });

  it('Esc stops from any focus in the conversation, but a docked dialog consumes it (Z-55)', async () => {
    const w = world();
    const s = await open(w);
    startRun(w);
    const { container } = render(() => (
      <div class="conv-tl">
        <BottomDock session={s} dialogs={[select]} onAnswer={() => {}} />
      </div>
    ));
    // Any focus inside the conversation element — not just the dock.
    const outside = document.createElement('button');
    container.querySelector('.conv-tl')!.appendChild(outside);
    fireEvent.keyDown(outside, { key: 'Escape' });
    await waitFor(() => expect(cmds(w, 'abort')).toHaveLength(1));
    // The docked dialog takes the Esc itself: the run is untouched from there.
    fireEvent.keyDown(screen.getByTestId('dialog-panel'), { key: 'Escape' });
    await new Promise((r) => setTimeout(r, 0));
    expect(cmds(w, 'abort')).toHaveLength(1);
  });

  it('renders the queue panel above the composer by default (Z-53)', async () => {
    const w = world();
    const s = await open(w);
    startRun(w);
    w.push('h1', 's1', [{ session: 's1', seq: 2, event: { type: 'queue_update', steering: ['Keep it small'], followUp: [] } }]);
    render(() => <BottomDock session={s} />);
    const queue = await screen.findByTestId('queue-panel');
    const composer = screen.getByTestId('tl-composer');
    expect(queue.compareDocumentPosition(composer) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(screen.getByText('Keep it small')).toBeTruthy();
  });
});

describe('[working-line] the one line above the composer', () => {
  it('shows Working while a run streams, above the composer; nothing when idle; no Subagents pill', async () => {
    const w = world();
    const s = await open(w);
    render(() => <BottomDock session={s} />);
    expect(screen.queryByTestId('working-line')).toBeNull();
    startRun(w);
    const line = await screen.findByTestId('working-line');
    expect(line.getAttribute('data-kind')).toBe('working');
    const wl = screen.getByTestId('working-line-text');
    expect(wl.getAttribute('title')).toBe('Working…');
    expect(CRAB_WORDS).toContain(wl.textContent);
    expect(screen.getByTestId('working-line-text').classList.contains('shimmer')).toBe(true);
    const composer = screen.getByTestId('tl-composer');
    expect(line.compareDocumentPosition(composer) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });
  it('a running-work summary is a quiet line with Open when the agent is idle', async () => {
    const w = world();
    const s = await open(w);
    render(() => <BottomDock session={s} trayLabel="Subagents 1 running" />);
    const line = await screen.findByTestId('working-line');
    expect(line.getAttribute('data-kind')).toBe('background');
    expect(screen.getByTestId('work-tray-button')).toBeTruthy();
    // Owner 2026-11-05: background work (a subagent running) animates too — spinning ring + shimmer.
    expect(screen.getByTestId('working-line-text').classList.contains('shimmer')).toBe(true);
  });
  it('[owner-fix-2] a docked approval while the run goes reads Needs you, not Working (like the sidebar)', async () => {
    const w = world();
    const s = await open(w);
    startRun(w);
    const approval: UiRequest = { id: 'a1', method: 'approval', title: 'bash', message: 'ls' } as unknown as UiRequest;
    render(() => <BottomDock session={s} dialogs={[approval]} onAnswer={() => {}} />);
    const line = await screen.findByTestId('working-line');
    expect(line.getAttribute('data-kind')).toBe('needs-you');
    expect(line.textContent).toContain('Approval waiting');
    expect(screen.queryByTestId('working-line-time')).toBeNull();
    expect(screen.getByTestId('dialog-pending')).toBeTruthy();
  });
});
