// @vitest-environment jsdom
/**
 * The project-trust banner (OIN-101 WI-3, SPEC §1 / §3 check 7): shown for an undecided folder with project settings,
 * never blocking; Trust folder / Trust parent / Don't trust save one decision through `_oino/trust_set` and hide it.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { createStore } from 'solid-js/store';
import { TrustBanner } from './TrustBanner';
import { resetTrustDecisions } from '../../lib/session/trust';
import { toasts } from '../../lib/toasts';
import type { ProjectTrust } from '../../lib/session/trust';
import type { SessionHandle } from '../../lib/session/store';

const CWD = '/home/you/work/clients/zm';

function mount(trust: ProjectTrust | undefined, cmd: (c: Record<string, unknown>) => Promise<unknown> = async () => ({ ok: true })) {
  const [state] = createStore({ projectTrust: trust });
  const send = vi.fn(cmd);
  const session = { state, cmd: send } as unknown as SessionHandle;
  render(() => <TrustBanner session={session} cwd={CWD} />);
  return send;
}
const undecided: ProjectTrust = { state: 'undecided', path: CWD, protected: ['.oino/mcp.json', '.mcp.json', '.oino/skills'] };

afterEach(() => {
  cleanup();
  resetTrustDecisions();
  toasts().clear();
});

describe('TrustBanner', () => {
  it('says what the folder has and offers the three choices — without blocking anything', () => {
    mount(undecided);
    const banner = screen.getByTestId('trust-banner');
    expect(banner.textContent).toContain('This folder has project settings');
    expect(banner.textContent).toContain('MCP servers (.oino/mcp.json, .mcp.json), skills');
    expect(screen.getByTestId('trust-folder').textContent).toContain('Trust folder');
    expect(screen.getByTestId('trust-parent').textContent).toContain('Trust parent');
    expect(screen.getByTestId('trust-never').textContent).toContain('Don’t trust');
    expect(banner.getAttribute('role')).toBe('region');
    expect(banner.getAttribute('aria-modal')).toBeNull();
  });

  it.each([
    ['trusted', { state: 'trusted', path: CWD, protected: ['.mcp.json'] }],
    ['saved untrusted ("Don’t trust" hides it for good)', { state: 'untrusted', path: CWD, protected: ['.mcp.json'] }],
    ['nothing protected in the folder', { state: 'undecided', path: CWD, protected: [] }],
    ['an older host (no project_trust)', undefined],
  ])('is absent for %s', (_name, trust) => {
    mount(trust as ProjectTrust | undefined);
    expect(screen.queryByTestId('trust-banner')).toBeNull();
  });

  it('Trust folder saves true on the folder and hides the banner', async () => {
    const send = mount(undecided);
    fireEvent.click(screen.getByTestId('trust-folder'));
    await waitFor(() => expect(send).toHaveBeenCalledWith({ type: '_oino/trust_set', path: CWD, trusted: true }));
    await waitFor(() => expect(screen.queryByTestId('trust-banner')).toBeNull());
    expect(toasts().list().some((t) => t.message.includes('Trusted'))).toBe(true);
  });

  it('Don’t trust saves false and hides the banner', async () => {
    const send = mount(undecided);
    fireEvent.click(screen.getByTestId('trust-never'));
    await waitFor(() => expect(send).toHaveBeenCalledWith({ type: '_oino/trust_set', path: CWD, trusted: false }));
    await waitFor(() => expect(screen.queryByTestId('trust-banner')).toBeNull());
  });

  it('Trust parent ▾ lists the parent folders and saves the chosen one', async () => {
    const send = mount(undecided);
    fireEvent.click(screen.getByTestId('trust-parent'));
    const parent = await screen.findByTestId('trust-parent-/home/you/work/clients');
    expect(screen.getByTestId('trust-parent-/home/you/work')).toBeTruthy();
    expect(screen.queryByTestId('trust-parent-/home/you')).toBeNull();
    fireEvent.click(parent);
    await waitFor(() => expect(send).toHaveBeenCalledWith({ type: '_oino/trust_set', path: '/home/you/work/clients', trusted: true }));
    await waitFor(() => expect(screen.queryByTestId('trust-banner')).toBeNull());
  });

  it('a failed save keeps the banner and says why', async () => {
    mount(undecided, async () => {
      throw new Error('E_UNSUPPORTED unknown command _oino/trust_set');
    });
    fireEvent.click(screen.getByTestId('trust-folder'));
    await waitFor(() => expect(toasts().list().some((t) => t.message.includes('Couldn’t save the trust decision'))).toBe(true));
    expect(screen.getByTestId('trust-banner')).toBeTruthy();
  });

  it('uses the snapshot’s path (a decision may apply above the chat’s folder)', async () => {
    const send = mount({ ...undecided, path: '/home/you/work/clients' });
    fireEvent.click(screen.getByTestId('trust-folder'));
    await waitFor(() => expect(send).toHaveBeenCalledWith({ type: '_oino/trust_set', path: '/home/you/work/clients', trusted: true }));
  });

  it('phone: the buttons are 44 px targets (CSS rule)', async () => {
    const { readFileSync } = await import('node:fs');
    const css = readFileSync(`${process.cwd()}/src/components/conversation/trust-banner.css`, 'utf8');
    expect(css).toMatch(/html:not\(\[data-shell=desktop\]\) \.trust-banner-btn\s*{[^}]*min-height:\s*44px/);
  });
});
