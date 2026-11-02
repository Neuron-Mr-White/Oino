/**
 * Phone spec §3.4 (PH-18): the chat ⋯ action sheet's Open section launches the pane surfaces
 * (Files layer, status-pane layer, session-tab sheets) — the side pane /
 * bottom panel don't exist on the phone (§3.9). Also covers the tab bar and
 * the draft's reduced group (no session-scoped kinds).
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Root } from '../../main';
import { createMockHost } from '../../dev/mock/host';
import { streamBasic } from '../../dev/mock/scenarios/stream-basic';
import { setHostClientFactory } from '../../lib/host/factory';
import { resetLayout, setDesktopOverride } from '../../lib/layout';
import { setPlatformOverride } from '../../lib/platform';
import { resetPanes } from '../../lib/panes';
import { resetHubs } from '../../lib/session/hub';
import { backDepth } from '../../lib/back';
import { closeGit } from '../../lib/git';
import { closePhoneSessionTab } from '../panes/tabs/sessionTabs';
import { resetShortcuts } from '../../lib/shortcuts/registry';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));
vi.mock('@tauri-apps/api/app', () => ({ onBackButtonPress: async () => ({ unregister: async () => {} }) }));
vi.mock('@tauri-apps/plugin-deep-link', () => ({ getCurrent: async () => [], onOpenUrl: async () => () => {} }));

const SESSION = streamBasic.hosts[0]!.cards![0]!.session_id;

function boot(hash: string) {
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: 412 });
  setPlatformOverride('tauri-mobile');
  setDesktopOverride(false);
  setHostClientFactory(createMockHost(streamBasic).client);
  location.hash = hash;
  return render(() => <Root />);
}

async function openMenu() {
  await waitFor(() => expect(screen.getByTestId('conv-title').textContent).not.toBe('Session'));
  fireEvent.click(screen.getByTestId('conv-more'));
  await waitFor(() => expect(screen.getByTestId('chat-actions-sheet')).toBeTruthy());
}

const rowLabels = () => [...document.querySelectorAll('.ph-action-row .ph-action-label')].map((el) => el.textContent?.trim());
const menuLabels = rowLabels;
const clickRow = async (label: string) => {
  const row = [...document.querySelectorAll('.ph-action-row')].find((el) => el.querySelector('.ph-action-label')?.textContent?.trim() === label);
  expect(row, `menu row "${label}"`).toBeTruthy();
  fireEvent.click(row!);
};

beforeEach(() => {
  localStorage.clear();
  (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = { transformCallback: () => 0, invoke: () => Promise.resolve(0) };
});
afterEach(() => {
  cleanup();
  setDesktopOverride(undefined);
  setPlatformOverride(undefined);
  setHostClientFactory(undefined);
  resetHubs();
  resetPanes();
  resetLayout();
  resetShortcuts();
  closeGit();
  closePhoneSessionTab();
  location.hash = '';
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: 1024 });
  delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
});

describe('phone tab bar (§3.9)', () => {
  it('shows Chat · Board · Settings', async () => {
    boot(`#/s/pc-coffee/${SESSION}`);
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy());
    for (const tab of ['tab-chat', 'tab-board', 'tab-settings']) expect(screen.getByTestId(tab)).toBeTruthy();
  });
});

describe('chat ⋯ sheet (phone spec §3.4)', () => {
  it('lists the launchable pane kinds in a session', async () => {
    boot(`#/s/pc-coffee/${SESSION}`);
    await openMenu();
    const labels = menuLabels();
    for (const label of ['Terminal', 'Files', 'Git changes', 'Conversation tree', 'Side chat', 'Context & info', 'Search in chat', 'Rename', 'Fork', 'Share / export', 'Archive']) expect(labels).toContain(label);
    // Desktop-only rows are gone from the phone (PH-18).
    for (const label of ['Duplicate', 'Copy session id', 'Copy path', 'Copy log path', 'Reveal folder', 'Panes']) expect(labels).not.toContain(label);
  });

  it('Files opens the /files layer at the chat host', async () => {
    boot('#/new/pc-coffee');
    await waitFor(() => expect(screen.getByTestId('conv-more')).toBeTruthy());
    fireEvent.click(screen.getByTestId('conv-more'));
    await waitFor(() => expect(screen.getByTestId('chat-actions-sheet')).toBeTruthy());
    await clickRow('Files');
    await waitFor(() => expect(screen.getByTestId('files-layer')).toBeTruthy());
    expect(location.hash.startsWith('#/files')).toBe(true);
  });

  it('Files from a session routes to /files (the unmount itself has a known upstream crash, tracked for A6/A9/A12)', async () => {
    boot(`#/s/pc-coffee/${SESSION}`);
    await openMenu();
    await clickRow('Files');
    await waitFor(() => expect(location.hash.startsWith('#/files')).toBe(true));
  });

  it('Git opens the status-pane layer', async () => {
    boot(`#/s/pc-coffee/${SESSION}`);
    await openMenu();
    await clickRow('Git changes');
    await waitFor(() => expect(screen.getByTestId('git-view')).toBeTruthy());
  });

  it('Tree opens the session-tab sheet', async () => {
    boot(`#/s/pc-coffee/${SESSION}`);
    await openMenu();
    await clickRow('Conversation tree');
    await waitFor(() => expect(screen.getByTestId('session-tab-sheet')).toBeTruthy());
  });

  it('Side chat opens its sheet', async () => {
    boot(`#/s/pc-coffee/${SESSION}`);
    await openMenu();
    await clickRow('Side chat');
    await waitFor(() => expect(screen.getByTestId('session-tab-sheet')).toBeTruthy());
  });

  it('a draft gets the group too, without session kinds', async () => {
    boot('#/new/pc-coffee');
    await waitFor(() => expect(screen.getByTestId('conv-more')).toBeTruthy());
    fireEvent.click(screen.getByTestId('conv-more'));
    await waitFor(() => expect(screen.getByTestId('chat-actions-sheet')).toBeTruthy());
    const labels = menuLabels();
    expect(labels).toContain('Files');
    expect(labels).toContain('Git changes');
    expect(labels).not.toContain('Conversation tree');
    expect(labels).not.toContain('Side chat');
  });
});

describe('Android Back stack (§3.9)', () => {
  it('Escape (= Back) pops the status layer and leaves nothing on the stack', async () => {
    boot(`#/s/pc-coffee/${SESSION}`);
    await openMenu();
    await clickRow('Git changes');
    await waitFor(() => expect(screen.getByTestId('git-view')).toBeTruthy());
    expect(backDepth()).toBe(1);
    fireEvent.keyDown(window, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByTestId('git-view')).toBeNull());
    expect(backDepth()).toBe(0);
  });

  it('Escape pops a session-tab sheet and leaves nothing on the stack', async () => {
    boot(`#/s/pc-coffee/${SESSION}`);
    await openMenu();
    await clickRow('Conversation tree');
    await waitFor(() => expect(screen.getByTestId('session-tab-sheet')).toBeTruthy());
    await waitFor(() => expect(backDepth()).toBeGreaterThan(0));
    fireEvent.keyDown(window, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByTestId('session-tab-sheet')).toBeNull());
    expect(backDepth()).toBe(0);
  });
});
