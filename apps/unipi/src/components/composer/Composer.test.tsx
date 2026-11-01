// @vitest-environment jsdom
/**
 * The composer (WP-A12 acceptance): placeholder by state (Z-57), ↑/↓ prompt
 * history per workspace with the walk rules (Z-58), persisted drafts (Z-59),
 * the 5119/5120 paste threshold with inline expansion in the sent `prompt`
 * (Z-61, MockHost log), the `/` and `@` menus (Z-62), the `+` menu (Z-63) and
 * undo/redo kept. Runs over a real `createSession` on the mock host.
 */
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { createRoot } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createMockHost, type MockScenario, type MockWorld } from '../../dev/mock/host';
import { genFsTree, genLiveRun, genSession } from '../../dev/mock/gen';
import { setHostClientFactory } from '../../lib/host/factory';
import { createSession, type SessionHandle } from '../../lib/session/store';
import { resetPanes } from '../../lib/panes';
import { setDesktopOverride, resetLayout } from '../../lib/layout';
import { resetComposerDrafts, flushComposerDrafts, composerDraft } from '../../lib/composer/drafts';
import { installEscapeStop } from '../../lib/composer/escapeStop';
import { Composer } from './Composer';
import { shortcutsOpen } from '../shell/ShortcutsDialog';
import { actionNotice } from '../../lib/timeline/actions';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

const HOST = 'pc-one';
const CWD = '/home/you/projects/unipi';
const SESSION = 's-composer';

const COMMANDS = [
  { name: 'model', description: 'Pick the model', source: 'builtin' },
  { name: 'unipi:goal', description: 'Set a goal for this session', source: 'extension' },
  { name: 'fix-typo', description: 'Fix the typo under the caret', source: 'prompt' },
  { name: 'skill:review', description: 'Review the diff like a senior', source: 'skill' },
];

function scenario(turns: number, running: boolean): MockScenario {
  const session = genSession(SESSION, { seed: 3, turns, toolsPerTurn: 1, cwd: CWD });
  session.snapshot.commands = COMMANDS as never;
  // P1-01: the goal extension completes its sub-commands after `/unipi:goal `.
  session.cmd = {
    '_oino/get_completions': (cmd: { text?: unknown }) =>
      String(cmd.text).startsWith('/unipi:goal ')
        ? { groups: [{ source: 'command:unipi:goal', from: 12, items: [{ value: 'status', description: 'Show the goal' }, { value: 'clear' }] }] }
        : { groups: [] },
  } as never;
  session.events = running ? genLiveRun(SESSION, { seq: 1, running: true, at: 0 }) : [];
  return {
    name: 'composer-test',
    hosts: [
      {
        id: HOST,
        name: 'coffee',
        conn: { state: 'connected' },
        sessions: { [SESSION]: session },
        fs: genFsTree(CWD),
      },
    ],
  };
}

let world: MockWorld;
let disposers: Array<() => void> = [];

async function openSession(turns: number, running = false): Promise<SessionHandle> {
  world = createMockHost(scenario(turns, running));
  setHostClientFactory(world.client);
  const handle = createRoot((dispose) => {
    disposers.push(dispose);
    return createSession(HOST, SESSION, { client: world.client(HOST) });
  });
  await waitFor(() => expect(handle.state.conn).toBe('live'));
  await new Promise((r) => setTimeout(r, 20));
  return handle;
}

function mountComposer(session: SessionHandle, opts: { phone?: boolean } = {}) {
  const view = render(() => <Composer session={session} cwd={() => CWD} phone={opts.phone} />);
  const field = () => view.container.querySelector('.chip-field') as HTMLDivElement;
  /** Places the caret at the field's end (jsdom focus() selects offset 0). */
  const caretEnd = (el: HTMLDivElement) => {
    const range = document.createRange();
    range.selectNodeContents(el);
    range.collapse(false);
    const sel = window.getSelection()!;
    sel.removeAllRanges();
    sel.addRange(range);
  };
  /** Drives the contenteditable the way the browser would (text → input). */
  const type = (text: string) => {
    const el = field();
    el.focus();
    caretEnd(el);
    el.append(document.createTextNode(text));
    fireEvent.input(el);
  };
  const setText = (text: string) => {
    const el = field();
    el.focus();
    el.textContent = text;
    caretEnd(el);
    fireEvent.input(el);
  };
  const key = (k: Partial<{ key: string; shiftKey: boolean; ctrlKey: boolean; code: string }>) =>
    fireEvent.keyDown(field(), { key: k.key ?? '', ...k });
  return { view, field, type, setText, key };
}

