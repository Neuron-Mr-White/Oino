// @vitest-environment jsdom
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { createStore } from 'solid-js/store';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { PermissionControl } from './PermissionControl';
import { ApprovalCard } from './ApprovalCard';
import { RenderCutoff } from '../chat/RenderCutoff';
import { buildTurns, createTurnsIndex, type TurnsInput } from '../../lib/session/turns';
import { applyEvent, initialState } from '../../lib/session/reducer';
import type { SessionHandle } from '../../lib/session/store';
import type { ApprovalRequest } from '../../lib/session/autonomy';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri')), transformCallback: () => 0 }));
afterEach(cleanup);
function handle() {
  const [state, set] = createStore({ ...initialState('h', 's'), conn: 'live' as const, info: { autonomy: 'low' } });
  const cmd = vi.fn(async (command) => { if (command.type === 'set_autonomy') set('info', 'autonomy', command.level); });
  return { state, cmd, key: 'h/s', hostId: 'h', sessionId: () => 's', acquire: () => () => {}, subscribeTopics: () => {} } as unknown as SessionHandle;
}
const request: ApprovalRequest = { requestId: 'r', sessionId: 's', tool: 'bash', arguments: { command: 'git push', summary: 'Push branch' }, riskLevel: 'high', ceiling: 'medium', reason: 'needs Autonomy High' };
describe('v3 autonomy', () => {
  it('cycles_levels_and_tooltips', async () => {
    const session = handle();
    const view = render(() => <PermissionControl session={session} onCommand={() => {}} />);
    const chip = view.getByTestId('permission-chip');
    for (const [level, bars] of [['low', 1], ['medium', 2], ['high', 3], ['low', 1]] as const) {
      await waitFor(() => expect(chip.querySelectorAll('.autonomy-bar[data-filled]')).toHaveLength(bars));
      expect(chip.querySelectorAll('.autonomy-bar')).toHaveLength(3);
      expect(chip.getAttribute('data-tip')).toBe(`Autonomy: ${level === 'low' ? 'Low (read-only commands)' : level === 'medium' ? 'Medium (reversible commands)' : 'High (allow all commands)'}`);
      if (vi.mocked(session.cmd).mock.calls.length < 3) fireEvent.click(chip);
    }
    expect(vi.mocked(session.cmd).mock.calls.map(([c]) => c)).toEqual([{ type: 'set_autonomy', level: 'medium' }, { type: 'set_autonomy', level: 'high' }, { type: 'set_autonomy', level: 'low' }]);
  });
  it('does not focus the approval card or approve on Enter or stop on Escape', () => {
    const session = handle();
    const view = render(() => <ApprovalCard session={session} request={request} />);
    const card = view.getByTestId('approval-card');
    expect(document.activeElement).not.toBe(card);
    let escaped = false;
    view.container.addEventListener('keydown', () => { escaped = true; });
    fireEvent.keyDown(card, { key: 'Escape' });
    expect(escaped).toBe(false);
    fireEvent.keyDown(card, { key: 'Enter' });
    expect(session.cmd).not.toHaveBeenCalled();
  });
  it('approval_allow_once_always_deny', async () => {
    for (const [label, decision] of [['Allow once', 'allow_once'], ['Always allow', 'always'], ['Deny', 'deny']]) {
      const session = handle();
      const view = render(() => <ApprovalCard session={session} request={request} />);
      fireEvent.click(view.getByText(label));
      fireEvent.click(view.getByText(label));
      await waitFor(() => expect(session.cmd).toHaveBeenCalledTimes(1));
      expect(session.cmd).toHaveBeenCalledWith({ type: 'approval_response', requestId: 'r', decision });
      cleanup();
    }
  });
  it('approval_correlation_and_disconnect', () => {
    const state = initialState('h', 's');
    applyEvent(state, { session: 'wrong', seq: 1, event: { type: 'approval_request', ...request } });
    expect(state.approvals).toBeUndefined();
    applyEvent(state, { session: 's', seq: 2, event: { type: 'approval_request', ...request } });
    applyEvent(state, { session: 's', seq: 3, event: { type: 'approval_request', ...request } });
    expect(state.approvals).toHaveLength(1);
    const original = handle();
    const session = { ...original, state: { ...original.state, conn: 'reconnecting' as const } };
    const view = render(() => <ApprovalCard session={session} request={request} />);
    fireEvent.click(view.getByText('Allow once'));
    expect(session.cmd).not.toHaveBeenCalled();
  });
  it('foreground_child_approval_is_answered_on_parent_channel', async () => {
    const state = initialState('h', 's');
    applyEvent(state, { session: 's', seq: 1, event: { type: 'approval_request', ...request, sessionId: 'child-session' } });
    expect(state.approvals).toHaveLength(1);
    const session = handle();
    const view = render(() => <ApprovalCard session={session} request={state.approvals![0]!} />);
    fireEvent.click(view.getByText('Allow once'));
    await waitFor(() => expect(session.cmd).toHaveBeenCalledWith({ type: 'approval_response', requestId: 'r', decision: 'allow_once' }));
  });
  it('render_cutoff_preserves_history', () => {
    const entries = [
      { type: 'message', id: 'u1', parentId: null, message: { role: 'user', content: 'old question' } },
      { type: 'message', id: 'a1', parentId: 'u1', message: { role: 'assistant', content: 'old answer', stopReason: 'stop' } },
      { type: 'message', id: 'u2', parentId: 'a1', message: { role: 'user', content: 'kept question' } },
      { type: 'message', id: 'a2', parentId: 'u2', message: { role: 'assistant', content: 'kept answer', stopReason: 'stop' } },
      { type: 'compaction_state', id: 'c', parentId: 'a2', summaryText: 'summary', anchorMessage: { id: 'u2', index: 2 }, removedCount: 2 },
    ];
    const before = JSON.stringify(entries);
    const input: TurnsInput = { entries, entriesRev: 0, leafId: 'c', liveTools: {}, run: { running: false } };
    for (const turns of [buildTurns(input), createTurnsIndex().update(input)]) {
      expect(turns[0]!.user?.message?.content).toBe('old question');
      expect(turns.find((t) => t.renderCutoff)?.user?.id).toBe('u2');
      expect(turns.flatMap((t) => t.entries)).toHaveLength(5);
    }
    expect(JSON.stringify(entries)).toBe(before);
    const view = render(() => <RenderCutoff />);
    expect(view.getByRole('separator').textContent).toContain('earlier history preserved');
  });
});
