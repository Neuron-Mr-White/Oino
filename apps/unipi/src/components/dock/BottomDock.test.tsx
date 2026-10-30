// @vitest-environment jsdom
/**
 * BottomDock (WP-A9, §3.11 / Z-46): dialogs dock **above** the composer and
 * never take it over — typing + Enter keep sending (a steer while the run
 * goes), Esc and the Stop button abort the run, and a taken-over session is
 * read-only (composer disabled) until Take back.
 */
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

    const input = screen.getByPlaceholderText('Keep typing to steer…') as HTMLInputElement;
    fireEvent.input(input, { target: { value: 'also check cli.ts' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => expect(cmds(w, 'steer')).toHaveLength(1));
    expect((cmds(w, 'steer')[0] as { message?: string }).message).toBe('also check cli.ts');
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
    const input = screen.getByPlaceholderText('Keep typing to steer…');
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