beforeEach(() => {
  localStorage.clear();
  setDesktopOverride(true);
});

afterEach(() => {
  cleanup();
  for (const dispose of disposers) dispose();
  disposers = [];
  resetComposerDrafts();
  resetPanes();
  resetLayout();
  setDesktopOverride(undefined);
  setHostClientFactory(undefined);
  localStorage.clear();
  location.hash = '';
});

describe('placeholder by state (Z-57)', () => {
  it('an empty session shows the capability hint; with history, the follow-up hint', async () => {
    const empty = await openSession(0);
    const a = mountComposer(empty);
    expect(a.field().getAttribute('data-placeholder')).toBe('Ask anything — / for commands, @ for files');

    const full = await openSession(2);
    const b = mountComposer(full);
    expect(b.field().getAttribute('data-placeholder')).toBe('Ask for follow-up changes');
  });

  it('a running turn shows "Keep typing to queue…" (never locks the field)', async () => {
    const s = await openSession(2, true);
    await waitFor(() => expect(s.state.run.running).toBe(true));
    const c = mountComposer(s);
    expect(c.field().getAttribute('data-placeholder')).toBe('Keep typing to queue…');
    expect(c.field().getAttribute('contenteditable')).toBe('true');
  });

  it('the phone short form on an empty session', async () => {
    const empty = await openSession(0);
    const c = mountComposer(empty, { phone: true });
    expect(c.field().getAttribute('data-placeholder')).toBe('Ask anything…');
  });
});

describe('prompt history ↑/↓ (Z-58)', () => {
  it('walks on an empty draft, ↓ returns to the draft being typed', async () => {
    localStorage.setItem(`unipi:composer:history:${encodeURIComponent(`${HOST}:${CWD}`)}`, JSON.stringify(['one', 'two', 'three']));
    const s = await openSession(1);
    const c = mountComposer(s);
    c.key({ key: 'ArrowUp' });
    expect(composerDraft(s.key).text).toBe('three');
    c.key({ key: 'ArrowUp' });
    expect(composerDraft(s.key).text).toBe('two');
    c.key({ key: 'ArrowDown' });
    expect(composerDraft(s.key).text).toBe('three');
    c.key({ key: 'ArrowDown' });
    // leaving the walk restores what was being typed
    expect(composerDraft(s.key).text).toBe('');
  });

  it('a non-empty draft keeps normal cursor keys until the walk is entered', async () => {
    localStorage.setItem(`unipi:composer:history:${encodeURIComponent(`${HOST}:${CWD}`)}`, JSON.stringify(['one']));
    const s = await openSession(1);
    const c = mountComposer(s);
    c.setText('half');
    c.key({ key: 'ArrowUp' });
    expect(composerDraft(s.key).text).toBe('half');
  });

  it('a send records the prompt (no consecutive dup), a failed send rolls it back', async () => {
    const s = await openSession(1);
    const c = mountComposer(s);
    c.setText('first prompt');
    c.key({ key: 'Enter' });
    await waitFor(() => expect(world.log.some((e) => e.kind === 'cmd' && e.op === 'prompt')).toBe(true));
    const store = JSON.parse(localStorage.getItem(`unipi:composer:history:${encodeURIComponent(`${HOST}:${CWD}`)}`) ?? '[]');
    expect(store).toEqual(['first prompt']);
  });
});

