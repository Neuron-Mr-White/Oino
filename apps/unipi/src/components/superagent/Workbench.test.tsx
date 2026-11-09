// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { setHostClientFactory } from '../../lib/host/factory';
import { createMockHost } from '../../dev/mock/host';
import { superagentLeads } from '../../dev/mock/scenarios/superagent-leads';
import { Root } from '../../main';
import { resetSessions } from '../../lib/session/store';
import { resetHubs } from '../../lib/session/hub';
import { resetLayout, setDesktopOverride } from '../../lib/layout';
import { setPlatformOverride } from '../../lib/platform';
import { boardUi } from '../../lib/board/ui';
import { endQueue } from '../../lib/board/taskOrigin';
import { StatusSchema, superagentApi } from './api';
import { CLO6_QUESTION, CLO6_REPORT } from './clo6.fixture';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri')), transformCallback: () => 0 }));
afterEach(() => { cleanup(); vi.restoreAllMocks(); setHostClientFactory(undefined); resetSessions(); resetHubs(); resetLayout(); setDesktopOverride(undefined); setPlatformOverride(undefined); boardUi().closeTask(); endQueue(); location.hash = ''; });

const plain = (text: string) => text.replace(/`/g, '');
const status = () => StatusSchema.parse({
  enabled: true, leads: [],
  needsYou: [
    { projectId: 'cloud', taskId: 'CLO-6', status: 'blocked', question: CLO6_REPORT },
    { projectId: 'website', taskId: 'WEB-8', status: 'blocked', question: 'Which domain should the staging deploy use?' },
  ],
  review: [{ projectId: 'heartpick', taskId: 'HP-3', status: 'in_review' }],
  projects: [{ projectId: 'cloud', name: 'Cloud', autoStart: false }, { projectId: 'website', name: 'Website', autoStart: true }, { projectId: 'heartpick', name: 'Heartpick', autoStart: true }],
});
function mount(phone = false) {
  setHostClientFactory(createMockHost(superagentLeads).client);
  setPlatformOverride(phone ? 'tauri-mobile' : 'tauri-desktop'); setDesktopOverride(!phone);
  vi.spyOn(superagentApi, 'status').mockImplementation(async () => ({ ok: true, value: status() }));
  location.hash = '#/superagent?session=superagent-chat&host=pc-coffee';
  render(() => <Root />);
}
async function openEscalate() {
  fireEvent.click(await screen.findByLabelText('Open escalations (2)'));
  return await screen.findByTestId('sa-escalation');
}
const before = (a: Node, b: Node) => !!(a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING);

describe('OIN-62: the workbench reads a blocked ticket as a decision', () => {
  it('the capsule carries the counts in its label', async () => {
    mount();
    await screen.findByLabelText('Open escalations (2)');
    await waitFor(() => expect(screen.getByRole('group', { name: /Workbench/ }).getAttribute('aria-label')).toBe('Workbench: 0 to approve, 1 to review, 2 to escalate'));
  });

  it('pins CLO-6\'s question first, with the one-line reason, and keeps the report behind "Read full report"', async () => {
    mount();
    const card = await openEscalate();
    const ask = within(card).getByTestId('sa-ask');
    expect(ask.textContent).toContain('Need from you');
    expect(ask.textContent).toContain(plain(CLO6_QUESTION));
    expect(within(card).getByTestId('sa-question-reason').textContent).toBe('CLO-6 — Invoice service v2: build report');
    // Nothing of the report is in the page yet: the question and the reply box are all there is to read.
    expect(within(card).queryByTestId('sa-report')).toBeNull();
    expect(card.textContent).not.toContain('Idempotency-Key');
    const toggle = within(card).getByRole('button', { name: 'Read full report' });
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    // Order: ask → reply → toggle.
    const reply = within(card).getByLabelText('Reply to CLO-6');
    expect(before(ask, reply)).toBe(true);
    expect(before(reply, toggle)).toBe(true);
    fireEvent.click(toggle);
    const report = await within(card).findByTestId('sa-report');
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    expect(toggle.getAttribute('aria-controls')).toBe(report.id);
    expect(before(toggle, report)).toBe(true);
    // Rendered Markdown, not a wall of text; the question is not repeated inside it.
    expect(report.querySelector('table')).toBeTruthy();
    expect(report.querySelector('h2')?.textContent).toBe('Status');
    expect(report.textContent).not.toContain('Need from you');
    fireEvent.click(toggle);
    expect(within(card).queryByTestId('sa-report')).toBeNull();
  });

  it('a short question has no report and no reason line', async () => {
    mount();
    await openEscalate();
    fireEvent.click(screen.getByLabelText('Next question'));
    const card = await screen.findByTestId('sa-question');
    expect(within(card).getByTestId('sa-ask').textContent).toContain('Which domain should the staging deploy use?');
    expect(within(card).queryByRole('button', { name: /full report/ })).toBeNull();
    expect(within(card).queryByTestId('sa-question-reason')).toBeNull();
  });

  it('answering advances to the next item, decrements the counts and keeps focus in the workbench; Esc returns to the capsule', async () => {
    const relay = vi.spyOn(superagentApi, 'relay').mockResolvedValue({ ok: true, value: { commentId: null, sessionId: 'lead', delivered: true } });
    mount();
    const card = await openEscalate();
    expect(within(card).getByText('1 of 2')).toBeTruthy();
    const dialog = screen.getByRole('dialog', { name: 'Workbench' });
    await waitFor(() => expect(dialog.contains(document.activeElement)).toBe(true));
    const box = within(card).getByLabelText('Reply to CLO-6');
    fireEvent.input(box, { target: { value: 'Pilot tenants only' } });
    fireEvent.keyDown(box, { key: 'Enter' });
    await waitFor(() => expect(relay).toHaveBeenCalledWith('pc-coffee', { projectId: 'cloud', text: 'Re CLO-6: Pilot tenants only', delivery: 'steer' }));
    // The next item is up, the counts dropped, and the reply box has focus.
    const next = await screen.findByLabelText('Reply to WEB-8');
    expect(screen.queryByLabelText('Reply to CLO-6')).toBeNull();
    expect(screen.getByRole('tab', { name: 'Escalate · 1' })).toBeTruthy();
    expect(screen.getByLabelText('Open escalations (1)')).toBeTruthy();
    expect(screen.getByRole('group', { name: /Workbench/ }).getAttribute('aria-label')).toContain('1 to escalate');
    await waitFor(() => expect(document.activeElement).toBe(next));
    // Last one: the section empties and the Escalate segment hides; Esc still closes and focus lands on the capsule.
    fireEvent.input(next, { target: { value: 'staging.example.com' } });
    fireEvent.keyDown(next, { key: 'Enter' });
    await screen.findByText('No questions waiting.');
    expect(screen.queryByLabelText(/Open escalations/)).toBeNull();
    await waitFor(() => expect(dialog.contains(document.activeElement)).toBe(true));
    fireEvent.keyDown(screen.getByRole('dialog', { name: 'Workbench' }), { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Workbench' })).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(screen.getByLabelText('Open review (1)')));
  });

  it('j / k and the arrows step the questions when no field has focus', async () => {
    mount();
    const card = await openEscalate();
    const dialog = screen.getByRole('dialog', { name: 'Workbench' });
    fireEvent.keyDown(dialog, { key: 'j' });
    await screen.findByLabelText('Reply to WEB-8');
    expect(within(card.isConnected ? card : dialog).queryByText('2 of 2')).toBeTruthy();
    fireEvent.keyDown(dialog, { key: 'ArrowLeft' });
    await screen.findByLabelText('Reply to CLO-6');
  });

  it('the right rail never carries a report body, on the desktop or the phone', async () => {
    mount();
    await screen.findByLabelText('Open escalations (2)');
    const rail = await screen.findByTestId('sa-side');
    await waitFor(() => expect(rail.textContent).toContain('Heartpick'));
    for (const body of ['Idempotency-Key', 'Migrations', 'billing_v2', 'Need from you']) expect(rail.textContent).not.toContain(body);
    cleanup(); resetSessions(); resetHubs();
    mount(true);
    fireEvent.click(await screen.findByTestId('sa-sheet-open'));
    const pane = await screen.findByLabelText('Leads and projects');
    for (const body of ['Idempotency-Key', 'Migrations', 'billing_v2', 'Need from you']) expect(pane.textContent).not.toContain(body);
  });
});