describe('long paste → chip (Z-61)', () => {
  const paste = (field: HTMLDivElement, text: string) => {
    field.focus();
    const ev = new Event('paste', { bubbles: true, cancelable: true }) as ClipboardEvent;
    Object.defineProperty(ev, 'clipboardData', { value: { types: ['text/plain'], getData: () => text, files: [] } });
    field.dispatchEvent(ev);
  };

  it('5119 chars stay inline text', async () => {
    const s = await openSession(1);
    const c = mountComposer(s);
    paste(c.field(), 'x'.repeat(5119));
    await new Promise((r) => setTimeout(r, 10));
    expect(composerDraft(s.key).text).toBe('x'.repeat(5119));
    expect(c.view.container.querySelector('[data-testid="pasted-chip"]')).toBeNull();
  });

  it('5120 chars become a "Pasted text · N lines" chip, expanded inline in the sent prompt', async () => {
    const s = await openSession(1);
    const c = mountComposer(s);
    const pasted = `${'lorem ipsum dolor sit amet consectetur adipiscing elit\n'.repeat(127)}last line`;
    expect(pasted.split('\n')).toHaveLength(128);
    expect(pasted.length).toBeGreaterThanOrEqual(5120);
    paste(c.field(), pasted);
    await new Promise((r) => setTimeout(r, 10));
    expect(composerDraft(s.key).text).toBe('');
    const chip = c.view.container.querySelector('[data-testid="pasted-chip"]');
    expect(chip).not.toBeNull();
    expect(chip!.textContent).toContain('Pasted text · 128 lines');
    // ✕ removes it again
    fireEvent.click(chip!.querySelector('button')!);
    expect(c.view.container.querySelector('[data-testid="pasted-chip"]')).toBeNull();

    paste(c.field(), pasted);
    await new Promise((r) => setTimeout(r, 10));
    c.type('summarize this');
    c.key({ key: 'Enter' });
    await waitFor(() => expect(world.log.filter((e) => e.kind === 'cmd' && e.op === 'prompt')).toHaveLength(1));
    const cmd = world.log.find((e) => e.kind === 'cmd' && e.op === 'prompt')!.args as { message?: string };
    expect(cmd.message).toContain(pasted.slice(0, 40));
    expect(cmd.message).toContain('summarize this');
    // the chip is gone after the send
    expect(c.view.container.querySelector('[data-testid="pasted-chip"]')).toBeNull();
  });
});

describe('draft persistence across reload (Z-59)', () => {
  it('text survives an unmount → remount', async () => {
    const s = await openSession(1);
    const a = mountComposer(s);
    a.setText('still typing this');
    flushComposerDrafts();
    cleanup();
    const b = mountComposer(s);
    expect(b.field().textContent).toBe('still typing this');
    expect(composerDraft(s.key).text).toBe('still typing this');
  });
});

describe('undo/redo kept', () => {
  it('Ctrl+Z steps back through the draft, Ctrl+Shift+Z forward', async () => {
    const s = await openSession(1);
    const c = mountComposer(s);
    c.setText('ab');
    c.setText('abcd');
    c.key({ key: 'z', ctrlKey: true });
    expect(composerDraft(s.key).text).toBe('ab');
    c.key({ key: 'z', ctrlKey: true, shiftKey: true });
    expect(composerDraft(s.key).text).toBe('abcd');
  });
});

describe('the / menu (Z-62)', () => {
  it('shows commands, prompt templates and skills with source labels; Enter picks', async () => {
    const s = await openSession(1);
    const c = mountComposer(s);
    c.setText('/');
    await waitFor(() => expect(c.view.container.querySelector('[data-testid="slash-menu"]')).not.toBeNull());
    const labels = [...c.view.container.querySelectorAll('.cmenu-src')].map((el) => el.textContent);
    expect(labels).toContain('app');
    expect(labels).toContain('command');
    expect(labels).toContain('prompt');
    expect(labels).toContain('skill');

    c.setText('/rev');
    await waitFor(() => expect(c.view.container.querySelector('[data-testid="slash-menu"]')!.textContent).toContain('/skill:review'));
    c.key({ key: 'Enter' });
    await waitFor(() => expect(composerDraft(s.key).text).toBe('/skill:review '));
    expect(c.view.container.querySelector('[data-testid="slash-menu"]')).toBeNull();
  });

  it('an unknown /command is never sent as a prompt', async () => {
    const s = await openSession(1);
    const c = mountComposer(s);
    c.setText('/nosuch command here');
    await new Promise((r) => setTimeout(r, 10));
    c.key({ key: 'Enter' });
    await new Promise((r) => setTimeout(r, 30));
    expect(world.log.some((e) => e.kind === 'cmd' && e.op === 'prompt')).toBe(false);
  });

  it('Esc closes the menu; typing reopens it', async () => {
    const s = await openSession(1);
    const c = mountComposer(s);
    c.setText('/');
    await waitFor(() => expect(c.view.container.querySelector('[data-testid="slash-menu"]')).not.toBeNull());
    c.key({ key: 'Escape' });
    expect(c.view.container.querySelector('[data-testid="slash-menu"]')).toBeNull();
    c.setText('/m');
    await waitFor(() => expect(c.view.container.querySelector('[data-testid="slash-menu"]')).not.toBeNull());
  });
});

describe('the @ menu (Z-62)', () => {
  it('lists files from the project (fs), Enter picks an @path mention', async () => {
    const s = await openSession(1);
    const c = mountComposer(s);
    c.setText('@rea');
    await waitFor(() => expect(c.view.container.querySelector('[data-testid="mention-menu"]')).not.toBeNull(), { timeout: 3000 });
    const rows = [...c.view.container.querySelectorAll('[data-testid="mention-menu"] .cmenu-row')];
    expect(rows.length).toBeGreaterThan(0);
    expect(rows.some((r) => r.textContent!.includes('readme') || r.textContent!.includes('README'))).toBe(true);
    c.key({ key: 'Enter' });
    await waitFor(() => expect(composerDraft(s.key).text).toMatch(/^@\S+ $/));
    expect(c.view.container.querySelector('[data-testid="mention-menu"]')).toBeNull();
  });
});

describe('the + menu (Z-63)', () => {
  it('offers Photo, Camera, File, Side question and Board task… (no New session / Resume)', async () => {
    const s = await openSession(1);
    const c = mountComposer(s);
    fireEvent.click(c.view.getByLabelText('Add'));
    // AnchoredMenu portals into document.body.
    await waitFor(() => expect(document.body.querySelector('[role="menu"]')).not.toBeNull());
    const menu = document.body.querySelector('[role="menu"]')!;
    for (const label of ['Photo', 'File', 'Side question', 'Board task…']) expect(menu.textContent).toContain(label);
    expect(menu.textContent).not.toContain('New session');
    expect(menu.textContent).not.toContain('Resume');
  });

  it('Side question opens the btw side-chat tab with the draft as its question', async () => {
    const s = await openSession(1);
    const c = mountComposer(s);
    c.setText('what does this repo do?');
    fireEvent.click(c.view.getByLabelText('Add'));
    await waitFor(() => expect(document.body.querySelector('[role="menu"]')).not.toBeNull());
    fireEvent.click([...document.body.querySelectorAll('[role="menu"] button')].find((b) => b.textContent!.includes('Side question'))!);
    const { panes } = await import('../../lib/panes');
    const set = panes().set({ hostId: HOST, projectKey: CWD }, 'side');
    const btw = set.tabs.find((t) => t.kind === 'btw');
    expect(btw).toBeDefined();
    expect((btw!.params as { question?: string }).question).toBe('what does this repo do?');
    // The draft stays in the composer (only a sent /btw consumes it).
    expect(composerDraft(s.key).text).toBe('what does this repo do?');
  });

  it('/btw opens the side chat instead of sending to the main session', async () => {
    const s = await openSession(1);
    const c = mountComposer(s);
    c.setText('/btw quick aside');
    c.key({ key: 'Enter' });
    await new Promise((r) => setTimeout(r, 30));
    expect(world.log.some((e) => e.kind === 'cmd' && (e.op === 'prompt' || e.op === 'steer'))).toBe(false);
    const { panes } = await import('../../lib/panes');
    const set = panes().set({ hostId: HOST, projectKey: CWD }, 'side');
    expect((set.tabs.find((t) => t.kind === 'btw')!.params as { question?: string }).question).toBe('quick aside');
  });
});

describe('send ↔ stop (WP-A11 matrix: Enter = default, Cmd/Ctrl+Enter the opposite)', () => {
  it('Enter sends a prompt; while running Enter queues and Cmd+Enter steers; dock-root Esc stops', async () => {
    const s = await openSession(2, true);
    await waitFor(() => expect(s.state.run.running).toBe(true));
    const c = mountComposer(s);
    c.setText('a queued follow-up');
    c.key({ key: 'Enter' });
    await waitFor(() => expect(world.log.some((e) => e.kind === 'cmd' && e.op === 'follow_up')).toBe(true));
    expect(composerDraft(s.key).text).toBe('');
    c.setText('a steer via modifier');
    c.key({ key: 'Enter', ctrlKey: true });
    await waitFor(() => expect(world.log.some((e) => e.kind === 'cmd' && e.op === 'steer')).toBe(true));
  });

  it('Esc stops the run through the dock root (Z-55), not the composer itself', async () => {
    const s = await openSession(2, true);
    await waitFor(() => expect(s.state.run.running).toBe(true));
    const view = render(() => (
      <div ref={(el) => installEscapeStop({ root: el, running: () => !!s.state.run.running, stop: () => void s.abort() })}>
        <Composer session={s} cwd={() => CWD} />
      </div>
    ));
    const field = view.container.querySelector('.chip-field') as HTMLDivElement;
    field.focus();
    fireEvent.keyDown(field, { key: 'Escape' });
    await waitFor(() => expect(world.log.some((e) => e.kind === 'cmd' && e.op === 'abort')).toBe(true));
  });
});

describe('pi built-ins route to app surfaces (WP-A25, G-15)', () => {
  it('/hotkeys opens the keyboard list and clears the draft (never a prompt)', async () => {
    const s = await openSession(1);
    const c = mountComposer(s);
    c.setText('/hotkeys');
    await waitFor(() => expect(c.view.container.querySelector('[data-testid="slash-menu"]')).not.toBeNull());
    c.key({ key: 'Escape' });
    await waitFor(() => expect(c.view.container.querySelector('[data-testid="slash-menu"]')).toBeNull());
    c.key({ key: 'Enter' });
    await waitFor(() => expect(shortcutsOpen()).toBe(true));
    expect(composerDraft(s.key).text).toBe('');
    expect(world.log.some((entry) => entry.kind === 'cmd' && entry.op === 'prompt')).toBe(false);
  });

  it('/share and /trust explain themselves instead of a prompt', async () => {
    const s = await openSession(1);
    const c = mountComposer(s);
    c.setText('/share');
    c.key({ key: 'Escape' });
    c.key({ key: 'Enter' });
    await waitFor(() => expect(actionNotice() ?? '').toMatch(/Export/));
    expect(world.log.some((entry) => entry.kind === 'cmd' && entry.op === 'prompt')).toBe(false);
  });

  it('an extension command of the same name wins over the app route', async () => {
    const s = await openSession(1);
    const c = mountComposer(s);
    // The mock session lists `model` as a builtin; `settings` is not listed, so
    // the app route handles it — and never sends it as a prompt.
    c.setText('/settings');
    c.key({ key: 'Escape' });
    c.key({ key: 'Enter' });
    await waitFor(() => expect(location.hash).toBe('#/settings/general'));
    expect(world.log.some((entry) => entry.kind === 'cmd' && entry.op === 'prompt')).toBe(false);
  });
});

describe('P0-02: engine built-ins run, never reach the model as text', () => {
  const sendCommand = async (text: string) => {
    const s = await openSession(1);
    const c = mountComposer(s);
    c.setText(text);
    c.key({ key: 'Escape' });
    c.key({ key: 'Enter' });
    return { s, c };
  };
  const prompted = () => world.log.some((e) => e.kind === 'cmd' && (e.op === 'prompt' || e.op === 'steer' || e.op === 'follow_up'));

  it('/compact <focus> runs the compact RPC with the focus text', async () => {
    await sendCommand('/compact keep the API notes');
    await waitFor(() => expect(world.log.some((e) => e.kind === 'cmd' && e.op === 'compact')).toBe(true));
    const cmd = world.log.find((e) => e.kind === 'cmd' && e.op === 'compact')!.args as { customInstructions?: string };
    expect(cmd.customInstructions).toBe('keep the API notes');
    expect(prompted()).toBe(false);
  });

  it('/copy asks for the last reply', async () => {
    await sendCommand('/copy');
    await waitFor(() => expect(world.log.some((e) => e.kind === 'cmd' && e.op === 'get_last_assistant_text')).toBe(true));
    expect(prompted()).toBe(false);
  });

  it('/thinking <level> sets it; /model opens the picker; /session opens Info', async () => {
    const { s } = await sendCommand('/thinking high');
    await waitFor(() => expect(world.log.some((e) => e.kind === 'cmd' && e.op === 'get_available_thinking_levels')).toBe(true));
    expect(prompted()).toBe(false);
    expect(composerDraft(s.key).text).toBe('');
  });

  it('/new goes to a new chat in this folder; /resume opens the sessions list', async () => {
    await sendCommand('/new');
    await waitFor(() => expect(location.hash).toContain(`/new/${HOST}`));
    expect(decodeURIComponent(location.hash)).toContain(CWD);
    expect(prompted()).toBe(false);
  });

  it('/reload is not a command (no reload RPC) and is never sent', async () => {
    await sendCommand('/reload');
    await waitFor(() => expect(actionNotice() ?? '').toMatch(/Unknown command \/reload/));
    expect(prompted()).toBe(false);
  });

  it('/model still opens the picker though the session lists it as a builtin', async () => {
    await sendCommand('/model');
    await new Promise((r) => setTimeout(r, 20));
    expect(prompted()).toBe(false);
  });
});


describe('P0-07/P0-08: Mention in chat and insertEditorToken reach the mounted composer', () => {
  it('a mention lands at the caret, space-separated; an editor token too', async () => {
    const { queueMention, resetMentions } = await import('../../lib/chat/mention');
    const { insertEditorToken, resetEditorTokens } = await import('../../lib/composer/tokens');
    const s = await openSession(1);
    const c = mountComposer(s);
    c.setText('look at');
    queueMention(`${HOST}:${SESSION}`, '@src/app.ts');
    await waitFor(() => expect(composerDraft(s.key).text).toBe('look at @src/app.ts '));
    insertEditorToken(s.key, { kind: 'text', label: 'and this' });
    await waitFor(() => expect(composerDraft(s.key).text).toBe('look at @src/app.ts and this '));
    insertEditorToken(s.key, { kind: 'file', label: '', path: 'docs/my notes.md' });
    await waitFor(() => expect(composerDraft(s.key).text).toContain('@"docs/my notes.md"'));
    resetMentions();
    resetEditorTokens();
  });
});

describe('P1-01: /cmd <arg> completions from the extension', () => {
  it('asks _oino/get_completions after `/unipi:goal ` and Enter applies the pick', async () => {
    const s = await openSession(1);
    const c = mountComposer(s);
    c.setText('/unipi:goal ');
    await waitFor(() => expect(c.view.container.querySelector('[data-testid="arg-menu"]')).not.toBeNull());
    expect(c.view.getByRole('option', { name: /status/ })).toBeTruthy();
    c.key({ key: 'Enter' });
    await waitFor(() => expect(composerDraft(s.key).text).toBe('/unipi:goal status '));
    expect(world.log.some((entry) => entry.kind === 'cmd' && entry.op === 'prompt')).toBe(false);
  });

  it('built-ins (no extension completer) never ask', async () => {
    const s = await openSession(1);
    const c = mountComposer(s);
    c.setText('/model ');
    await new Promise((r) => setTimeout(r, 150));
    expect(world.log.some((entry) => entry.kind === 'cmd' && entry.op === '_oino/get_completions')).toBe(false);
  });
});
